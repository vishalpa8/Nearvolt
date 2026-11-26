//! Sending side: holds a connection to one receiver and sends batches of files
//! over it, one after another, until the app disconnects. Each file travels
//! whole over one of several parallel streams; when the link drops, the sender
//! reconnects the same way and resumes from the last byte the receiver has.

use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::disk;
use super::manifest::{FileSet, Manifest};
use super::state::hash_range;
use super::{emit, is_fatal, Event, EventSender, Fatal, Stats, Summary};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{self, Secret, Ticket};
use crate::protocol::{
    read_msg, write_msg, Control, FileHave, FileHeader, SessionId, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT,
    PROTOCOL_VERSION,
};
use crate::transport::{peer_fingerprint, Connector, Link, TcpLink, TlsStream};
use crate::util::AbortOnDrop;

/// With nothing left to send and no progress for this long, the session is
/// restarted so the receiver can report exactly what it is still missing.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const BLOCK: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct SendOptions {
    pub device_name: String,
    /// Parallel data streams; each carries one whole file at a time.
    pub connections: usize,
    /// How long to keep trying to reconnect after the link drops.
    pub reconnect_for: Duration,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self { device_name: "Nearvolt device".into(), connections: 8, reconnect_for: Duration::from_secs(60) }
    }
}

/// Who to send to and how to reach them.
pub struct Peer<L> {
    pub link: L,
    pub name: String,
    /// Pins the receiver's certificate. Every ticket carries it; USB has none.
    pub fingerprint: Option<Fingerprint>,
    /// From a scanned or tapped ticket. Without it, both devices confirm a code.
    pub secret: Option<Secret>,
}

impl Peer<TcpLink> {
    pub fn from_ticket(ticket: &Ticket) -> Self {
        Self {
            link: TcpLink::new(ticket.addrs.clone()),
            name: ticket.name.clone(),
            fingerprint: Some(ticket.fingerprint),
            secret: ticket.secret,
        }
    }
}

/// What the app asks of a connection.
pub enum Command {
    /// Queues a batch of files; batches are sent one after another.
    Send(FileSet),
    /// Ends the connection now, even in the middle of a batch.
    Disconnect,
}

/// Sends one batch, then ends the connection.
pub async fn send<L: Link>(
    identity: &Identity,
    peer: &Peer<L>,
    files: FileSet,
    options: &SendOptions,
    stats: Arc<Stats>,
    events: Option<EventSender>,
) -> Result<Summary> {
    let (commands, queue) = mpsc::unbounded_channel();
    let _ = commands.send(Command::Send(files));
    drop(commands);
    connect(identity, peer, options, queue, stats, events).await
}

/// Connects to `peer` and keeps the connection, reconnecting whenever the link
/// drops, until a `Disconnect` command, until `commands` closes and every
/// queued batch is sent, or until the receiver ends it. Progress in `stats` is
/// for the batch being sent. Returns the totals of everything sent.
pub async fn connect<L: Link>(
    identity: &Identity,
    peer: &Peer<L>,
    options: &SendOptions,
    mut commands: mpsc::UnboundedReceiver<Command>,
    stats: Arc<Stats>,
    events: Option<EventSender>,
) -> Result<Summary> {
    let started = Instant::now();
    let connector = Connector::new(identity, peer.fingerprint)?;
    let mut batches = Batches::default();
    let mut failing_since: Option<Instant> = None;
    let mut attempt = 0;
    loop {
        match open(&peer.link, &connector).await {
            Ok(control) => {
                attempt = 0;
                let session =
                    session(identity, &connector, peer, control, options, &mut batches, &mut commands, &stats, &events);
                match session.await {
                    Ok(()) => return Ok(batches.summary(started)),
                    Err(e) if is_fatal(&e) => return Err(e),
                    Err(e) => {
                        emit(&events, Event::ConnectionLost { reason: format!("{e:#}") });
                        failing_since = Some(Instant::now());
                    }
                }
            }
            Err(e) => {
                if batches.finished() {
                    return Ok(batches.summary(started));
                }
                let since = *failing_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= options.reconnect_for {
                    return Err(e.context("could not reach the receiver"));
                }
                attempt += 1;
                // Batches still queue while offline, and Disconnect ends at once.
                let wait = tokio::time::sleep(backoff(attempt));
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        _ = &mut wait => break,
                        command = commands.recv(), if !batches.closed => {
                            if batches.handle(command, &events).await.is_break() {
                                return Ok(batches.summary(started));
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn open<L: Link>(link: &L, connector: &Connector) -> Result<TlsStream> {
    connector.connect(link.open_stream().await?).await
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(match attempt {
        1 => 100,
        2 => 250,
        3 => 500,
        4 => 1000,
        _ => 2000,
    })
}

/// Batches waiting their turn, and the one being sent. Kept across reconnects.
#[derive(Default)]
struct Batches {
    next: u32,
    pending: VecDeque<(u32, Arc<Manifest>)>,
    current: Option<(u32, Arc<Manifest>)>,
    /// The app dropped its command channel: no more batches will come.
    closed: bool,
    files: u64,
    bytes: u64,
}

impl Batches {
    /// Breaks on `Disconnect`.
    async fn handle(&mut self, command: Option<Command>, events: &Option<EventSender>) -> ControlFlow<()> {
        match command {
            Some(Command::Send(set)) => {
                let batch = self.next;
                self.next += 1;
                match tokio::task::spawn_blocking(move || Manifest::build(set)).await {
                    Ok(Ok(manifest)) => self.pending.push_back((batch, Arc::new(manifest))),
                    Ok(Err(e)) => emit(events, Event::BatchFailed { batch, reason: format!("{e:#}") }),
                    Err(e) => emit(events, Event::BatchFailed { batch, reason: e.to_string() }),
                }
                ControlFlow::Continue(())
            }
            Some(Command::Disconnect) => ControlFlow::Break(()),
            None => {
                self.closed = true;
                ControlFlow::Continue(())
            }
        }
    }

    /// Nothing left to send and nothing more coming.
    fn finished(&self) -> bool {
        self.closed && self.current.is_none() && self.pending.is_empty()
    }

    fn summary(&self, started: Instant) -> Summary {
        Summary { files: self.files, bytes: self.bytes, elapsed: started.elapsed() }
    }
}

#[derive(Clone)]
struct Job {
    batch: u32,
    file: u32,
    offset: u64,
    manifest: Arc<Manifest>,
}

#[allow(clippy::too_many_arguments)]
async fn session<L: Link>(
    identity: &Identity,
    connector: &Connector,
    peer: &Peer<L>,
    control: TlsStream,
    options: &SendOptions,
    batches: &mut Batches,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    stats: &Arc<Stats>,
    events: &Option<EventSender>,
) -> Result<()> {
    let receiver = peer_fingerprint(&control).context("the receiver sent no certificate")?;
    let (mut rd, mut wr) = tokio::io::split(control);
    let nonce = peer.secret.is_none().then(pairing::new_secret);
    let hello = Control::Hello {
        version: PROTOCOL_VERSION,
        secret: peer.secret,
        device_name: options.device_name.clone(),
        session: None,
        commitment: nonce.as_ref().map(pairing::commitment),
    };
    write_msg(&mut wr, &hello).await?;
    let (session_id, receiver_nonce) = match read_msg(&mut rd).await? {
        Control::Welcome { session, nonce, .. } => (session, nonce),
        Control::Reject { reason } => return Err(Fatal(reason).into()),
        other => bail!("unexpected reply: {other:?}"),
    };
    if let Some(ours) = nonce {
        let theirs = receiver_nonce.context("the receiver did not send its pairing nonce")?;
        write_msg(&mut wr, &Control::Reveal { nonce: ours }).await?;
        let code = pairing::pairing_code(&receiver, identity.fingerprint(), &ours, &theirs);
        emit(events, Event::Code { code });
    }

    let (out, out_rx) = mpsc::unbounded_channel();
    let writer = AbortOnDrop(tokio::spawn(write_loop(wr, out_rx)));
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let _reader = AbortOnDrop(tokio::spawn(read_loop(rd, control_tx)));

    // The receiver's user may take a while to accept; commands still work meanwhile.
    loop {
        tokio::select! {
            msg = control_rx.recv() => match msg {
                Some(Ok(Control::Ready)) => break,
                Some(Ok(Control::Reject { reason })) => return Err(Fatal(reason).into()),
                Some(Ok(Control::Bye)) => {
                    emit(events, Event::PeerLeft);
                    return Ok(());
                }
                Some(Ok(other)) => bail!("unexpected reply: {other:?}"),
                Some(Err(e)) => return Err(e),
                None => bail!("the receiver closed the connection"),
            },
            command = commands.recv(), if !batches.closed => {
                if batches.handle(command, events).await.is_break() {
                    return say_bye(out, writer).await;
                }
            }
        }
    }
    emit(events, Event::Connected { peer: peer.name.clone(), route: peer.link.route() });

    // Data streams open alongside everything else, so the receiver's messages,
    // including a goodbye, are handled from the first moment.
    let mut opening: FuturesUnordered<_> =
        (0..options.connections.max(1)).map(|_| open(&peer.link, connector)).collect();
    let (jobs, jobs_rx) = async_channel::unbounded();
    let mut workers = JoinSet::new();

    let mut offered = false;
    let mut accepted = false;
    let mut batch_started = Instant::now();
    // Anything moving counts as progress: bytes arriving, the receiver checking
    // finished files, or this side hashing what the receiver already has.
    let mut remote = 0u64;
    let mut last_progress = (0u64, Instant::now());
    let mut stall_check = tokio::time::interval(Duration::from_secs(1));
    loop {
        if batches.current.is_none() {
            batches.current = batches.pending.pop_front();
            (offered, accepted) = (false, false);
        }
        match &batches.current {
            Some((batch, manifest)) if !offered => {
                let _ = out.send(Control::Offer { batch: *batch, files: manifest.offer.files.clone() });
                offered = true;
            }
            None if batches.closed => return say_bye(out, writer).await,
            _ => {}
        }
        tokio::select! {
            msg = control_rx.recv() => match msg {
                Some(Ok(Control::Accept { batch, have })) => {
                    let Some((_, manifest)) = batches.current.clone().filter(|(b, _)| *b == batch) else { continue };
                    let offer = &manifest.offer;
                    ensure!(have.len() == offer.files.len(), "the receiver answered for a different file list");
                    let (mut done_files, mut done_bytes) = (0u64, 0u64);
                    for (i, state) in have.iter().enumerate() {
                        let offset = match state {
                            FileHave::Complete => {
                                done_files += 1;
                                done_bytes += offer.files[i].size;
                                continue;
                            }
                            FileHave::None => 0,
                            FileHave::Partial { offset } => *offset,
                        };
                        done_bytes += offset;
                        let _ = jobs.try_send(Job { batch, file: i as u32, offset, manifest: manifest.clone() });
                    }
                    let (files, bytes) = (offer.files.len() as u64, offer.total_bytes());
                    stats.set_totals(files, bytes);
                    stats.set_done(done_files, done_bytes);
                    (accepted, batch_started, last_progress.1) = (true, Instant::now(), Instant::now());
                    emit(events, Event::Started { batch, files, bytes });
                }
                Some(Ok(Control::Progress { files, bytes, checked })) => {
                    stats.set_done(files, bytes);
                    remote = bytes.wrapping_add(checked);
                }
                Some(Ok(Control::Resend { files })) => {
                    if let Some((batch, manifest)) = &batches.current {
                        for file in files.into_iter().filter(|&f| (f as usize) < manifest.offer.files.len()) {
                            let _ = jobs.try_send(Job { batch: *batch, file, offset: 0, manifest: manifest.clone() });
                        }
                    }
                }
                Some(Ok(Control::Complete { batch, files, bytes })) => {
                    if batches.current.as_ref().is_some_and(|(b, _)| *b == batch) {
                        stats.set_done(files, bytes);
                        (batches.files, batches.bytes) = (batches.files + files, batches.bytes + bytes);
                        let summary = Summary { files, bytes, elapsed: batch_started.elapsed() };
                        emit(events, Event::Completed { batch, summary });
                        batches.current = None;
                    }
                }
                // The receiver refused this batch, for example for lack of space;
                // the connection stays for the next one.
                Some(Ok(Control::Reject { reason })) => {
                    if let Some((batch, _)) = batches.current.take() {
                        emit(events, Event::BatchFailed { batch, reason });
                    }
                }
                Some(Ok(Control::Ping)) => {}
                Some(Ok(Control::Bye)) => {
                    emit(events, Event::PeerLeft);
                    return Ok(());
                }
                Some(Ok(other)) => bail!("unexpected message: {other:?}"),
                Some(Err(e)) => return Err(e),
                None => bail!("the control connection closed"),
            },
            command = commands.recv(), if !batches.closed => {
                if batches.handle(command, events).await.is_break() {
                    return say_bye(out, writer).await;
                }
            }
            Some(stream) = opening.next() => {
                workers.spawn(worker(stream?, session_id, peer.secret, jobs_rx.clone(), stats.clone()));
            }
            Some(joined) = workers.join_next() => {
                let error = match joined {
                    Ok(Ok(())) => continue,
                    Ok(Err(e)) => e,
                    Err(e) => anyhow!("a send worker failed: {e}"),
                };
                // A receiver that ends the connection closes its data streams right
                // after saying goodbye, and the goodbye can arrive a moment later.
                if said_bye(&mut control_rx).await {
                    emit(events, Event::PeerLeft);
                    return Ok(());
                }
                return Err(error);
            }
            _ = stall_check.tick() => {
                let pulse = remote.wrapping_add(stats.checked());
                if pulse != last_progress.0 {
                    last_progress = (pulse, Instant::now());
                } else if accepted && jobs.is_empty() && last_progress.1.elapsed() >= STALL_TIMEOUT {
                    bail!("the receiver stopped making progress; resynchronizing");
                }
            }
        }
    }
}

/// Whether the receiver says goodbye within a second.
async fn said_bye(control: &mut mpsc::UnboundedReceiver<Result<Control>>) -> bool {
    let bye = async {
        while let Some(Ok(msg)) = control.recv().await {
            if matches!(msg, Control::Bye) {
                return true;
            }
        }
        false
    };
    tokio::time::timeout(Duration::from_secs(1), bye).await.unwrap_or(false)
}

/// Tells the receiver the connection is over, so it stops waiting for a reconnect.
async fn say_bye(out: mpsc::UnboundedSender<Control>, mut writer: AbortOnDrop<Result<()>>) -> Result<()> {
    let _ = out.send(Control::Bye);
    drop(out);
    let _ = tokio::time::timeout(Duration::from_secs(2), &mut writer.0).await;
    Ok(())
}

/// Sends queued control messages, plus a ping every half second so the receiver
/// knows the link is alive.
async fn write_loop(mut wr: WriteHalf<TlsStream>, mut out: mpsc::UnboundedReceiver<Control>) -> Result<()> {
    let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        let msg = tokio::select! {
            msg = out.recv() => match msg {
                Some(msg) => msg,
                None => return Ok(()),
            },
            _ = tick.tick() => Control::Ping,
        };
        write_msg(&mut wr, &msg).await?;
    }
}

/// Forwards the receiver's messages. Before `Ready` it waits as long as the user
/// takes to accept; after that, silence means the link is gone, even if TCP
/// hasn't noticed yet.
async fn read_loop(mut rd: ReadHalf<TlsStream>, tx: mpsc::UnboundedSender<Result<Control>>) {
    let mut ready = false;
    loop {
        let msg = if ready {
            tokio::time::timeout(HEARTBEAT_TIMEOUT, read_msg(&mut rd)).await.unwrap_or_else(|_| {
                Err(anyhow!("no word from the receiver for {} seconds", HEARTBEAT_TIMEOUT.as_secs()))
            })
        } else {
            read_msg(&mut rd).await
        };
        ready |= matches!(msg, Ok(Control::Ready));
        let failed = msg.is_err();
        if tx.send(msg).is_err() || failed {
            break;
        }
    }
}

/// Sends whole files from the shared queue over one data stream.
async fn worker(
    mut stream: TlsStream,
    session: SessionId,
    secret: Option<Secret>,
    jobs: async_channel::Receiver<Job>,
    stats: Arc<Stats>,
) -> Result<()> {
    let hello = Control::Hello {
        version: PROTOCOL_VERSION,
        secret,
        device_name: String::new(),
        session: Some(session),
        commitment: None,
    };
    write_msg(&mut stream, &hello).await?;
    while let Ok(job) = jobs.recv().await {
        send_file(&mut stream, job, &stats).await?;
    }
    Ok(())
}

/// One file: its header, the bytes from `offset` to the end, then the hash of the
/// whole file. The part the receiver already has is read only to hash it.
async fn send_file(stream: &mut TlsStream, job: Job, stats: &Arc<Stats>) -> Result<()> {
    let Job { batch, file, offset, manifest } = job;
    let entry = &manifest.offer.files[file as usize];
    let size = entry.size;
    let unreadable = |e: std::io::Error| Fatal(format!("can't read {}: {e}", entry.path));
    let (m, path, stats) = (manifest.clone(), entry.path.clone(), stats.clone());
    let (mut source, mut hasher) = tokio::task::spawn_blocking(move || -> Result<_> {
        let source = m.open(file)?;
        let mut hasher = blake3::Hasher::new();
        hash_range(&source, 0, offset, &mut hasher, &stats).map_err(|e| Fatal(format!("can't read {path}: {e}")))?;
        Ok((source, hasher))
    })
    .await??;

    stream.write_all(&FileHeader { batch, file, offset, len: size - offset }.encode()).await?;
    let mut buf = vec![0u8; BLOCK.min((size - offset) as usize)];
    let mut pos = offset;
    while pos < size {
        let n = (size - pos).min(BLOCK as u64) as usize;
        (source, hasher, buf) = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
            disk::read_exact_at(&source, &mut buf[..n], pos)?;
            hasher.update(&buf[..n]);
            Ok((source, hasher, buf))
        })
        .await?
        .map_err(unreadable)?;
        stream.write_all(&buf[..n]).await?;
        pos += n as u64;
    }
    stream.write_all(hasher.finalize().as_bytes()).await?;
    // TLS keeps the tail of each write buffered until flushed; without this the
    // end of a file before an idle period would never leave the device.
    stream.flush().await?;
    Ok(())
}

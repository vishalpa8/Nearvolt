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
use super::manifest::{plan_batches, FileSet, Manifest, MAX_FILES_PER_BATCH};
use super::state::hash_range;
use super::{classify, emit, is_fatal, Event, EventSender, FailureKind, Fatal, Stats, Summary};
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
/// A connection that lasted this long was working, so its loss isn't a reason
/// to wait longer before the next try.
const STABLE: Duration = Duration::from_secs(5);
const BLOCK: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct SendOptions {
    pub device_name: String,
    /// Parallel data streams; each carries one whole file at a time.
    pub connections: usize,
    /// How long to keep trying to reconnect after the link drops.
    pub reconnect_for: Duration,
    /// How long the first connection may take to be made before giving up. Much
    /// shorter than `reconnect_for`: a receiver that can't be reached at all
    /// is almost always on another network, not just slow.
    pub first_connect_for: Duration,
    /// Most files in one batch. A bigger selection is sent as several batches, one
    /// after another, because the offer lists every file in one message.
    pub max_files_per_batch: usize,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            device_name: "Nearvolt device".into(),
            connections: 8,
            reconnect_for: Duration::from_secs(60),
            first_connect_for: Duration::from_secs(20),
            max_files_per_batch: MAX_FILES_PER_BATCH,
        }
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
    let mut batches = Batches { max_files: options.max_files_per_batch, ..Batches::default() };
    let mut failing_since: Option<Instant> = None;
    let mut attempt = 0;
    // `ConnectionLost` goes out once per outage, and only once the connection
    // has been up: before that the app is still connecting.
    let mut up = false;
    let mut reported = false;
    loop {
        // Disconnect, and batches queued meanwhile, are handled even while connecting, so
        // cancelling a connection that's still being made takes effect at once.
        let connecting = tokio::select! {
            opened = open(&peer.link, &connector) => opened,
            command = commands.recv(), if !batches.closed => {
                if batches.handle(command, &events).await.is_break() {
                    return Ok(batches.summary(started));
                }
                continue;
            }
        };
        let error = match connecting {
            Ok(control) => {
                let opened = Instant::now();
                let mut ready = false;
                let result = session(
                    identity,
                    &connector,
                    peer,
                    control,
                    options,
                    &mut batches,
                    &mut commands,
                    &stats,
                    &events,
                    &mut ready,
                )
                .await;
                if ready {
                    (up, reported, failing_since) = (true, false, None);
                }
                match result {
                    Ok(()) => return Ok(batches.summary(started)),
                    Err(e) if is_fatal(&e) => return Err(e),
                    Err(e) => {
                        // Only a connection that lasted starts the backoff over; one the
                        // receiver drops at once is retried more and more slowly.
                        if opened.elapsed() >= STABLE {
                            attempt = 0;
                        }
                        e
                    }
                }
            }
            Err(e) => {
                if batches.finished() {
                    return Ok(batches.summary(started));
                }
                e
            }
        };
        if up && !reported {
            emit(&events, Event::ConnectionLost { reason: format!("{error:#}"), kind: FailureKind::OtherPhoneGone });
            reported = true;
        }
        let since = *failing_since.get_or_insert_with(Instant::now);
        let patience = if up { options.reconnect_for } else { options.first_connect_for };
        if since.elapsed() >= patience {
            let kind = if up { FailureKind::OtherPhoneGone } else { FailureKind::CannotReach };
            return Err(Fatal::new(kind, format!("could not reach the receiver: {error:#}")).into());
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

async fn open<L: Link>(link: &L, connector: &Connector) -> Result<TlsStream> {
    connector.connect(link.open_stream().await?).await
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(match attempt {
        1 => 100,
        2 => 250,
        3 => 500,
        4 => 1000,
        5..=8 => 2000,
        _ => 5000,
    })
}

/// Batches waiting their turn, and the one being sent. Kept across reconnects.
#[derive(Default)]
struct Batches {
    /// Most files in one batch; a bigger selection is cut into several.
    max_files: usize,
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
                // The app lists as many batches as the selection will be cut into, so a
                // failure has to account for all of them.
                let expected = match &set {
                    FileSet::Opened { files, .. } => plan_batches(files, self.max_files).len().max(1) as u32,
                    FileSet::Paths(_) => 1,
                };
                let max_files = self.max_files;
                match tokio::task::spawn_blocking(move || Manifest::build(set).map(|m| m.split(max_files))).await {
                    Ok(Ok(manifests)) => {
                        for manifest in manifests {
                            self.pending.push_back((self.next, Arc::new(manifest)));
                            self.next += 1;
                        }
                    }
                    Ok(Err(e)) => self.fail_all(events, expected, format!("{e:#}"), classify(&e)),
                    Err(e) => self.fail_all(events, expected, e.to_string(), FailureKind::Other),
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

    /// A selection that couldn't be turned into batches: each batch it would have
    /// been is reported failed, so the app's list and the engine's ids stay in step.
    fn fail_all(&mut self, events: &Option<EventSender>, count: u32, reason: String, kind: FailureKind) {
        for _ in 0..count {
            let batch = self.next;
            self.next += 1;
            emit(events, Event::BatchFailed { batch, reason: reason.clone(), kind });
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
    ready: &mut bool,
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
        Control::Reject { reason, kind } => return Err(Fatal::new(kind.unwrap_or(FailureKind::Other), reason).into()),
        other => bail!("unexpected reply: {other:?}"),
    };
    if let Some(ours) = nonce {
        let theirs = receiver_nonce.context("the receiver did not send its pairing nonce")?;
        write_msg(&mut wr, &Control::Reveal { nonce: ours }).await?;
        let code = pairing::pairing_code(&receiver, identity.fingerprint(), &ours, &theirs);
        emit(events, Event::Code { code });
    } else {
        emit(events, Event::Waiting);
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
                Some(Ok(Control::Reject { reason, kind })) => {
                    return Err(Fatal::new(kind.unwrap_or(FailureKind::Other), reason).into());
                }
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
                    return say_bye(out, writer, &mut control_rx).await;
                }
            }
        }
    }
    *ready = true;
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
            None if batches.closed => return say_bye(out, writer, &mut control_rx).await,
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
                Some(Ok(Control::Reject { reason, kind })) => {
                    if let Some((batch, _)) = batches.current.clone() {
                        fail_current(batches, &jobs_rx, events, batch, reason, kind.unwrap_or(FailureKind::Other));
                    }
                }
                // The receiver started the batch but can't go on, for example because its
                // storage filled up. What's left of it is dropped, the connection stays.
                Some(Ok(Control::Abort { batch, kind })) => {
                    let reason = "The receiver couldn't keep the files.".to_string();
                    fail_current(batches, &jobs_rx, events, batch, reason, kind);
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
                    return say_bye(out, writer, &mut control_rx).await;
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
                let current = batches.current.as_ref().map(|(batch, _)| *batch);
                // A file that can't be read fails its batch, not the connection: the
                // receiver is told to drop the batch, and the stream that was in the
                // middle of that file, now unusable, is replaced.
                if let (true, Some(batch)) = (is_fatal(&error), current) {
                    fail_current(batches, &jobs_rx, events, batch, format!("{error:#}"), classify(&error));
                    let _ = out.send(Control::Cancel { batch });
                    opening.push(open(&peer.link, connector));
                    continue;
                }
                // Nothing is being sent, so a broken data stream just gets replaced; if
                // the link is really gone, opening the replacement fails and says so.
                if current.is_none() {
                    opening.push(open(&peer.link, connector));
                    continue;
                }
                // A receiver that ends the connection, or drops the batch, closes its data
                // streams right away, and the message that says why can arrive a moment later.
                match settle(&mut control_rx).await {
                    Settled::Bye => {
                        emit(events, Event::PeerLeft);
                        return Ok(());
                    }
                    Settled::Abort { batch, kind } => {
                        let reason = "The receiver couldn't keep the files.".to_string();
                        fail_current(batches, &jobs_rx, events, batch, reason, kind);
                        opening.push(open(&peer.link, connector));
                        continue;
                    }
                    Settled::Neither => return Err(error),
                }
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

/// What the receiver says within a second of a data stream breaking.
enum Settled {
    Bye,
    Abort { batch: u32, kind: FailureKind },
    Neither,
}

async fn settle(control: &mut mpsc::UnboundedReceiver<Result<Control>>) -> Settled {
    let wait = async {
        while let Some(Ok(msg)) = control.recv().await {
            match msg {
                Control::Bye => return Settled::Bye,
                Control::Abort { batch, kind } => return Settled::Abort { batch, kind },
                _ => {}
            }
        }
        Settled::Neither
    };
    tokio::time::timeout(Duration::from_secs(1), wait).await.unwrap_or(Settled::Neither)
}

/// Gives up the batch being sent, if it's `batch`: nothing more of it is sent and
/// the app is told why. Returns whether it was the current batch.
fn fail_current(
    batches: &mut Batches,
    jobs: &async_channel::Receiver<Job>,
    events: &Option<EventSender>,
    batch: u32,
    reason: String,
    kind: FailureKind,
) -> bool {
    if !batches.current.as_ref().is_some_and(|(b, _)| *b == batch) {
        return false;
    }
    batches.current = None;
    while jobs.try_recv().is_ok() {}
    emit(events, Event::BatchFailed { batch, reason, kind });
    true
}

/// Tells the receiver the connection is over, so it stops waiting for a reconnect. The
/// goodbye is flushed and the stream closed properly, and what the receiver still sends
/// is read until it hangs up: closing a socket with unread data makes the system reset
/// it, which can throw the goodbye away and leave the receiver waiting for us to return.
async fn say_bye(
    out: mpsc::UnboundedSender<Control>,
    mut writer: AbortOnDrop<Result<()>>,
    control: &mut mpsc::UnboundedReceiver<Result<Control>>,
) -> Result<()> {
    let _ = out.send(Control::Bye);
    drop(out);
    let _ = tokio::time::timeout(Duration::from_secs(2), &mut writer.0).await;
    let drain = async { while let Some(Ok(_)) = control.recv().await {} };
    let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
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
                // Nothing more to say: close the stream properly (close_notify, then FIN).
                None => {
                    let _ = wr.shutdown().await;
                    return Ok(());
                }
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
    let unreadable =
        |e: std::io::Error| Fatal::new(FailureKind::CannotReadFile, format!("can't read {}: {e}", entry.path));
    let (m, path, stats) = (manifest.clone(), entry.path.clone(), stats.clone());
    let (mut source, mut hasher) = tokio::task::spawn_blocking(move || -> Result<_> {
        let source = m.open(file)?;
        let mut hasher = blake3::Hasher::new();
        hash_range(&source, 0, offset, &mut hasher, &stats)
            .map_err(|e| Fatal::new(FailureKind::CannotReadFile, format!("can't read {path}: {e}")))?;
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::net::TcpListener;

    use super::*;
    use crate::transport::{Acceptor, RawStream};

    /// Cancelling while the connection is still being made takes effect at once, not
    /// after the handshake times out.
    #[tokio::test]
    async fn disconnect_works_while_still_connecting() {
        // Accepts the connection and then says nothing, so the handshake just waits.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut keep = Vec::new();
            while let Ok((tcp, _)) = listener.accept().await {
                keep.push(tcp);
            }
        });
        let receiver = Identity::generate().unwrap();
        let sender = Identity::generate().unwrap();
        let peer = Peer {
            link: TcpLink::new(vec![addr]),
            name: "receiver".into(),
            fingerprint: Some(*receiver.fingerprint()),
            secret: Some([7; 16]),
        };
        let (commands, queue) = mpsc::unbounded_channel();
        let options = SendOptions::default();
        let connecting = tokio::spawn({
            let stats = Arc::new(Stats::default());
            async move { connect(&sender, &peer, &options, queue, stats, None).await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        let asked = Instant::now();
        commands.send(Command::Disconnect).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), connecting).await;
        held.abort();
        assert!(result.expect("it should stop at once").unwrap().is_ok());
        assert!(asked.elapsed() < Duration::from_secs(2));
    }

    /// A receiver that finishes the TLS handshake and reads the hello, then hangs
    /// up: every connection "works" and fails straight away.
    #[tokio::test]
    async fn a_receiver_that_hangs_up_at_once_is_retried_more_and_more_slowly() {
        let receiver = Identity::generate().unwrap();
        let acceptor = Acceptor::new(&receiver).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::Relaxed);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let raw: RawStream = Box::new(tcp);
                    if let Ok((mut stream, _)) = acceptor.handshake(raw).await {
                        let _ = read_msg(&mut stream).await;
                    }
                });
            }
        });

        let sender = Identity::generate().unwrap();
        let peer = Peer {
            link: TcpLink::new(vec![addr]),
            name: "receiver".into(),
            fingerprint: Some(*receiver.fingerprint()),
            secret: Some([7; 16]),
        };
        let options = SendOptions {
            reconnect_for: Duration::from_secs(2),
            first_connect_for: Duration::from_secs(2),
            ..SendOptions::default()
        };
        let (_commands, queue) = mpsc::unbounded_channel();
        let (events, mut seen) = mpsc::unbounded_channel();
        let result = connect(&sender, &peer, &options, queue, Arc::new(Stats::default()), Some(events)).await;

        assert!(result.is_err());
        // Without backoff this is hundreds of attempts in two seconds.
        let attempts = accepted.load(Ordering::Relaxed);
        assert!(attempts <= 8, "{attempts} attempts");
        while let Ok(event) = seen.try_recv() {
            assert!(!matches!(event, Event::ConnectionLost { .. }), "lost a connection that never came up");
        }
    }
}

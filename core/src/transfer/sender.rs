//! Sending side: offers files, then streams only the chunks the receiver is missing
//! over several parallel streams, reconnecting automatically when the link drops.

use std::fs::File;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::disk;
use super::manifest::{FileSet, Manifest};
use super::{emit, is_fatal, Event, EventSender, Fatal, Stats, Summary};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{self, Secret, Ticket};
use crate::protocol::{
    bit_get, chunk_range, read_msg, write_msg, ChunkHeader, ChunkRef, Control, FileHave, SessionId, DEFAULT_CHUNK_SIZE,
    HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, PROTOCOL_VERSION,
};
use crate::transport::{peer_fingerprint, Connector, Link, TcpLink, TlsStream};
use crate::util::AbortOnDrop;

/// With nothing left to send and no verified progress for this long, the session
/// is restarted so the receiver can report exactly what it is still missing.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct SendOptions {
    pub device_name: String,
    /// Parallel data streams.
    pub connections: usize,
    pub chunk_size: u32,
    /// How long to keep trying to reconnect after the link drops.
    pub reconnect_for: Duration,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            device_name: "Nearvolt device".into(),
            connections: 8,
            chunk_size: DEFAULT_CHUNK_SIZE,
            reconnect_for: Duration::from_secs(60),
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

/// Sends `files` to `peer`, resuming across as many reconnects as it takes.
/// Progress in `stats` is what the receiver has verified.
pub async fn send<L: Link>(
    identity: &Identity,
    peer: &Peer<L>,
    files: FileSet,
    options: &SendOptions,
    stats: Arc<Stats>,
    events: Option<EventSender>,
) -> Result<Summary> {
    let started = Instant::now();
    let chunk_size = options.chunk_size;
    let manifest = Arc::new(tokio::task::spawn_blocking(move || Manifest::build(files, chunk_size)).await??);
    let (total_files, total_bytes) = (manifest.offer.files.len() as u64, manifest.offer.total_bytes());
    stats.set_totals(total_files, total_bytes);

    let connector = Connector::new(identity, peer.fingerprint)?;
    let mut failing_since: Option<Instant> = None;
    let mut attempt = 0;
    loop {
        match open(&peer.link, &connector).await {
            Ok(control) => {
                attempt = 0;
                emit(&events, Event::Connected { peer: peer.name.clone() });
                match session(identity, &connector, peer, control, &manifest, options, &stats, &events).await {
                    Ok(()) => {
                        let summary = Summary { files: total_files, bytes: total_bytes, elapsed: started.elapsed() };
                        emit(&events, Event::Completed(summary));
                        return Ok(summary);
                    }
                    Err(e) if is_fatal(&e) => return Err(e),
                    Err(e) => {
                        emit(&events, Event::ConnectionLost { reason: format!("{e:#}") });
                        failing_since = Some(Instant::now());
                    }
                }
            }
            Err(e) => {
                let since = *failing_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= options.reconnect_for {
                    return Err(e.context("could not reach the receiver"));
                }
                attempt += 1;
                tokio::time::sleep(backoff(attempt)).await;
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

#[allow(clippy::too_many_arguments)]
async fn session<L: Link>(
    identity: &Identity,
    connector: &Connector,
    peer: &Peer<L>,
    control: TlsStream,
    manifest: &Arc<Manifest>,
    options: &SendOptions,
    stats: &Arc<Stats>,
    events: &Option<EventSender>,
) -> Result<()> {
    let offer = &manifest.offer;
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
    write_msg(&mut wr, &Control::Offer(offer.clone())).await?;
    let have = match read_msg(&mut rd).await? {
        Control::Accept { have } => have,
        Control::Reject { reason } => return Err(Fatal(reason).into()),
        other => bail!("unexpected reply: {other:?}"),
    };
    ensure!(have.len() == offer.files.len(), "the receiver answered for a different file list");

    let (queue_tx, queue_rx) = async_channel::unbounded();
    let (mut done_files, mut done_bytes) = (0u64, 0u64);
    for (i, state) in have.iter().enumerate() {
        let size = offer.files[i].size;
        let missing = |chunk| ChunkRef { file: i as u32, chunk };
        match state {
            FileHave::Complete => {
                done_files += 1;
                done_bytes += size;
            }
            FileHave::None => {
                for c in 0..offer.chunk_count(i) {
                    let _ = queue_tx.try_send(missing(c));
                }
            }
            FileHave::Partial { bitmap } => {
                for c in 0..offer.chunk_count(i) {
                    if bit_get(bitmap, c) {
                        done_bytes += chunk_range(size, offer.chunk_size, c).1 as u64;
                    } else {
                        let _ = queue_tx.try_send(missing(c));
                    }
                }
            }
        }
    }
    stats.set_done(done_files, done_bytes);
    if done_bytes > 0 {
        emit(events, Event::Resumed { done_bytes });
    }

    // The receiver reports verified progress every half second. Silence means the
    // link is gone, even if TCP hasn't noticed yet.
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let _reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            let msg = match tokio::time::timeout(HEARTBEAT_TIMEOUT, read_msg(&mut rd)).await {
                Ok(msg) => msg,
                Err(_) => Err(anyhow!("no word from the receiver for {} seconds", HEARTBEAT_TIMEOUT.as_secs())),
            };
            let failed = msg.is_err();
            if control_tx.send(msg).is_err() || failed {
                break;
            }
        }
    }));
    let _pinger = AbortOnDrop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            tick.tick().await;
            if write_msg(&mut wr, &Control::Ping).await.is_err() {
                break;
            }
        }
    }));

    let streams =
        futures::future::try_join_all((0..options.connections.max(1)).map(|_| open(&peer.link, connector))).await?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut workers = JoinSet::new();
    for stream in streams {
        workers.spawn(worker(stream, session_id, peer.secret, manifest.clone(), queue_rx.clone(), stop_rx.clone()));
    }

    let mut last_progress = (done_bytes, Instant::now());
    let mut stall_check = tokio::time::interval(Duration::from_secs(1));
    let result = loop {
        tokio::select! {
            msg = control_rx.recv() => match msg {
                Some(Ok(Control::Progress { files, bytes })) => {
                    stats.set_done(files, bytes);
                    if bytes != last_progress.0 {
                        last_progress = (bytes, Instant::now());
                    }
                }
                Some(Ok(Control::Resend { chunks })) => {
                    for c in chunks {
                        if (c.file as usize) < offer.files.len() && c.chunk < offer.chunk_count(c.file as usize) {
                            let _ = queue_tx.try_send(c);
                        }
                    }
                }
                Some(Ok(Control::Complete { files, bytes })) => {
                    stats.set_done(files, bytes);
                    break Ok(());
                }
                Some(Ok(Control::Reject { reason })) => break Err(Fatal(reason).into()),
                Some(Ok(other)) => break Err(anyhow!("unexpected message: {other:?}")),
                Some(Err(e)) => break Err(e),
                None => break Err(anyhow!("the control connection closed")),
            },
            Some(joined) = workers.join_next() => match joined {
                Ok(Ok(())) => {}
                Ok(Err(e)) => break Err(e),
                Err(e) => break Err(anyhow!("a send worker failed: {e}")),
            },
            _ = stall_check.tick() => {
                if queue_tx.is_empty() && last_progress.1.elapsed() >= STALL_TIMEOUT {
                    break Err(anyhow!("the receiver stopped making progress; resynchronizing"));
                }
            }
        }
    };
    let _ = stop_tx.send(true);
    workers.shutdown().await;
    result
}

/// Pulls chunks off the shared queue and writes them to one data stream.
async fn worker(
    mut stream: TlsStream,
    session: SessionId,
    secret: Option<Secret>,
    manifest: Arc<Manifest>,
    queue: async_channel::Receiver<ChunkRef>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let hello = Control::Hello {
        version: PROTOCOL_VERSION,
        secret,
        device_name: String::new(),
        session: Some(session),
        commitment: None,
    };
    write_msg(&mut stream, &hello).await?;
    let mut buf = Vec::new();
    let mut cache: Option<(u32, File)> = None;
    loop {
        let job = tokio::select! {
            job = queue.recv() => match job {
                Ok(job) => job,
                Err(_) => break,
            },
            _ = stop.changed() => break,
        };
        let m = manifest.clone();
        let (b, c, hash) = tokio::task::spawn_blocking(move || {
            let (mut buf, mut cache) = (buf, cache);
            let hash = read_chunk(&m, job, &mut cache, &mut buf)?;
            anyhow::Ok((buf, cache, hash))
        })
        .await??;
        (buf, cache) = (b, c);
        let header = ChunkHeader { file: job.file, chunk: job.chunk, len: buf.len() as u32, hash };
        stream.write_all(&header.encode()).await?;
        stream.write_all(&buf).await?;
        // TLS keeps the tail of each write buffered until flushed; without this the
        // last chunk before an idle period would never leave the device.
        stream.flush().await?;
    }
    Ok(())
}

fn read_chunk(
    manifest: &Manifest,
    job: ChunkRef,
    cache: &mut Option<(u32, File)>,
    buf: &mut Vec<u8>,
) -> Result<[u8; 32]> {
    if cache.as_ref().map(|(i, _)| *i) != Some(job.file) {
        *cache = Some((job.file, manifest.open(job.file)?));
    }
    let (_, file) = cache.as_ref().expect("cached above");
    let entry = &manifest.offer.files[job.file as usize];
    let (offset, len) = chunk_range(entry.size, manifest.offer.chunk_size, job.chunk);
    buf.resize(len as usize, 0);
    disk::read_exact_at(file, buf, offset).map_err(|e| Fatal(format!("can't read {}: {e}", entry.path)))?;
    Ok(*blake3::hash(buf).as_bytes())
}

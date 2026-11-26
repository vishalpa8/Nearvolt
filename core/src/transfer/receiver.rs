//! Receiving side: accepts authenticated senders, verifies and writes chunks, and
//! journals progress so interrupted transfers resume where they stopped.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures::future::BoxFuture;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::state::{plan_files, transfer_id, TransferState};
use super::{emit, is_fatal, Event, EventSender, Fatal, Stats, Summary};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{self, constant_time_eq, truncate_utf8, Secret, Ticket, MAX_NAME_LEN};
use crate::protocol::{
    read_header, read_msg, write_msg, ChunkHeader, ChunkRef, Control, SessionId, CHUNK_HEADER_LEN,
    HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, PROTOCOL_VERSION,
};
use crate::transport::{Acceptor, Listener, TlsStream};
use crate::util::AbortOnDrop;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_AUTH_FAILURES: u32 = 20;
const STATE_DIR: &str = ".nearvolt";

#[derive(Clone, Debug)]
pub struct ReceiveOptions {
    /// Destination folder.
    pub dir: PathBuf,
    /// Where transfer journals live. Defaults to `<dir>/.nearvolt`.
    pub journal_dir: Option<PathBuf>,
    pub device_name: String,
    /// Re-read every finished file from disk and check it against the chunk hashes.
    pub verify_on_disk: bool,
}

impl ReceiveOptions {
    fn journal_root(&self) -> PathBuf {
        self.journal_dir.clone().unwrap_or_else(|| self.dir.join(STATE_DIR))
    }
}

#[derive(Clone, Debug)]
pub struct IncomingOffer {
    pub sender_name: String,
    pub files: u64,
    pub bytes: u64,
    /// The first few paths, for an approval prompt.
    pub sample: Vec<String>,
}

/// Decides whether to accept an offer, for example by asking the user.
pub type Approver = Arc<dyn Fn(IncomingOffer) -> BoxFuture<'static, bool> + Send + Sync>;

pub struct Receiver {
    listener: Listener,
    ticket: Ticket,
    options: Arc<ReceiveOptions>,
}

impl Receiver {
    pub fn bind(identity: &Identity, bind: SocketAddr, options: ReceiveOptions) -> Result<Self> {
        let listener = Listener::bind(identity, bind)?;
        let port = listener.local_addr()?.port();
        let addrs = if bind.ip().is_unspecified() {
            pairing::local_addrs(port)
        } else {
            vec![SocketAddr::new(bind.ip(), port)]
        };
        let ticket = Ticket {
            fingerprint: *identity.fingerprint(),
            secret: pairing::new_secret(),
            addrs,
            name: options.device_name.clone(),
        };
        Ok(Self { listener, ticket, options: Arc::new(options) })
    }

    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Receives one transfer, across as many reconnects as it takes.
    pub async fn run(&self, approve: Approver, stats: Arc<Stats>, events: Option<EventSender>) -> Result<Summary> {
        fs::create_dir_all(&self.options.dir)
            .with_context(|| format!("can't create {}", self.options.dir.display()))?;
        let shared = Arc::new(Shared {
            options: self.options.clone(),
            approve,
            stats,
            transfer: tokio::sync::Mutex::new(None),
        });
        let (auth_tx, mut auth_rx) = mpsc::channel::<Option<Authed>>(32);
        let mut current: Option<Current> = None;
        let mut failures = 0;
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let Ok((tcp, acceptor)) = accepted else { continue };
                    let (tx, secret) = (auth_tx.clone(), self.ticket.secret);
                    tokio::spawn(async move {
                        let _ = tx.send(authenticate(tcp, acceptor, secret).await.ok()).await;
                    });
                }
                Some(authed) = auth_rx.recv() => {
                    let Some(authed) = authed else {
                        failures += 1;
                        if failures >= MAX_AUTH_FAILURES {
                            return Err(Fatal("Too many failed pairing attempts. Start a new receive session.".into()).into());
                        }
                        continue;
                    };
                    match authed.session {
                        Some(session) => {
                            if let Some(c) = current.as_ref().filter(|c| c.session == session && c.sender == authed.sender) {
                                let _ = c.data_tx.try_send(authed.stream);
                            }
                        }
                        None => {
                            // A reconnecting sender replaces its stale session at once
                            // instead of waiting for the old one to time out.
                            drop(current.take());
                            emit(&events, Event::Connected { peer: authed.sender_name.clone() });
                            let (session, sender) = (pairing::new_secret(), authed.sender);
                            let (data_tx, data_rx) = mpsc::channel(64);
                            let task = AbortOnDrop(tokio::spawn(serve(shared.clone(), authed, session, data_rx)));
                            current = Some(Current { session, sender, data_tx, task });
                        }
                    }
                }
                result = join(&mut current), if current.is_some() => {
                    current = None;
                    match result {
                        Ok(Outcome::Completed(summary)) => {
                            emit(&events, Event::Completed(summary));
                            return Ok(summary);
                        }
                        Ok(Outcome::Declined) => return Err(Fatal("The transfer was declined.".into()).into()),
                        Ok(Outcome::Busy) => {}
                        Err(e) if is_fatal(&e) => return Err(e),
                        Err(e) => emit(&events, Event::ConnectionLost { reason: format!("{e:#}") }),
                    }
                }
            }
        }
    }
}

struct Shared {
    options: Arc<ReceiveOptions>,
    approve: Approver,
    stats: Arc<Stats>,
    transfer: tokio::sync::Mutex<Option<Arc<TransferState>>>,
}

enum Outcome {
    Completed(Summary),
    Declined,
    Busy,
}

struct Authed {
    stream: TlsStream,
    sender: Fingerprint,
    sender_name: String,
    /// `None` for a control connection; the session it belongs to for a data connection.
    session: Option<SessionId>,
}

/// The session currently being served. Data connections join it by session ID.
struct Current {
    session: SessionId,
    sender: Fingerprint,
    data_tx: mpsc::Sender<TlsStream>,
    task: AbortOnDrop<Result<Outcome>>,
}

async fn join(current: &mut Option<Current>) -> Result<Outcome> {
    match current {
        Some(c) => (&mut c.task.0).await.unwrap_or_else(|e| Err(anyhow!("connection task failed: {e}"))),
        None => std::future::pending().await,
    }
}

async fn authenticate(tcp: TcpStream, acceptor: Acceptor, secret: Secret) -> Result<Authed> {
    let (mut stream, sender) = acceptor.handshake(tcp).await?;
    let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await??;
    let Control::Hello { version, secret: offered, device_name, session } = hello else {
        bail!("expected hello");
    };
    if !constant_time_eq(&offered, &secret) {
        let _ = write_msg(&mut stream, &reject("This code is not valid. Scan the receiver's code again.")).await;
        bail!("invalid ticket secret");
    }
    if version != PROTOCOL_VERSION {
        let _ = write_msg(&mut stream, &reject("Update Nearvolt on both devices.")).await;
        bail!("unsupported protocol version {version}");
    }
    let sender_name = truncate_utf8(device_name.trim(), MAX_NAME_LEN).to_string();
    Ok(Authed { stream, sender, sender_name, session })
}

fn reject(reason: &str) -> Control {
    Control::Reject { reason: reason.to_string() }
}

async fn serve(
    shared: Arc<Shared>,
    authed: Authed,
    session: SessionId,
    mut data_rx: mpsc::Receiver<TlsStream>,
) -> Result<Outcome> {
    let Authed { stream, sender, sender_name, .. } = authed;
    let (mut recv, mut send) = tokio::io::split(stream);
    let welcome = Control::Welcome { device_name: shared.options.device_name.clone(), session };
    write_msg(&mut send, &welcome).await?;
    let offer = match read_msg(&mut recv).await? {
        Control::Offer(offer) => offer,
        other => bail!("expected an offer, got {other:?}"),
    };
    let id = transfer_id(&sender, &offer);

    let mut slot = shared.transfer.lock().await;
    let state = match slot.as_ref() {
        Some(state) if state.id == id => state.clone(),
        Some(_) => {
            let _ = write_msg(&mut send, &reject("This device is busy with another transfer.")).await;
            return Ok(Outcome::Busy);
        }
        None => {
            let files = match plan_files(&offer) {
                Ok(files) => files,
                Err(e) => {
                    let _ = write_msg(&mut send, &reject(&format!("{e:#}"))).await;
                    return Err(e);
                }
            };
            let incoming = IncomingOffer {
                sender_name,
                files: offer.files.len() as u64,
                bytes: offer.total_bytes(),
                sample: offer.files.iter().take(5).map(|f| f.path.clone()).collect(),
            };
            if !(shared.approve)(incoming).await {
                let _ = write_msg(&mut send, &reject("The receiver declined.")).await;
                return Ok(Outcome::Declined);
            }
            let options = shared.options.clone();
            let (stats, chunk_size, id) = (shared.stats.clone(), offer.chunk_size, id.clone());
            let opened = tokio::task::spawn_blocking(move || {
                TransferState::open(
                    id,
                    chunk_size,
                    files,
                    &options.dir,
                    &options.journal_root(),
                    options.verify_on_disk,
                    stats,
                )
            })
            .await?;
            let state = match opened {
                Ok(state) => state,
                Err(e) => {
                    let _ = write_msg(&mut send, &reject(&format!("{e:#}"))).await;
                    return Err(e);
                }
            };
            *slot = Some(state.clone());
            state
        }
    };
    drop(slot);

    write_msg(&mut send, &Control::Accept { have: state.have_list() }).await?;

    // The sender pings every half second. Silence means the link is gone, even if
    // TCP hasn't noticed yet.
    let (lost_tx, mut lost_rx) = mpsc::unbounded_channel();
    let _reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            match tokio::time::timeout(HEARTBEAT_TIMEOUT, read_msg(&mut recv)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => break lost_tx.send(e),
                Err(_) => {
                    break lost_tx.send(anyhow!("no word from the sender for {} seconds", HEARTBEAT_TIMEOUT.as_secs()))
                }
            }
        }
    }));

    let (resend_tx, mut resend_rx) = mpsc::unbounded_channel();
    let mut readers = JoinSet::new();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut complete = state.complete.subscribe();
    loop {
        tokio::select! {
            done = async { complete.wait_for(|done| *done).await.map(|_| ()) } => {
                done?;
                break;
            }
            Some(stream) = data_rx.recv() => {
                readers.spawn(read_data(stream, state.clone(), resend_tx.clone()));
            }
            Some(chunks) = resend_rx.recv() => {
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &Control::Resend { chunks })).await??;
            }
            Some(e) = lost_rx.recv() => return Err(e),
            Some(joined) = readers.join_next() => joined??,
            _ = heartbeat.tick() => {
                let p = shared.stats.snapshot();
                let progress = Control::Progress { files: p.done_files, bytes: p.done_bytes };
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &progress)).await??;
            }
        }
    }

    let summary = state.summary();
    write_msg(&mut send, &Control::Complete { files: summary.files, bytes: summary.bytes }).await?;
    // Give the sender a moment to read the result and close the connection itself.
    let _ = tokio::time::timeout(Duration::from_secs(2), lost_rx.recv()).await;
    state.cleanup();
    Ok(Outcome::Completed(summary))
}

async fn read_data(
    mut stream: TlsStream,
    state: Arc<TransferState>,
    resend: mpsc::UnboundedSender<Vec<ChunkRef>>,
) -> Result<()> {
    let mut header = [0u8; CHUNK_HEADER_LEN];
    let mut buf = Vec::new();
    while read_header(&mut stream, &mut header).await? {
        let h = ChunkHeader::decode(&header);
        let len = state.expected_len(&h)?;
        buf.resize(len as usize, 0);
        stream.read_exact(&mut buf).await?;
        let st = state.clone();
        let data = std::mem::take(&mut buf);
        let (data, stored) = tokio::task::spawn_blocking(move || {
            let stored = st.store_chunk(h, &data);
            (data, stored)
        })
        .await?;
        buf = data;
        let bad = stored?;
        if !bad.is_empty() {
            let _ = resend.send(bad);
        }
    }
    Ok(())
}

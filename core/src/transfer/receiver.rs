//! Receiving side: accepts authenticated senders, verifies and writes chunks, and
//! journals progress so interrupted transfers resume where they stopped.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use tokio::io::AsyncReadExt;
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
use crate::transport::{listen_tcp, Acceptor, RawStream, TlsStream};
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
    /// Accept senders without the ticket secret once the user confirms a code.
    pub allow_code_pairing: bool,
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
    /// Present when the sender paired without the ticket secret; the user should
    /// check the sender shows the same code before accepting.
    pub code: Option<String>,
}

/// Decides whether to accept an offer, for example by asking the user.
pub type Approver = Arc<dyn Fn(IncomingOffer) -> BoxFuture<'static, bool> + Send + Sync>;

pub struct Receiver {
    acceptor: Acceptor,
    fingerprint: Fingerprint,
    secret: Secret,
    bind_ip: IpAddr,
    port: u16,
    options: Arc<ReceiveOptions>,
    streams_tx: mpsc::Sender<RawStream>,
    streams_rx: tokio::sync::Mutex<mpsc::Receiver<RawStream>>,
    _tcp: AbortOnDrop<()>,
}

impl Receiver {
    /// Listens for TCP on `bind`. Other links (USB) can add streams through
    /// [`Receiver::streams`].
    pub fn bind(identity: &Identity, bind: SocketAddr, options: ReceiveOptions) -> Result<Self> {
        let (streams_tx, streams_rx) = mpsc::channel(64);
        let (local, tcp) = listen_tcp(bind, streams_tx.clone())?;
        Ok(Self {
            acceptor: Acceptor::new(identity)?,
            fingerprint: *identity.fingerprint(),
            secret: pairing::new_secret(),
            bind_ip: bind.ip(),
            port: local.port(),
            options: Arc::new(options),
            streams_tx,
            streams_rx: tokio::sync::Mutex::new(streams_rx),
            _tcp: tcp,
        })
    }

    /// A ticket with this device's current addresses and the secret.
    pub fn ticket(&self) -> Ticket {
        let addrs = if self.bind_ip.is_unspecified() {
            pairing::local_addrs(self.port)
        } else {
            vec![SocketAddr::new(self.bind_ip, self.port)]
        };
        Ticket {
            fingerprint: self.fingerprint,
            secret: Some(self.secret),
            addrs,
            name: self.options.device_name.clone(),
            wifi: None,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.port)
    }

    /// Feeds raw streams from another link, such as a USB cable.
    pub fn streams(&self) -> mpsc::Sender<RawStream> {
        self.streams_tx.clone()
    }

    /// Receives one transfer, across as many reconnects as it takes.
    pub async fn run(&self, approve: Approver, stats: Arc<Stats>, events: Option<EventSender>) -> Result<Summary> {
        fs::create_dir_all(&self.options.dir)
            .with_context(|| format!("can't create {}", self.options.dir.display()))?;
        let shared = Arc::new(Shared {
            options: self.options.clone(),
            fingerprint: self.fingerprint,
            approve,
            stats,
            transfer: tokio::sync::Mutex::new(None),
        });
        let mut incoming = self.streams_rx.lock().await;
        let (auth_tx, mut auth_rx) = mpsc::channel::<Option<Authed>>(32);
        let mut current: Option<Current> = None;
        let mut failures = 0;
        loop {
            tokio::select! {
                Some(raw) = incoming.recv() => {
                    let tx = auth_tx.clone();
                    let (acceptor, secret, code) = (self.acceptor.clone(), self.secret, self.options.allow_code_pairing);
                    tokio::spawn(async move {
                        let _ = tx.send(authenticate(raw, acceptor, secret, code).await.ok()).await;
                    });
                }
                Some(authed) = auth_rx.recv() => {
                    let Some(authed) = authed else {
                        failures += 1;
                        if failures >= MAX_AUTH_FAILURES {
                            return Err(Fatal("Too many failed pairing attempts. Open Receive again.".into()).into());
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
    fingerprint: Fingerprint,
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
    /// `None` for a control stream; the session it belongs to for a data stream.
    session: Option<SessionId>,
    /// Present when the sender pairs by code instead of the ticket secret.
    commitment: Option<[u8; 32]>,
}

/// The session currently being served. Data streams join it by session ID.
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

async fn authenticate(raw: RawStream, acceptor: Acceptor, secret: Secret, allow_code: bool) -> Result<Authed> {
    let (mut stream, sender) = acceptor.handshake(raw).await?;
    let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await??;
    let Control::Hello { version, secret: offered, device_name, session, commitment } = hello else {
        bail!("expected hello");
    };
    if version != PROTOCOL_VERSION {
        let _ = write_msg(&mut stream, &reject("Update Nearvolt on both devices.")).await;
        bail!("unsupported protocol version {version}");
    }
    let allowed = match (offered, session) {
        (Some(offered), _) => constant_time_eq(&offered, &secret),
        // A data stream; the caller checks its session ID and certificate.
        (None, Some(_)) => true,
        (None, None) => allow_code && commitment.is_some(),
    };
    if !allowed {
        let _ = write_msg(&mut stream, &reject("This code is not valid. Scan the receiver's code again.")).await;
        bail!("unauthorized sender");
    }
    let sender_name = truncate_utf8(device_name.trim(), MAX_NAME_LEN).to_string();
    let commitment = if offered.is_none() { commitment } else { None };
    Ok(Authed { stream, sender, sender_name, session, commitment })
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
    let Authed { stream, sender, sender_name, commitment, .. } = authed;
    let (mut recv, mut send) = tokio::io::split(stream);
    let nonce = commitment.map(|_| pairing::new_secret());
    let welcome = Control::Welcome { device_name: shared.options.device_name.clone(), session, nonce };
    write_msg(&mut send, &welcome).await?;
    let code = match (commitment, nonce) {
        (Some(commitment), Some(ours)) => {
            let Control::Reveal { nonce: theirs } = read_msg(&mut recv).await? else {
                bail!("expected the sender's pairing nonce");
            };
            ensure!(
                constant_time_eq(&pairing::commitment(&theirs), &commitment),
                "the sender's pairing nonce does not match its commitment"
            );
            Some(pairing::pairing_code(&shared.fingerprint, &sender, &theirs, &ours))
        }
        _ => None,
    };
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
                code,
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

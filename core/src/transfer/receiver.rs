//! Receiving side: accepts one sender at a time and keeps its connection while it
//! sends batch after batch. Every file is written, verified on disk and journaled,
//! so an interrupted transfer resumes where it stopped.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use tokio::io::{AsyncReadExt, ReadHalf};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::state::{plan_files, sweep_stale, transfer_id, TransferState};
use super::{emit, is_fatal, Event, EventSender, Fatal, Stats, Summary};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{self, constant_time_eq, truncate_utf8, Secret, Ticket, MAX_NAME_LEN};
use crate::protocol::{
    read_header, read_msg, write_msg, Control, FileHeader, Offer, SessionId, FILE_HEADER_LEN, HASH_LEN,
    HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, PROTOCOL_VERSION,
};
use crate::transport::{listen_tcp, Acceptor, RawStream, TlsStream};
use crate::util::AbortOnDrop;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_AUTH_FAILURES: u32 = 20;
const STATE_DIR: &str = ".nearvolt";
/// A connection that dropped and isn't back after this long is ended on both phones.
const RECONNECT_WAIT: Duration = Duration::from_secs(5 * 60);
/// A half-received file nobody came back for in this long is removed when Receive opens.
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const BLOCK: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct ReceiveOptions {
    /// Destination folder.
    pub dir: PathBuf,
    /// Where transfer journals live. Defaults to `<dir>/.nearvolt`.
    pub journal_dir: Option<PathBuf>,
    pub device_name: String,
    /// Accept senders without the ticket secret once the user confirms a code.
    pub allow_code_pairing: bool,
}

impl ReceiveOptions {
    fn journal_root(&self) -> PathBuf {
        self.journal_dir.clone().unwrap_or_else(|| self.dir.join(STATE_DIR))
    }
}

/// A phone asking to connect.
#[derive(Clone, Debug)]
pub struct Request {
    pub sender_name: String,
    /// Present when the sender paired without the ticket secret; the user should
    /// check the sender shows the same code before accepting.
    pub code: Option<String>,
}

/// Decides whether to accept a connection, for example by asking the user.
pub type Approver = Arc<dyn Fn(Request) -> BoxFuture<'static, bool> + Send + Sync>;

pub struct Receiver {
    acceptor: Acceptor,
    fingerprint: Fingerprint,
    secret: Secret,
    tap_secret: Secret,
    bind_ip: IpAddr,
    port: u16,
    options: Arc<ReceiveOptions>,
    streams_tx: mpsc::Sender<RawStream>,
    streams_rx: tokio::sync::Mutex<mpsc::Receiver<RawStream>>,
    ending: watch::Sender<bool>,
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
            tap_secret: pairing::new_secret(),
            bind_ip: bind.ip(),
            port: local.port(),
            options: Arc::new(options),
            streams_tx,
            streams_rx: tokio::sync::Mutex::new(streams_rx),
            ending: watch::channel(false).0,
            _tcp: tcp,
        })
    }

    /// A ticket with this device's current addresses and the secret, for the QR code.
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

    /// The ticket served over NFC. Its own secret tells the receiver the phones
    /// were tapped together, which is consent enough to connect without asking.
    pub fn tap_ticket(&self) -> Ticket {
        Ticket { secret: Some(self.tap_secret), ..self.ticket() }
    }

    pub fn local_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.port)
    }

    /// Feeds raw streams from another link, such as a USB cable.
    pub fn streams(&self) -> mpsc::Sender<RawStream> {
        self.streams_tx.clone()
    }

    /// Ends the connection: the sender is told, so it stops reconnecting, and
    /// [`Receiver::run`] returns.
    pub fn disconnect(&self) {
        self.ending.send_replace(true);
    }

    /// Serves one sender's connection, across as many reconnects as it takes,
    /// until either side ends it. Returns the totals received.
    pub async fn run(&self, approve: Approver, stats: Arc<Stats>, events: Option<EventSender>) -> Result<Summary> {
        fs::create_dir_all(&self.options.dir)
            .with_context(|| format!("can't create {}", self.options.dir.display()))?;
        let (dir, journals) = (self.options.dir.clone(), self.options.journal_root());
        tokio::task::spawn_blocking(move || sweep_stale(&dir, &journals, STALE_AFTER));
        let shared = Arc::new(Shared {
            options: self.options.clone(),
            fingerprint: self.fingerprint,
            approve,
            stats,
            events: events.clone(),
            trusted: Mutex::new(None),
            active: tokio::sync::Mutex::new(None),
            totals: Mutex::new((0, 0)),
            started: Instant::now(),
        });
        let mut incoming = self.streams_rx.lock().await;
        let mut ending = self.ending.subscribe();
        let (auth_tx, mut auth_rx) = mpsc::channel::<Option<Authed>>(32);
        let mut current: Option<Current> = None;
        let mut failures = 0;
        // When a connection that was working dropped; cleared when the sender is back.
        let mut lost_since: Option<Instant> = None;
        loop {
            let still_waiting = lost_since.map(|t| RECONNECT_WAIT.saturating_sub(t.elapsed()));
            tokio::select! {
                // The sender had its chance to come back; stop waiting and listening.
                () = tokio::time::sleep(still_waiting.unwrap_or_default()), if still_waiting.is_some() && current.is_none() => {
                    return Err(Fatal("The other phone didn't come back after the link dropped.".into()).into());
                }
                Some(raw) = incoming.recv() => {
                    let tx = auth_tx.clone();
                    let (acceptor, secrets, code) =
                        (self.acceptor.clone(), [self.secret, self.tap_secret], self.options.allow_code_pairing);
                    tokio::spawn(async move {
                        let _ = tx.send(authenticate(raw, acceptor, secrets, code).await.ok()).await;
                    });
                }
                Some(authed) = auth_rx.recv() => {
                    let Some(mut authed) = authed else {
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
                            let trusted = *shared.trusted.lock().unwrap();
                            if trusted.is_some_and(|f| f != authed.sender) {
                                let busy = reject("This phone is connected to another device.");
                                tokio::spawn(async move { write_msg(&mut authed.stream, &busy).await });
                                continue;
                            }
                            // A reconnecting sender replaces its stale session at once
                            // instead of waiting for the old one to time out.
                            drop(current.take());
                            lost_since = None;
                            let (session, sender) = (pairing::new_secret(), authed.sender);
                            let (data_tx, data_rx) = mpsc::channel(64);
                            let task = serve(shared.clone(), authed, session, data_rx, self.ending.subscribe());
                            current = Some(Current { session, sender, data_tx, task: AbortOnDrop(tokio::spawn(task)) });
                        }
                    }
                }
                result = join(&mut current), if current.is_some() => {
                    current = None;
                    match result {
                        Ok(Outcome::Ended) => {
                            shared.discard_unfinished().await;
                            return Ok(shared.summary());
                        }
                        Ok(Outcome::Declined) => {}
                        Err(e) if is_fatal(&e) => return Err(e),
                        Err(e) => {
                            // Only a connection that was set up has anything to wait for.
                            if shared.trusted.lock().unwrap().is_some() {
                                lost_since.get_or_insert_with(Instant::now);
                            }
                            emit(&events, Event::ConnectionLost { reason: format!("{e:#}") });
                        }
                    }
                }
                _ = ending.changed(), if current.is_none() => {
                    shared.discard_unfinished().await;
                    return Ok(shared.summary());
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
    events: Option<EventSender>,
    /// The sender the user accepted. It reconnects without being asked about again.
    trusted: Mutex<Option<Fingerprint>>,
    /// The batch being received. It outlives a dropped connection, so the sender
    /// resumes it after reconnecting.
    active: tokio::sync::Mutex<Option<Active>>,
    totals: Mutex<(u64, u64)>,
    started: Instant,
}

impl Shared {
    fn summary(&self) -> Summary {
        let (files, bytes) = *self.totals.lock().unwrap();
        Summary { files, bytes, elapsed: self.started.elapsed() }
    }

    /// A deliberate disconnect leaves nothing to resume, so a batch still in
    /// progress is thrown away rather than kept as hidden part files.
    async fn discard_unfinished(&self) {
        let Some(Active { state, .. }) = self.active.lock().await.take() else { return };
        let _ = tokio::task::spawn_blocking(move || state.discard()).await;
    }
}

struct Active {
    batch: u32,
    state: Arc<TransferState>,
}

enum Outcome {
    /// Either side ended the connection on purpose.
    Ended,
    /// The user turned the sender away; Receive stays open for someone else.
    Declined,
}

struct Authed {
    stream: TlsStream,
    sender: Fingerprint,
    sender_name: String,
    /// `None` for a control stream; the connection it belongs to for a data stream.
    session: Option<SessionId>,
    /// Present when the sender pairs by code instead of the ticket secret.
    commitment: Option<[u8; 32]>,
    /// The sender got the ticket by tapping phones.
    tapped: bool,
}

/// The connection currently being served. Data streams join it by session ID.
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

/// `secrets` holds the QR code's secret, then the NFC tap's.
async fn authenticate(raw: RawStream, acceptor: Acceptor, secrets: [Secret; 2], allow_code: bool) -> Result<Authed> {
    let (mut stream, sender) = acceptor.handshake(raw).await?;
    let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await??;
    let Control::Hello { version, secret: offered, device_name, session, commitment } = hello else {
        bail!("expected hello");
    };
    if version != PROTOCOL_VERSION {
        let _ = write_msg(&mut stream, &reject("Update Nearvolt on both devices.")).await;
        bail!("unsupported protocol version {version}");
    }
    let tapped = offered.is_some_and(|s| constant_time_eq(&s, &secrets[1]));
    let allowed = match (offered, session) {
        (Some(offered), _) => tapped || constant_time_eq(&offered, &secrets[0]),
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
    Ok(Authed { stream, sender, sender_name, session, commitment, tapped })
}

fn reject(reason: &str) -> Control {
    Control::Reject { reason: reason.to_string() }
}

async fn serve(
    shared: Arc<Shared>,
    authed: Authed,
    session: SessionId,
    mut data_rx: mpsc::Receiver<TlsStream>,
    mut ending: watch::Receiver<bool>,
) -> Result<Outcome> {
    let Authed { stream, sender, sender_name, commitment, tapped, .. } = authed;
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

    // The sender pings every half second from here on, so a vanished sender is
    // noticed even while the user is still deciding.
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let _reader = AbortOnDrop(tokio::spawn(read_loop(recv, control_tx)));

    let accepted = tapped || *shared.trusted.lock().unwrap() == Some(sender);
    if !accepted {
        let request = Request { sender_name: sender_name.clone(), code };
        let decision = (shared.approve)(request);
        tokio::pin!(decision);
        let approved = loop {
            tokio::select! {
                approved = &mut decision => break approved,
                // Pings keep arriving while the user decides; only a goodbye, an
                // error or a closed stream means the sender is gone.
                msg = control_rx.recv() => match msg {
                    Some(Ok(Control::Ping)) => {}
                    Some(Ok(Control::Bye)) => {
                        emit(&shared.events, Event::PeerLeft);
                        return Ok(Outcome::Ended);
                    }
                    Some(Err(e)) => return Err(e),
                    Some(Ok(other)) => bail!("unexpected message before the connection was accepted: {other:?}"),
                    None => bail!("the sender left before the connection was accepted"),
                },
                _ = ending.changed() => {
                    let _ = write_msg(&mut send, &Control::Bye).await;
                    return Ok(Outcome::Ended);
                }
            }
        };
        if !approved {
            let _ = write_msg(&mut send, &reject("The receiver declined.")).await;
            return Ok(Outcome::Declined);
        }
    }
    *shared.trusted.lock().unwrap() = Some(sender);
    write_msg(&mut send, &Control::Ready).await?;
    emit(&shared.events, Event::Connected { peer: sender_name, route: String::new() });

    let (resend_tx, mut resend_rx) = mpsc::unbounded_channel();
    let mut readers = JoinSet::new();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut complete: Option<watch::Receiver<bool>> = None;
    loop {
        tokio::select! {
            msg = control_rx.recv() => match msg {
                Some(Ok(Control::Offer { batch, files })) => match open_batch(&shared, sender, batch, Offer { files }).await {
                    Ok(state) => {
                        write_msg(&mut send, &Control::Accept { batch, have: state.have_list() }).await?;
                        complete = Some(state.complete.subscribe());
                    }
                    Err(e) => {
                        emit(&shared.events, Event::BatchFailed { batch, reason: format!("{e:#}") });
                        write_msg(&mut send, &reject(&format!("{e:#}"))).await?;
                    }
                },
                Some(Ok(Control::Ping)) => {}
                // The sender gave up this batch: throw away what arrived of it.
                Some(Ok(Control::Cancel { batch })) => {
                    let mut active = shared.active.lock().await;
                    if active.as_ref().is_some_and(|a| a.batch == batch) {
                        if let Some(Active { state, .. }) = active.take() {
                            complete = None;
                            drop(active);
                            let _ = tokio::task::spawn_blocking(move || state.discard()).await;
                            let reason = "The sender couldn't read one of the files.".to_string();
                            emit(&shared.events, Event::BatchFailed { batch, reason });
                        }
                    }
                }
                Some(Ok(Control::Bye)) => {
                    emit(&shared.events, Event::PeerLeft);
                    return Ok(Outcome::Ended);
                }
                Some(Ok(other)) => bail!("unexpected message: {other:?}"),
                Some(Err(e)) => return Err(e),
                None => bail!("the control connection closed"),
            },
            Some(stream) = data_rx.recv() => {
                readers.spawn(read_data(stream, shared.clone(), resend_tx.clone()));
            }
            Some(files) = resend_rx.recv() => {
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &Control::Resend { files })).await??;
            }
            () = finished(&mut complete) => {
                complete = None;
                let Some(Active { batch, state }) = shared.active.lock().await.take() else { continue };
                let summary = state.summary();
                write_msg(&mut send, &Control::Complete { batch, files: summary.files, bytes: summary.bytes }).await?;
                state.cleanup();
                {
                    let mut totals = shared.totals.lock().unwrap();
                    *totals = (totals.0 + summary.files, totals.1 + summary.bytes);
                }
                emit(&shared.events, Event::Completed { batch, summary });
            }
            // A broken data stream is the sender's to notice: it reconnects and
            // resumes. Here only the control stream decides whether the link is gone,
            // so a goodbye that races a closing data stream is never missed.
            Some(_) = readers.join_next() => {}
            _ = heartbeat.tick() => {
                let msg = if complete.is_some() {
                    let p = shared.stats.snapshot();
                    Control::Progress { files: p.done_files, bytes: p.done_bytes, checked: shared.stats.checked() }
                } else {
                    Control::Ping
                };
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &msg)).await??;
            }
            _ = ending.changed() => {
                let _ = write_msg(&mut send, &Control::Bye).await;
                return Ok(Outcome::Ended);
            }
        }
    }
}

/// The batch's state: the unfinished one when the sender re-offers it after a
/// reconnect, otherwise a new one, resumed from its journal if there is one.
async fn open_batch(shared: &Shared, sender: Fingerprint, batch: u32, offer: Offer) -> Result<Arc<TransferState>> {
    let id = transfer_id(&sender, &offer);
    let mut active = shared.active.lock().await;
    if let Some(a) = active.as_mut().filter(|a| a.state.id == id) {
        a.batch = batch;
        return Ok(a.state.clone());
    }
    let files = plan_files(&offer)?;
    emit(&shared.events, Event::Incoming { batch, files: offer.files.len() as u64, bytes: offer.total_bytes() });
    let (options, stats, id) = (shared.options.clone(), shared.stats.clone(), id.clone());
    let state = tokio::task::spawn_blocking(move || {
        TransferState::open(id, files, &options.dir, &options.journal_root(), stats)
    })
    .await??;
    *active = Some(Active { batch, state: state.clone() });
    Ok(state)
}

async fn finished(complete: &mut Option<watch::Receiver<bool>>) {
    if let Some(rx) = complete {
        if rx.wait_for(|done| *done).await.is_ok() {
            return;
        }
    }
    std::future::pending().await
}

/// Forwards the sender's messages. The sender pings every half second, so
/// silence means the link is gone, even if TCP hasn't noticed yet.
async fn read_loop(mut recv: ReadHalf<TlsStream>, tx: mpsc::UnboundedSender<Result<Control>>) {
    loop {
        let msg = tokio::time::timeout(HEARTBEAT_TIMEOUT, read_msg(&mut recv))
            .await
            .unwrap_or_else(|_| Err(anyhow!("no word from the sender for {} seconds", HEARTBEAT_TIMEOUT.as_secs())));
        let failed = msg.is_err();
        if tx.send(msg).is_err() || failed {
            break;
        }
    }
}

/// Receives whole files from one data stream: each one is written as it
/// arrives, then verified on disk against the sender's hash.
async fn read_data(mut stream: TlsStream, shared: Arc<Shared>, resend: mpsc::UnboundedSender<Vec<u32>>) -> Result<()> {
    let mut header = [0u8; FILE_HEADER_LEN];
    let mut buf = vec![0u8; BLOCK];
    while read_header(&mut stream, &mut header).await? {
        let h = FileHeader::decode(&header);
        let end = h.offset.checked_add(h.len).context("file data has an impossible length")?;
        // Data from an earlier batch, or for a file already saved, is read and dropped.
        let state = shared.active.lock().await.as_ref().filter(|a| a.batch == h.batch).map(|a| a.state.clone());
        let target = match state {
            Some(state) => {
                let st = state.clone();
                let writing = tokio::task::spawn_blocking(move || st.begin(h.file, h.offset, h.len)).await??;
                writing.map(|w| (state, Arc::new(w)))
            }
            None => None,
        };
        let mut pos = h.offset;
        while pos < end {
            let n = (end - pos).min(BLOCK as u64) as usize;
            stream.read_exact(&mut buf[..n]).await?;
            if let Some((state, writing)) = &target {
                let (st, w, data) = (state.clone(), writing.clone(), buf);
                buf =
                    tokio::task::spawn_blocking(move || st.write(h.file, &w, pos, &data[..n]).map(|()| data)).await??;
            }
            pos += n as u64;
        }
        let mut hash = [0u8; HASH_LEN];
        stream.read_exact(&mut hash).await?;
        if let Some((state, _)) = target {
            if !tokio::task::spawn_blocking(move || state.finish(h.file, hash)).await?? {
                let _ = resend.send(vec![h.file]);
            }
        }
    }
    Ok(())
}

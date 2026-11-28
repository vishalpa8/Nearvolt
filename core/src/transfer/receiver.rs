//! Receiving side: accepts one sender at a time and keeps its connection while it
//! sends batch after batch. Every file is written, verified on disk and journaled,
//! so an interrupted transfer resumes where it stopped.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;

use super::state::{plan_files, sweep_stale, transfer_id, TransferState};
use super::{classify, emit, is_fatal, Event, EventSender, FailureKind, Fatal, Stats, Summary};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{self, constant_time_eq, Secret, Ticket};
use crate::protocol::{
    read_header, read_msg, write_msg, Control, FileHeader, Offer, SessionId, FILE_HEADER_LEN, HASH_LEN,
    HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION,
};
use crate::transport::{listen_tcp, Acceptor, Arrival, RawStream, TlsStream};
use crate::util::AbortOnDrop;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How many connections may be in their handshake at once. Anything beyond is
/// dropped, so a flood of junk connections can't use up the phone, and a
/// transfer in progress never depends on how many there were.
const MAX_HANDSHAKES: usize = 32;
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
    /// Tells requests apart, so an answer reaches the one the user was shown.
    pub id: u64,
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
    streams_tx: mpsc::Sender<Arrival>,
    streams_rx: tokio::sync::Mutex<mpsc::Receiver<Arrival>>,
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
    pub fn streams(&self) -> mpsc::Sender<Arrival> {
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
            deciding: Mutex::new(None),
            next_request: AtomicU64::new(1),
            active: tokio::sync::Mutex::new(None),
            totals: Mutex::new((0, 0)),
            started: Instant::now(),
        });
        let mut incoming = self.streams_rx.lock().await;
        let mut ending = self.ending.subscribe();
        let (auth_tx, mut auth_rx) = mpsc::channel::<Option<Authed>>(32);
        let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
        let ignored = Arc::new(AtomicU32::new(0));
        let mut noted = tokio::time::interval(Duration::from_secs(30));
        let mut current: Option<Current> = None;
        // When a connection that was working dropped; cleared when the sender is back.
        let mut lost_since: Option<Instant> = None;
        loop {
            let still_waiting = lost_since.map(|t| RECONNECT_WAIT.saturating_sub(t.elapsed()));
            tokio::select! {
                // The sender had its chance to come back; stop waiting and listening.
                () = tokio::time::sleep(still_waiting.unwrap_or_default()), if still_waiting.is_some() && current.is_none() => {
                    return Err(Fatal::new(
                        FailureKind::OtherPhoneGone,
                        "The other phone didn't come back after the link dropped.",
                    )
                    .into());
                }
                Some(Arrival { stream: raw, from }) = incoming.recv() => {
                    // Too many at once: drop it. Counted, and reported now and then.
                    let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                        ignored.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    let tx = auth_tx.clone();
                    let (acceptor, secrets, code) =
                        (self.acceptor.clone(), [self.secret, self.tap_secret], self.options.allow_code_pairing);
                    let ignored = ignored.clone();
                    tokio::spawn(async move {
                        let authed = authenticate(raw, from, acceptor, secrets, code).await.ok();
                        if authed.is_none() {
                            ignored.fetch_add(1, Ordering::Relaxed);
                        }
                        drop(permit);
                        let _ = tx.send(authed).await;
                    });
                }
                _ = noted.tick() => {
                    let n = ignored.swap(0, Ordering::Relaxed);
                    if n > 0 {
                        emit(&events, Event::Note { text: format!("{n} unknown connection attempts were ignored") });
                    }
                }
                Some(authed) = auth_rx.recv() => {
                    // Failed handshakes are only counted; they never end Receive.
                    let Some(mut authed) = authed else { continue };
                    match authed.session {
                        Some(session) => {
                            if let Some(c) = current.as_ref().filter(|c| c.session == session && c.sender == authed.sender) {
                                let _ = c.data_tx.try_send(authed.stream);
                            }
                        }
                        None => {
                            // Told to stop a moment ago: a connection arriving now would be dropped
                            // without a goodbye, so it is turned away with a reason instead.
                            if *self.ending.borrow() {
                                let closing = reject(FailureKind::Busy, "This phone has stopped receiving.");
                                tokio::spawn(async move { write_msg(&mut authed.stream, &closing).await });
                                continue;
                            }
                            let trusted = *shared.trusted.lock().unwrap();
                            if trusted.is_some_and(|f| f != authed.sender) {
                                let busy = reject(FailureKind::Busy, "This phone is connected to another device.");
                                tokio::spawn(async move { write_msg(&mut authed.stream, &busy).await });
                                continue;
                            }
                            // One decision at a time: while the user is deciding about one
                            // phone, a different one waits its turn. The same phone asking
                            // again, or a tap, takes over.
                            let deciding = *shared.deciding.lock().unwrap();
                            if deciding.is_some_and(|f| f != authed.sender) && !authed.consented {
                                let busy = reject(FailureKind::Busy, "Another phone is asking to connect.");
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
                            emit(&events, Event::ConnectionLost { reason: format!("{e:#}"), kind: FailureKind::OtherPhoneGone });
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
    /// The sender the user is being asked about right now, if any.
    deciding: Mutex<Option<Fingerprint>>,
    next_request: AtomicU64,
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
    /// The sender scanned this phone's QR code or tapped it, which is consent enough.
    consented: bool,
    /// Where the sender connected from, when it came over a network.
    from: Option<std::net::IpAddr>,
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
async fn authenticate(
    raw: RawStream,
    from: Option<std::net::IpAddr>,
    acceptor: Acceptor,
    secrets: [Secret; 2],
    allow_code: bool,
) -> Result<Authed> {
    let (mut stream, sender) = acceptor.handshake(raw).await?;
    let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await??;
    let Control::Hello { version, secret: offered, device_name, session, commitment } = hello else {
        bail!("expected hello");
    };
    if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&version) {
        let _ = write_msg(&mut stream, &reject(FailureKind::OldVersion, "Update Nearvolt on both devices.")).await;
        bail!("unsupported protocol version {version}");
    }
    // Scanning the QR code or tapping the phones together is the consent: the sender had to
    // see this phone's screen or touch it, so nobody is asked again.
    let consented = offered.is_some_and(|s| constant_time_eq(&s, &secrets[0]) || constant_time_eq(&s, &secrets[1]));
    let allowed = match (offered, session) {
        (Some(_), _) => consented,
        // A data stream; the caller checks its session ID and certificate.
        (None, Some(_)) => true,
        (None, None) => allow_code && commitment.is_some(),
    };
    if !allowed {
        let invalid = reject(FailureKind::Other, "This code is not valid. Scan the receiver's code again.");
        let _ = write_msg(&mut stream, &invalid).await;
        bail!("unauthorized sender");
    }
    let sender_name = pairing::clean_name(&device_name);
    let commitment = if offered.is_none() { commitment } else { None };
    Ok(Authed { stream, sender, sender_name, session, commitment, consented, from })
}

fn reject(kind: FailureKind, reason: &str) -> Control {
    Control::Reject { reason: reason.to_string(), kind: Some(kind) }
}

/// Tells the app the question it showed no longer applies, unless it was
/// answered: the asking phone left, or another took its place.
struct Question<'a> {
    shared: &'a Shared,
    id: u64,
    answered: bool,
}

impl Drop for Question<'_> {
    fn drop(&mut self) {
        *self.shared.deciding.lock().unwrap() = None;
        if !self.answered {
            emit(&self.shared.events, Event::RequestWithdrawn { id: self.id });
        }
    }
}

async fn serve(
    shared: Arc<Shared>,
    authed: Authed,
    session: SessionId,
    mut data_rx: mpsc::Receiver<TlsStream>,
    mut ending: watch::Receiver<bool>,
) -> Result<Outcome> {
    let Authed { stream, sender, sender_name, commitment, consented, from, .. } = authed;
    let (mut recv, mut send) = tokio::io::split(stream);
    let nonce = commitment.map(|_| pairing::new_secret());
    let welcome = Control::Welcome { device_name: shared.options.device_name.clone(), session, nonce };
    write_msg(&mut send, &welcome).await?;
    let code = match (commitment, nonce) {
        (Some(commitment), Some(ours)) => {
            let Control::Reveal { nonce: theirs } =
                tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut recv)).await??
            else {
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

    let accepted = consented || *shared.trusted.lock().unwrap() == Some(sender);
    if !accepted {
        let id = shared.next_request.fetch_add(1, Ordering::Relaxed);
        *shared.deciding.lock().unwrap() = Some(sender);
        let mut question = Question { shared: &shared, id, answered: false };
        let request = Request { id, sender_name: sender_name.clone(), code };
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
                    close_with(&mut send, &mut control_rx, &Control::Bye).await;
                    return Ok(Outcome::Ended);
                }
            }
        };
        question.answered = true;
        drop(question);
        if !approved {
            close_with(&mut send, &mut control_rx, &reject(FailureKind::Declined, "The receiver declined.")).await;
            return Ok(Outcome::Declined);
        }
    }
    *shared.trusted.lock().unwrap() = Some(sender);
    write_msg(&mut send, &Control::Ready).await?;
    emit(
        &shared.events,
        Event::Connected { peer: sender_name, route: from.map(|ip| ip.to_string()).unwrap_or_default() },
    );

    let (redo_tx, mut redo_rx) = mpsc::unbounded_channel();
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
                        let kind = classify(&e);
                        emit(&shared.events, Event::BatchFailed { batch, reason: format!("{e:#}"), kind });
                        write_msg(&mut send, &reject(kind, &format!("{e:#}"))).await?;
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
                            emit(&shared.events, Event::BatchFailed { batch, reason, kind: FailureKind::CannotReadFile });
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
                readers.spawn(read_data(stream, shared.clone(), redo_tx.clone()));
            }
            Some((file, offset, len)) = redo_rx.recv() => {
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &Control::Redo { file, offset, len })).await??;
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
            // A data stream that broke because of the network is the sender's to notice:
            // it reconnects and resumes. Here only the control stream decides whether the
            // link is gone, so a goodbye that races a closing data stream is never missed.
            // A stream that failed on this phone (full storage, a file it can't save, a
            // file that keeps arriving damaged) can't be fixed by reconnecting, so the
            // batch is dropped and the sender told why.
            Some(joined) = readers.join_next() => {
                if let Ok(Err(e)) = joined {
                    if is_fatal(&e) {
                        let kind = classify(&e);
                        let from = e.downcast_ref::<InBatch>().map(|b| b.0);
                        let mut active = shared.active.lock().await;
                        // Two streams can fail on the same batch; the second report must not
                        // take down the batch that came after it.
                        if active.as_ref().is_some_and(|a| from.is_none_or(|b| b == a.batch)) {
                            let Some(Active { batch, state }) = active.take() else { continue };
                            complete = None;
                            drop(active);
                            let reason = format!("{e:#}");
                            let _ = tokio::task::spawn_blocking(move || state.discard()).await;
                            emit(&shared.events, Event::BatchFailed { batch, reason, kind });
                            write_msg(&mut send, &Control::Abort { batch, kind }).await?;
                        }
                    }
                }
            }
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
                close_with(&mut send, &mut control_rx, &Control::Bye).await;
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
async fn read_data(stream: TlsStream, shared: Arc<Shared>, redo: mpsc::UnboundedSender<(u32, u64, u64)>) -> Result<()> {
    let mut batch = None;
    read_data_for(stream, shared, redo, &mut batch).await.map_err(|e| match batch {
        // Which batch it was working on, so a late error from an old batch can't end the next one.
        Some(b) => e.context(InBatch(b)),
        None => e,
    })
}

/// Ends the connection so the other phone is sure to read `last`. Dropping a
/// socket that still has unread data makes the system reset it, which can throw
/// away what was just written; so the write side is closed properly and what the
/// other phone still sends is read until it hangs up.
async fn close_with(
    send: &mut WriteHalf<TlsStream>,
    control_rx: &mut mpsc::UnboundedReceiver<Result<Control>>,
    last: &Control,
) {
    let _ = write_msg(send, last).await;
    let _ = send.shutdown().await;
    let drain = async { while let Some(Ok(_)) = control_rx.recv().await {} };
    let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
}

/// The batch a data stream was working on when it failed.
#[derive(Debug)]
struct InBatch(u32);

impl std::fmt::Display for InBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "while receiving batch {}", self.0)
    }
}

async fn read_data_for(
    mut stream: TlsStream,
    shared: Arc<Shared>,
    redo: mpsc::UnboundedSender<(u32, u64, u64)>,
    working_on: &mut Option<u32>,
) -> Result<()> {
    let mut header = [0u8; FILE_HEADER_LEN];
    let mut buf = vec![0u8; BLOCK];
    while read_header(&mut stream, &mut header).await? {
        let h = FileHeader::decode(&header);
        *working_on = Some(h.batch);
        let end = h.offset.checked_add(h.len).context("file data has an impossible length")?;
        // Data from an earlier batch, or for a file already saved, is read and dropped.
        let state = shared.active.lock().await.as_ref().filter(|a| a.batch == h.batch).map(|a| a.state.clone());
        let mut chunk = match state {
            Some(state) => tokio::task::spawn_blocking(move || state.begin(h.file, h.offset, h.len))
                .await?
                .map_err(on_this_phone)?,
            None => None,
        };
        let mut pos = h.offset;
        while pos < end {
            let n = (end - pos).min(BLOCK as u64) as usize;
            stream.read_exact(&mut buf[..n]).await?;
            if let Some(mut c) = chunk.take() {
                let data = buf;
                (chunk, buf) = tokio::task::spawn_blocking(move || {
                    let written = c.write(pos, &data[..n]);
                    written.map(|()| (Some(c), data))
                })
                .await?
                .map_err(on_this_phone)?;
            }
            pos += n as u64;
        }
        let mut hash = [0u8; HASH_LEN];
        stream.read_exact(&mut hash).await?;
        if let Some(c) = chunk {
            let (file, offset, len) = (c.index(), c.offset(), c.len());
            if !tokio::task::spawn_blocking(move || c.finish(hash)).await?.map_err(on_this_phone)? {
                let _ = redo.send((file, offset, len));
            }
        }
    }
    Ok(())
}
/// A failure on this phone, as opposed to the network: reconnecting can't fix it,
/// so it's marked as one that retrying won't help, with what went wrong.
fn on_this_phone(error: anyhow::Error) -> anyhow::Error {
    if is_fatal(&error) {
        return error;
    }
    let Some(io) = error.chain().find_map(|cause| cause.downcast_ref::<std::io::Error>()) else {
        // Not storage: a bad header or an out-of-step sender. Breaking the stream and
        // starting over is the right answer.
        return error;
    };
    let kind = match io.kind() {
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => FailureKind::DiskFull,
        _ => FailureKind::CannotSaveFile,
    };
    Fatal::new(kind, format!("{error:#}")).into()
}

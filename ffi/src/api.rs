//! What the Flutter app calls. Sizes are `i64` so Dart sees plain `int`s.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use flutter_rust_bridge::frb;
use futures::FutureExt;
use nearvolt_core::protocol::FileEntry;
use nearvolt_core::util::AbortOnDrop;
use nearvolt_core::{
    send, Approver, Event, EventSender, FileSet, Identity, IncomingOffer, Link, MuxLink, Peer, Progress, RawStream,
    ReceiveOptions, Receiver, SendOptions, SourceOpener, Stats, Summary, Ticket, WifiDirect,
};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, oneshot};

use crate::frb_generated::StreamSink;

/// The UI never needs progress more often than this.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub struct WifiInfo {
    pub ssid: String,
    pub passphrase: String,
    /// MHz.
    pub frequency: u16,
}

pub struct TicketInfo {
    pub name: String,
    /// True for QR codes and NFC taps, which skip the confirmation code.
    pub has_secret: bool,
    pub wifi: Option<WifiInfo>,
}

pub struct ReceiveTickets {
    /// For the QR code and NFC: includes the one-time secret.
    pub private_ticket: String,
    /// For the local network and Bluetooth: no secret, so senders confirm a code.
    pub public_ticket: String,
    pub port: u16,
}

/// One file the platform picked. `path` is relative, with `/` separators.
pub struct SourceFile {
    pub path: String,
    pub size: i64,
    pub modified_ms: Option<i64>,
}

#[derive(Clone, Copy)]
pub enum EventKind {
    /// Receive is open; `receive_tickets` is ready.
    Ready,
    Connected,
    /// The sender shows `code`; the receiver shows the same code in its offer.
    Code,
    /// An incoming transfer waits for `answer_offer`.
    Offer,
    Progress,
    ConnectionLost,
    Completed,
    Failed,
}

/// `text` is the peer name, the reason for a lost connection or the error,
/// depending on `kind`. Counts are filled for offers, progress and completion.
#[derive(Clone)]
pub struct TransferEvent {
    pub kind: EventKind,
    pub text: Option<String>,
    pub code: Option<String>,
    pub done_bytes: i64,
    pub total_bytes: i64,
    pub done_files: i64,
    pub total_files: i64,
}

impl TransferEvent {
    fn new(kind: EventKind, text: Option<String>) -> Self {
        Self { kind, text, code: None, done_bytes: 0, total_bytes: 0, done_files: 0, total_files: 0 }
    }

    fn with_progress(mut self, p: Progress) -> Self {
        self.done_bytes = p.done_bytes as i64;
        self.total_bytes = p.total_bytes as i64;
        self.done_files = p.done_files as i64;
        self.total_files = p.total_files as i64;
        self
    }
}

struct Engine {
    identity: Identity,
    device_name: String,
    runtime: Runtime,
    receiving: Mutex<Option<Receiving>>,
    sending: Mutex<Option<AbortOnDrop<()>>>,
    usb: MuxLink,
    usb_session: Mutex<Option<AbortOnDrop<()>>>,
}

struct Receiving {
    receiver: Arc<Receiver>,
    pending: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    _task: AbortOnDrop<()>,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn engine() -> Result<&'static Engine> {
    ENGINE.get().context("the engine has not started")
}

#[frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

/// Loads or creates this device's identity in `data_dir`. Safe to call again.
#[frb(sync)]
pub fn start_engine(data_dir: String, device_name: String) -> Result<()> {
    if ENGINE.get().is_some() {
        return Ok(());
    }
    let identity = Identity::load_or_create(&std::path::Path::new(&data_dir).join("identity.bin"))?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("nearvolt").build()?;
    let _ = ENGINE.set(Engine {
        identity,
        device_name,
        runtime,
        receiving: Mutex::new(None),
        sending: Mutex::new(None),
        usb: MuxLink::default(),
        usb_session: Mutex::new(None),
    });
    Ok(())
}

#[frb(sync)]
pub fn read_ticket(text: String) -> Result<TicketInfo> {
    let ticket = Ticket::decode(&text)?;
    Ok(TicketInfo {
        name: ticket.name,
        has_secret: ticket.secret.is_some(),
        wifi: ticket.wifi.map(|w| WifiInfo { ssid: w.ssid, passphrase: w.passphrase, frequency: w.frequency }),
    })
}

/// Opens Receive: listens for senders until `stop_receiving`. Files go to
/// `dest_dir`; journals for interrupted transfers go to `journal_dir`.
pub fn receive(dest_dir: String, journal_dir: String, events: StreamSink<TransferEvent>) -> Result<()> {
    let engine = engine()?;
    let _context = engine.runtime.enter();
    let options = ReceiveOptions {
        dir: dest_dir.into(),
        journal_dir: Some(journal_dir.into()),
        device_name: engine.device_name.clone(),
        verify_on_disk: true,
        allow_code_pairing: true,
    };
    let receiver = Arc::new(Receiver::bind(&engine.identity, SocketAddr::from(([0, 0, 0, 0], 0)), options)?);
    let pending: Arc<Mutex<Option<oneshot::Sender<bool>>>> = Arc::default();
    let approve: Approver = {
        let (pending, sink) = (pending.clone(), events.clone());
        Arc::new(move |offer: IncomingOffer| {
            let (tx, rx) = oneshot::channel();
            *pending.lock().unwrap() = Some(tx);
            let mut event = TransferEvent::new(EventKind::Offer, Some(offer.sender_name));
            event.code = offer.code;
            event.total_bytes = offer.bytes as i64;
            event.total_files = offer.files as i64;
            let _ = sink.add(event);
            async move { rx.await.unwrap_or(false) }.boxed()
        })
    };
    let running = receiver.clone();
    let ready = events.clone();
    let task = engine
        .runtime
        .spawn(drive(move |stats, events| async move { running.run(approve, stats, Some(events)).await }, events));
    *engine.receiving.lock().unwrap() = Some(Receiving { receiver, pending, _task: AbortOnDrop(task) });
    let _ = ready.add(TransferEvent::new(EventKind::Ready, None));
    Ok(())
}

/// Tickets for the open receive session. Call again once a Wi-Fi Direct group is
/// up, so the tickets carry its details and address.
#[frb(sync)]
pub fn receive_tickets(wifi: Option<WifiInfo>) -> Result<ReceiveTickets> {
    let receiving = engine()?.receiving.lock().unwrap();
    let receiver = &receiving.as_ref().context("Receive is not open")?.receiver;
    let mut ticket = receiver.ticket();
    ticket.wifi = wifi.map(|w| WifiDirect { ssid: w.ssid, passphrase: w.passphrase, frequency: w.frequency });
    Ok(ReceiveTickets {
        private_ticket: ticket.encode(),
        public_ticket: ticket.public().encode(),
        port: receiver.local_addr().port(),
    })
}

#[frb(sync)]
pub fn answer_offer(accept: bool) {
    let Ok(engine) = engine() else { return };
    if let Some(receiving) = engine.receiving.lock().unwrap().as_ref() {
        if let Some(answer) = receiving.pending.lock().unwrap().take() {
            let _ = answer.send(accept);
        }
    }
}

#[frb(sync)]
pub fn stop_receiving() {
    if let Ok(engine) = engine() {
        engine.receiving.lock().unwrap().take();
    }
}

/// Sends the picked files (`source_token` from the platform picker) to the
/// receiver in `ticket`, reconnecting and resuming on its own.
pub fn send_files(
    ticket: String,
    source_token: i64,
    files: Vec<SourceFile>,
    events: StreamSink<TransferEvent>,
) -> Result<()> {
    start_send(Peer::from_ticket(&Ticket::decode(&ticket)?), source_token, files, events)
}

/// Sends over the USB cable. With no ticket to pin, both phones confirm a code.
pub fn send_usb(source_token: i64, files: Vec<SourceFile>, events: StreamSink<TransferEvent>) -> Result<()> {
    let peer = Peer { link: engine()?.usb.clone(), name: "USB".into(), fingerprint: None, secret: None };
    start_send(peer, source_token, files, events)
}

/// A USB cable to another phone is ready: `fd` is a stream socket the engine now
/// owns. `host` is true on the phone powering the cable, so the two ends differ.
/// Streams the other phone opens go to Receive while it is open.
#[frb(sync)]
pub fn attach_usb(fd: i32, host: bool) -> Result<()> {
    let engine = engine()?;
    let _context = engine.runtime.enter();
    let session = engine.usb.attach(usb_stream(fd)?, host, |stream| {
        let receiving = ENGINE.get().and_then(|e| e.receiving.lock().unwrap().as_ref().map(|r| r.receiver.streams()));
        if let Some(streams) = receiving {
            let _ = streams.try_send(stream);
        }
    });
    *engine.usb_session.lock().unwrap() = Some(session);
    Ok(())
}

fn start_send<L: Link + 'static>(
    peer: Peer<L>,
    source_token: i64,
    files: Vec<SourceFile>,
    events: StreamSink<TransferEvent>,
) -> Result<()> {
    let engine = engine()?;
    let files = files
        .into_iter()
        .map(|f| FileEntry { path: f.path, size: f.size as u64, modified: f.modified_ms.map(|m| m as u64) })
        .collect();
    let set = FileSet::Opened { files, opener: picked_files(source_token) };
    let options = SendOptions { device_name: engine.device_name.clone(), ..Default::default() };
    let task = engine.runtime.spawn(drive(
        move |stats, events| async move { send(&engine.identity, &peer, set, &options, stats, Some(events)).await },
        events,
    ));
    *engine.sending.lock().unwrap() = Some(AbortOnDrop(task));
    Ok(())
}

#[frb(sync)]
pub fn stop_sending() {
    if let Ok(engine) = engine() {
        engine.sending.lock().unwrap().take();
    }
}

#[cfg(unix)]
fn usb_stream(fd: i32) -> Result<RawStream> {
    use std::os::fd::FromRawFd;
    // SAFETY: the platform hands over ownership of `fd`, a connected stream socket.
    let socket = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    socket.set_nonblocking(true)?;
    Ok(Box::new(tokio::net::UnixStream::from_std(socket)?))
}

#[cfg(not(unix))]
fn usb_stream(_fd: i32) -> Result<RawStream> {
    anyhow::bail!("USB links are only available on Android")
}

#[cfg(target_os = "android")]
fn picked_files(token: i64) -> Arc<dyn SourceOpener> {
    Arc::new(crate::android::PickedFiles { token })
}

#[cfg(not(target_os = "android"))]
fn picked_files(_token: i64) -> Arc<dyn SourceOpener> {
    struct Unsupported;
    impl SourceOpener for Unsupported {
        fn open(&self, _index: u32) -> std::io::Result<std::fs::File> {
            Err(std::io::Error::other("picked files are only available on Android"))
        }
    }
    Arc::new(Unsupported)
}

/// Runs a transfer and streams its events to the app, with progress throttled
/// to `PROGRESS_INTERVAL` and a final `Completed` or `Failed`.
async fn drive<W, F>(work: W, sink: StreamSink<TransferEvent>)
where
    W: FnOnce(Arc<Stats>, EventSender) -> F,
    F: Future<Output = Result<Summary>>,
{
    let stats = Arc::new(Stats::default());
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let work = work(stats.clone(), events_tx);
    tokio::pin!(work);
    let mut tick = tokio::time::interval(PROGRESS_INTERVAL);
    let mut last = None;
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            Some(event) = events_rx.recv() => forward(&sink, event),
            _ = tick.tick() => {
                let progress = stats.snapshot();
                if last != Some(progress) {
                    last = Some(progress);
                    let _ = sink.add(TransferEvent::new(EventKind::Progress, None).with_progress(progress));
                }
            }
        }
    };
    while let Ok(event) = events_rx.try_recv() {
        forward(&sink, event);
    }
    let progress = stats.snapshot();
    let _ = sink.add(match result {
        Ok(_) => TransferEvent::new(EventKind::Completed, None).with_progress(progress),
        Err(e) => TransferEvent::new(EventKind::Failed, Some(format!("{e:#}"))).with_progress(progress),
    });
}

fn forward(sink: &StreamSink<TransferEvent>, event: Event) {
    let event = match event {
        Event::Connected { peer } => TransferEvent::new(EventKind::Connected, Some(peer)),
        Event::Code { code } => {
            let mut event = TransferEvent::new(EventKind::Code, None);
            event.code = Some(code);
            event
        }
        Event::ConnectionLost { reason } => TransferEvent::new(EventKind::ConnectionLost, Some(reason)),
        Event::Resumed { .. } | Event::Completed(_) => return,
    };
    let _ = sink.add(event);
}

//! End-to-end connections over loopback: pairing by ticket, code and tap, several
//! batches on one connection, resuming in the middle of a file, a USB-style
//! multiplexed link, and disconnecting from either side.

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::FutureExt;
use nearvolt_core::{
    connect, send, Approver, Command, Event, EventSender, FileSet, Identity, Link, MuxLink, Peer, RawStream,
    ReceiveOptions, Receiver, SendOptions, Stats, Summary, TcpLink,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

const BIG: usize = 9 * 1024 * 1024 + 13;

fn sample_files(dir: &Path) {
    fs::create_dir_all(dir.join("album/nested")).unwrap();
    let big: Vec<u8> = (0..BIG).map(|i| (i % 251) as u8).collect();
    fs::write(dir.join("album/a.bin"), big).unwrap();
    fs::write(dir.join("album/nested/b.txt"), b"hello nearvolt").unwrap();
    fs::write(dir.join("album/empty.txt"), b"").unwrap();
}

fn receiver(dir: &Path, allow_code_pairing: bool) -> Receiver {
    let options = ReceiveOptions {
        dir: dir.to_path_buf(),
        journal_dir: None,
        device_name: "Receiver".into(),
        allow_code_pairing,
    };
    Receiver::bind(&Identity::generate().unwrap(), SocketAddr::from(([127, 0, 0, 1], 0)), options).unwrap()
}

/// Accepts every connection and counts how often it was asked.
fn approve_all(asked: Arc<AtomicU32>) -> Approver {
    Arc::new(move |_| {
        asked.fetch_add(1, Ordering::SeqCst);
        async { true }.boxed()
    })
}

async fn send_album<L: Link>(peer: &Peer<L>, src: &Path, events: Option<EventSender>) -> anyhow::Result<Summary> {
    let identity = Identity::generate()?;
    let files = FileSet::Paths(vec![src.join("album")]);
    send(&identity, peer, files, &SendOptions::default(), Arc::new(Stats::default()), events).await
}

fn assert_same(a: &Path, b: &Path) {
    for rel in ["a.bin", "nested/b.txt", "empty.txt"] {
        assert_eq!(fs::read(a.join(rel)).unwrap(), fs::read(b.join(rel)).unwrap(), "{rel} differs");
    }
}

fn drain(events: &mut mpsc::UnboundedReceiver<Event>) -> Vec<Event> {
    std::iter::from_fn(|| events.try_recv().ok()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn qr_ticket_connects_after_the_user_accepts() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let asked = Arc::new(AtomicU32::new(0));

    let (received, sent) = tokio::join!(
        rx.run(approve_all(asked.clone()), Arc::new(Stats::default()), None),
        send_album(&peer, &src, None)
    );
    assert_eq!(received.unwrap().files, 3);
    assert_eq!(sent.unwrap().bytes, BIG as u64 + 14);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_same(&src.join("album"), &dst.join("album"));
    assert!(!dst.join(".nearvolt").exists(), "journal should be cleaned up");
}

/// The sender pings every half second while the receiver's user decides. A slow
/// answer must neither drop the sender nor ask the user again.
#[tokio::test(flavor = "multi_thread")]
async fn a_user_who_takes_a_few_seconds_to_accept_is_asked_once() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let asked = Arc::new(AtomicU32::new(0));
    let count = asked.clone();
    let approve: Approver = Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        async {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            true
        }
        .boxed()
    });
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();

    let (received, sent) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&peer, &src, Some(events_tx)));
    received.unwrap();
    sent.unwrap();

    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(!drain(&mut events_rx).iter().any(|e| matches!(e, Event::ConnectionLost { .. })));
    assert_same(&src.join("album"), &dst.join("album"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tap_connects_without_asking() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.tap_ticket());
    let asked = Arc::new(AtomicU32::new(0));

    let (received, sent) = tokio::join!(
        rx.run(approve_all(asked.clone()), Arc::new(Stats::default()), None),
        send_album(&peer, &src, None)
    );
    received.unwrap();
    sent.unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert_same(&src.join("album"), &dst.join("album"));
}

#[tokio::test(flavor = "multi_thread")]
async fn code_pairing_shows_the_same_code_on_both_devices() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, true);
    let peer = Peer::from_ticket(&rx.ticket().public());

    let receiver_code = Arc::new(Mutex::new(None));
    let seen = receiver_code.clone();
    let approve: Approver = Arc::new(move |request| {
        *seen.lock().unwrap() = request.code.clone();
        async { true }.boxed()
    });
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (received, sent) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&peer, &src, Some(events_tx)));
    received.unwrap();
    sent.unwrap();

    let sender_code = drain(&mut events_rx).into_iter().find_map(|e| match e {
        Event::Code { code } => Some(code),
        _ => None,
    });
    assert!(sender_code.is_some());
    assert_eq!(sender_code, *receiver_code.lock().unwrap());
    assert_same(&src.join("album"), &dst.join("album"));
}

#[tokio::test(flavor = "multi_thread")]
async fn several_batches_share_one_connection_and_one_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    fs::write(src.join("second.txt"), b"queued after the album").unwrap();
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let asked = Arc::new(AtomicU32::new(0));

    let (commands, queue) = mpsc::unbounded_channel();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("album")]))).unwrap();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("second.txt")]))).unwrap();
    drop(commands);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let identity = Identity::generate().unwrap();
    let options = SendOptions::default();
    let sending = connect(&identity, &peer, &options, queue, Arc::new(Stats::default()), Some(events_tx));
    let (received, sent) = tokio::join!(rx.run(approve_all(asked.clone()), Arc::new(Stats::default()), None), sending);

    assert_eq!(received.unwrap().files, 4);
    assert_eq!(sent.unwrap().files, 4);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    let completed: Vec<u32> = drain(&mut events_rx)
        .into_iter()
        .filter_map(|e| match e {
            Event::Completed { batch, .. } => Some(batch),
            _ => None,
        })
        .collect();
    assert_eq!(completed, vec![0, 1]);
    assert_same(&src.join("album"), &dst.join("album"));
    assert_eq!(fs::read(dst.join("second.txt")).unwrap(), b"queued after the album");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cut_in_the_middle_of_a_file_resumes_instead_of_restarting() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let ticket = rx.ticket();
    let link = FlakyLink {
        inner: TcpLink::new(ticket.addrs.clone()),
        budget: Arc::new(AtomicI64::new(5 * 1024 * 1024)),
        cut: Arc::new(AtomicBool::new(false)),
        written: Arc::new(AtomicI64::new(0)),
    };
    let (budget, written) = (link.cut.clone(), link.written.clone());
    let peer = Peer { link, name: ticket.name.clone(), fingerprint: Some(ticket.fingerprint), secret: ticket.secret };

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let asked = Arc::new(AtomicU32::new(0));
    let (received, sent) = tokio::join!(
        rx.run(approve_all(asked.clone()), Arc::new(Stats::default()), None),
        send_album(&peer, &src, Some(events_tx))
    );
    received.unwrap();
    sent.unwrap();

    assert!(budget.load(Ordering::SeqCst), "the link should have been cut");
    assert!(drain(&mut events_rx).iter().any(|e| matches!(e, Event::ConnectionLost { .. })));
    assert_eq!(asked.load(Ordering::SeqCst), 1, "a reconnect is not asked about again");
    let on_the_wire = written.load(Ordering::SeqCst) as usize;
    assert!(on_the_wire < BIG + BIG / 2, "sent {on_the_wire} bytes for a {BIG}-byte file: it restarted");
    assert_same(&src.join("album"), &dst.join("album"));
}

#[tokio::test(flavor = "multi_thread")]
async fn one_cable_carries_every_stream_of_a_transfer() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, true);

    let (sender_end, receiver_end) = tokio::io::duplex(1 << 20);
    let (sender_link, receiver_link) = (MuxLink::default(), MuxLink::default());
    let streams = rx.streams();
    let _receiving = receiver_link.attach(receiver_end, false, move |stream| {
        let _ = streams.try_send(stream);
    });
    let _sending = sender_link.attach(sender_end, true, drop);
    let peer = Peer { link: sender_link, name: "Receiver".into(), fingerprint: None, secret: None };

    let approve: Approver = Arc::new(|request| async move { request.code.is_some() }.boxed());
    let (received, sent) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&peer, &src, None));
    assert_eq!(received.unwrap().files, 3);
    sent.unwrap();
    assert_same(&src.join("album"), &dst.join("album"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_disconnecting_ends_the_sender_too() {
    let tmp = tempfile::tempdir().unwrap();
    let rx = Arc::new(receiver(&tmp.path().join("dst"), false));
    let peer = Peer::from_ticket(&rx.ticket());
    let (receiver_events, mut connected) = mpsc::unbounded_channel();
    let ending = rx.clone();
    tokio::spawn(async move {
        while let Some(event) = connected.recv().await {
            if matches!(event, Event::Connected { .. }) {
                ending.disconnect();
            }
        }
    });

    let (_commands, queue) = mpsc::unbounded_channel();
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let identity = Identity::generate().unwrap();
    let options = SendOptions::default();
    let sending = connect(&identity, &peer, &options, queue, Arc::new(Stats::default()), Some(events_tx));
    let receiving = rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), Some(receiver_events));
    let (received, sent) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async { tokio::join!(receiving, sending) })
            .await
            .expect("both sides should stop");
    received.unwrap();
    sent.unwrap();
    assert!(drain(&mut events_rx).iter().any(|e| matches!(e, Event::PeerLeft)));
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_without_secret_is_refused_when_code_pairing_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket().public());

    let receiving = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), None),
    );
    let (_, sent) = tokio::join!(receiving, send_album(&peer, &src, None));
    let error = format!("{:#}", sent.unwrap_err());
    assert!(error.contains("not valid"), "unexpected error: {error}");
    assert!(!dst.join("album").exists());
}

/// TCP that breaks one stream for good once `budget` bytes have been written
/// across all streams, like Wi-Fi dropping mid-transfer. Counts every byte sent.
struct FlakyLink {
    inner: TcpLink,
    budget: Arc<AtomicI64>,
    cut: Arc<AtomicBool>,
    written: Arc<AtomicI64>,
}

impl Link for FlakyLink {
    async fn open_stream(&self) -> anyhow::Result<RawStream> {
        let inner = self.inner.open_stream().await?;
        Ok(Box::new(Flaky { inner, budget: self.budget.clone(), cut: self.cut.clone(), written: self.written.clone() }))
    }
}

struct Flaky {
    inner: RawStream,
    budget: Arc<AtomicI64>,
    cut: Arc<AtomicBool>,
    written: Arc<AtomicI64>,
}

impl AsyncRead for Flaky {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Flaky {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let left = self.budget.fetch_sub(buf.len() as i64, Ordering::SeqCst);
        if left < 0 && !self.cut.swap(true, Ordering::SeqCst) {
            return Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()));
        }
        let written = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = written {
            self.written.fetch_add(n as i64, Ordering::SeqCst);
        }
        written
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

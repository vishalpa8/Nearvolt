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
use nearvolt_core::protocol::FileEntry;
use nearvolt_core::transfer::FailureKind;
use nearvolt_core::{
    connect, send, Approver, Command, Event, EventSender, FileSet, Identity, Link, MuxLink, Peer, RawStream,
    ReceiveOptions, Receiver, SendOptions, SourceOpener, Stats, Summary, TcpLink,
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

/// Opens the first file for real and fails every other, as when a picked file
/// is deleted or its permission is withdrawn in the middle of sending.
struct HalfGone(std::path::PathBuf);

impl SourceOpener for HalfGone {
    fn open(&self, index: u32) -> std::io::Result<fs::File> {
        match index {
            0 => fs::File::open(&self.0),
            _ => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "the file is gone")),
        }
    }
}

/// A file that can't be read fails its own batch only: both phones say so, the
/// half-sent batch is thrown away, and the next batch goes through on the same
/// connection.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_file_fails_its_batch_and_the_connection_goes_on() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    fs::write(src.join("first.bin"), vec![7u8; 1 << 20]).unwrap();
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let (receiver_events, mut receiver_seen) = mpsc::unbounded_channel();

    let entry = |path: &str, size: u64| FileEntry { path: path.into(), size, modified: None };
    let broken = FileSet::Opened {
        files: vec![entry("first.bin", 1 << 20), entry("second.bin", 5 << 20)],
        opener: Arc::new(HalfGone(src.join("first.bin"))),
    };
    let (commands, queue) = mpsc::unbounded_channel();
    commands.send(Command::Send(broken)).unwrap();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("album")]))).unwrap();
    drop(commands);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let identity = Identity::generate().unwrap();
    let options = SendOptions::default();
    let sending = connect(&identity, &peer, &options, queue, Arc::new(Stats::default()), Some(events_tx));
    let receiving = rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), Some(receiver_events));
    let (received, sent) =
        tokio::time::timeout(std::time::Duration::from_secs(20), async { tokio::join!(receiving, sending) })
            .await
            .expect("the connection should finish");
    received.unwrap();
    let sent = sent.unwrap();

    assert_eq!(sent.files, 3, "only the second batch counts as sent");
    let failed = |events: Vec<Event>| -> Vec<u32> {
        events
            .into_iter()
            .filter_map(|e| match e {
                Event::BatchFailed { batch, .. } => Some(batch),
                _ => None,
            })
            .collect()
    };
    assert_eq!(failed(drain(&mut events_rx)), vec![0], "the sender reports the broken batch");
    assert_eq!(failed(drain(&mut receiver_seen)), vec![0], "so does the receiver");
    assert_same(&src.join("album"), &dst.join("album"));
    let leftovers: Vec<String> = files_under(&dst).into_iter().filter(|p| p.ends_with(".nvpart")).collect();
    assert_eq!(leftovers, Vec::<String>::new(), "no half file is left");
    assert!(!dst.join("second.bin").exists());
}

/// A selection bigger than one batch is sent as several, one after another, and
/// every file arrives once.
#[tokio::test(flavor = "multi_thread")]
async fn a_big_selection_is_sent_as_several_batches() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let (commands, queue) = mpsc::unbounded_channel();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("album")]))).unwrap();
    drop(commands);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let identity = Identity::generate().unwrap();
    // Two files to a batch: the three files of the album take two batches.
    let options = SendOptions { max_files_per_batch: 2, ..SendOptions::default() };
    let sending = connect(&identity, &peer, &options, queue, Arc::new(Stats::default()), Some(events_tx));
    let receiving = rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), None);
    let (received, sent) = tokio::join!(receiving, sending);
    assert_eq!(received.unwrap().files, 3);
    assert_eq!(sent.unwrap().files, 3);

    let completed: Vec<u32> = drain(&mut events_rx)
        .into_iter()
        .filter_map(|e| match e {
            Event::Completed { batch, .. } => Some(batch),
            _ => None,
        })
        .collect();
    assert_eq!(completed, vec![0, 1]);
    assert_same(&src.join("album"), &dst.join("album"));
}

/// A Wi-Fi Direct address that was joined but doesn't answer must not leave the
/// sender stuck: the other addresses in the code are tried.
#[tokio::test(flavor = "multi_thread")]
async fn a_direct_address_that_does_not_answer_falls_back_to_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let ticket = rx.ticket();
    // A port nothing listens on.
    let dead = {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        socket.local_addr().unwrap()
    };
    let peer = Peer {
        link: TcpLink::preferring(dead, ticket.addrs.clone()),
        name: ticket.name.clone(),
        fingerprint: Some(ticket.fingerprint),
        secret: ticket.secret,
    };
    let (received, sent) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::join!(
            rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), None),
            send_album(&peer, &src, None)
        )
    })
    .await
    .expect("it should connect through the other addresses");
    received.unwrap();
    sent.unwrap();
    assert_same(&src.join("album"), &dst.join("album"));
}

/// A file the receiving phone can't save fails its own batch and says why; the
/// sender doesn't retry forever and the connection goes on.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_the_receiver_cannot_save_fails_only_its_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    fs::write(src.join("second.txt"), b"after the failure").unwrap();
    // A file where the album's folder should go, so nothing in it can be saved.
    fs::create_dir_all(&dst).unwrap();
    fs::write(dst.join("album"), b"in the way").unwrap();
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let (receiver_events, mut receiver_seen) = mpsc::unbounded_channel();

    let (commands, queue) = mpsc::unbounded_channel();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("album")]))).unwrap();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("second.txt")]))).unwrap();
    drop(commands);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let identity = Identity::generate().unwrap();
    let options = SendOptions::default();
    let sending = connect(&identity, &peer, &options, queue, Arc::new(Stats::default()), Some(events_tx));
    let receiving = rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), Some(receiver_events));
    let (received, sent) =
        tokio::time::timeout(std::time::Duration::from_secs(20), async { tokio::join!(receiving, sending) })
            .await
            .expect("the connection should finish, not retry for ever");
    received.unwrap();
    assert_eq!(sent.unwrap().files, 1, "only the second batch was sent");

    let failures = |events: Vec<Event>| -> Vec<(u32, FailureKind)> {
        events
            .into_iter()
            .filter_map(|e| match e {
                Event::BatchFailed { batch, kind, .. } => Some((batch, kind)),
                _ => None,
            })
            .collect()
    };
    assert_eq!(failures(drain(&mut events_rx)), vec![(0, FailureKind::CannotSaveFile)]);
    assert_eq!(failures(drain(&mut receiver_seen)), vec![(0, FailureKind::CannotSaveFile)]);
    assert_eq!(fs::read(dst.join("second.txt")).unwrap(), b"after the failure");
}

/// While the user is deciding about one phone, another one waits its turn instead
/// of taking the question over; and the question is withdrawn if its phone leaves.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_phone_is_turned_away_while_one_is_being_decided() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, true);
    let first = Peer::from_ticket(&rx.ticket().public());
    let second = Peer::from_ticket(&rx.ticket().public());
    let asked = Arc::new(AtomicU32::new(0));
    let count = asked.clone();
    let approve: Approver = Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        async {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            true
        }
        .boxed()
    });

    let late = async {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        send_album(&second, &src, None).await
    };
    let (received, sent, turned_away) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&first, &src, None), late);
    received.unwrap();
    sent.unwrap();
    let error = turned_away.expect_err("the second phone must not get in");
    assert_eq!(nearvolt_core::transfer::classify(&error), FailureKind::Busy);
    assert_eq!(asked.load(Ordering::SeqCst), 1, "the user was asked about one phone only");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_is_withdrawn_when_the_phone_asking_leaves() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = Arc::new(receiver(&dst, true));
    let peer = Peer::from_ticket(&rx.ticket().public());
    let (receiver_events, mut seen) = mpsc::unbounded_channel();
    // Nobody ever answers.
    let approve: Approver = Arc::new(|_| futures::future::pending().boxed());
    let running = rx.clone();
    let receiving =
        tokio::spawn(async move { running.run(approve, Arc::new(Stats::default()), Some(receiver_events)).await });

    let sending = tokio::spawn(async move { send_album(&peer, &tmp.path().join("src"), None).await });
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    sending.abort();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    rx.disconnect();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), receiving).await;

    assert!(drain(&mut seen).iter().any(|e| matches!(e, Event::RequestWithdrawn { .. })));
}

/// Connections that never get past the handshake are ignored, however many; they
/// can't end Receive, and a real sender still gets through afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_flood_of_junk_connections_does_not_end_receive() {
    use tokio::io::AsyncWriteExt;
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());
    let addr = rx.local_addr();

    let junk = async {
        for _ in 0..80 {
            if let Ok(mut stream) = tokio::net::TcpStream::connect(addr).await {
                let _ = stream.write_all(b"GET / HTTP/1.1\r\n\r\nthis is not a handshake").await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        send_album(&peer, &src, None).await
    };
    let (received, sent) = tokio::join!(rx.run(approve_all(Arc::default()), Arc::new(Stats::default()), None), junk);
    received.expect("Receive must survive the junk");
    sent.unwrap();
    assert_same(&src.join("album"), &dst.join("album"));
}

fn files_under(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            out.push(path.to_string_lossy().into_owned());
            if path.is_dir() {
                walk(&path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}

/// Ending the connection on purpose in the middle of a file leaves nothing
/// behind: no half file, no hidden part file, no journal, no empty folder.
#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_in_the_middle_of_a_file_removes_the_unfinished_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst, journals) = (tmp.path().join("src"), tmp.path().join("dst"), tmp.path().join("journals"));
    fs::create_dir_all(src.join("trip")).unwrap();
    // A sparse 400 MB file: big enough to still be arriving when we disconnect.
    fs::File::create(src.join("trip/video.bin")).unwrap().set_len(400 * 1024 * 1024).unwrap();
    let options = ReceiveOptions {
        dir: dst.clone(),
        journal_dir: Some(journals.clone()),
        device_name: "Receiver".into(),
        allow_code_pairing: false,
    };
    let rx = Receiver::bind(&Identity::generate().unwrap(), SocketAddr::from(([127, 0, 0, 1], 0)), options).unwrap();
    let peer = Peer::from_ticket(&rx.ticket());
    let received_stats = Arc::new(Stats::default());

    let (commands, queue) = mpsc::unbounded_channel();
    commands.send(Command::Send(FileSet::Paths(vec![src.join("trip")]))).unwrap();
    let watch = received_stats.clone();
    let stopper = tokio::spawn(async move {
        // Disconnect once part of the file is on the receiver.
        while watch.snapshot().done_bytes < 8 * 1024 * 1024 {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        commands.send(Command::Disconnect).unwrap();
        commands
    });
    let identity = Identity::generate().unwrap();
    let options = SendOptions::default();
    let sending = connect(&identity, &peer, &options, queue, Arc::new(Stats::default()), None);
    let receiving = rx.run(approve_all(Arc::default()), received_stats.clone(), None);
    let (received, sent) =
        tokio::time::timeout(std::time::Duration::from_secs(20), async { tokio::join!(receiving, sending) })
            .await
            .expect("both sides should stop");
    received.unwrap();
    sent.unwrap();
    let _ = stopper.await;

    assert!(received_stats.snapshot().done_bytes < 400 * 1024 * 1024, "it should have been cut off");
    assert_eq!(files_under(&dst), Vec::<String>::new(), "the receiver kept something");
    assert_eq!(files_under(&journals), Vec::<String>::new(), "the receiver kept a journal");
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

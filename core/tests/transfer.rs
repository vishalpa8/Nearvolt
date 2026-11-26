//! End-to-end transfers over loopback: the ticket secret path, the code pairing
//! path, rejection of a sender without the secret, and one byte stream standing
//! in for a USB cable.

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use nearvolt_core::{
    send, Approver, Event, EventSender, FileSet, Identity, Link, MuxLink, Peer, ReceiveOptions, Receiver, SendOptions,
    Stats, Summary,
};
use tokio::sync::mpsc;

fn sample_files(dir: &Path) {
    fs::create_dir_all(dir.join("album/nested")).unwrap();
    fs::write(dir.join("album/a.bin"), vec![7u8; 9 * 1024 * 1024 + 13]).unwrap();
    fs::write(dir.join("album/nested/b.txt"), b"hello nearvolt").unwrap();
    fs::write(dir.join("album/empty.txt"), b"").unwrap();
}

fn receiver(dir: &Path, allow_code_pairing: bool) -> Receiver {
    let options = ReceiveOptions {
        dir: dir.to_path_buf(),
        journal_dir: None,
        device_name: "Receiver".into(),
        verify_on_disk: true,
        allow_code_pairing,
    };
    Receiver::bind(&Identity::generate().unwrap(), SocketAddr::from(([127, 0, 0, 1], 0)), options).unwrap()
}

async fn send_album<L: Link>(peer: &Peer<L>, src: &Path, events: Option<EventSender>) -> anyhow::Result<Summary> {
    let identity = Identity::generate()?;
    let options = SendOptions::default();
    let files = FileSet::Paths(vec![src.join("album")]);
    send(&identity, peer, files, &options, Arc::new(Stats::default()), events).await
}

fn assert_same(a: &Path, b: &Path) {
    for rel in ["a.bin", "nested/b.txt", "empty.txt"] {
        assert_eq!(fs::read(a.join(rel)).unwrap(), fs::read(b.join(rel)).unwrap(), "{rel} differs");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ticket_with_secret_transfers_every_byte() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket());

    let approve: Approver = Arc::new(|offer| async move { offer.code.is_none() }.boxed());
    let (received, sent) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&peer, &src, None));
    assert_eq!(received.unwrap().files, 3);
    assert_eq!(sent.unwrap().bytes, 9 * 1024 * 1024 + 13 + 14);
    assert_same(&src.join("album"), &dst.join("album"));
    assert!(!dst.join(".nearvolt").exists(), "journal should be cleaned up");
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
    let approve: Approver = Arc::new(move |offer| {
        *seen.lock().unwrap() = offer.code.clone();
        async { true }.boxed()
    });
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (received, sent) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&peer, &src, Some(events_tx)));
    received.unwrap();
    sent.unwrap();

    let mut sender_code = None;
    while let Ok(event) = events_rx.try_recv() {
        if let Event::Code { code } = event {
            sender_code = Some(code);
        }
    }
    let receiver_code = receiver_code.lock().unwrap().clone();
    assert!(sender_code.is_some());
    assert_eq!(sender_code, receiver_code);
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

    let approve: Approver = Arc::new(|offer| async move { offer.code.is_some() }.boxed());
    let (received, sent) =
        tokio::join!(rx.run(approve, Arc::new(Stats::default()), None), send_album(&peer, &src, None));
    assert_eq!(received.unwrap().files, 3);
    sent.unwrap();
    assert_same(&src.join("album"), &dst.join("album"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_without_secret_is_refused_when_code_pairing_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    sample_files(&src);
    let rx = receiver(&dst, false);
    let peer = Peer::from_ticket(&rx.ticket().public());

    let approve: Approver = Arc::new(|_| async { true }.boxed());
    let receiving =
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.run(approve, Arc::new(Stats::default()), None));
    let (_, sent) = tokio::join!(receiving, send_album(&peer, &src, None));
    let error = format!("{:#}", sent.unwrap_err());
    assert!(error.contains("not valid"), "unexpected error: {error}");
    assert!(!dst.join("album").exists());
}

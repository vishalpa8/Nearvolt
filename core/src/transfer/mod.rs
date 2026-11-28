mod disk;
mod journal;
pub mod manifest;
mod ranges;
pub mod receiver;
pub mod sender;
mod state;

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// What went wrong, in terms an app can turn into a plain sentence. The text of
/// an error is for the log; this is for the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The receiving phone has no room for the files.
    DiskFull,
    /// The receiving phone couldn't save a file.
    CannotSaveFile,
    /// A file couldn't be read on the sending phone, or changed while it was sent.
    CannotReadFile,
    /// A file kept arriving damaged.
    FileDamaged,
    /// The receiver's user said no.
    Declined,
    /// The receiver is connected to another phone.
    Busy,
    /// The other phone stopped answering and didn't come back.
    OtherPhoneGone,
    /// The first connection couldn't be made.
    CannotReach,
    /// The two phones run versions of Nearvolt that can't talk to each other.
    OldVersion,
    /// Too many files for one offer, or some other limit.
    TooBig,
    /// The user cancelled the batch.
    Cancelled,
    Other,
}

/// An error that retrying won't fix, such as the receiver declining.
#[derive(Debug)]
pub struct Fatal(pub String, pub FailureKind);

impl Fatal {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self(message.into(), kind)
    }
}

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fatal {}

pub fn is_fatal(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Fatal>().is_some()
}

/// The kind of failure behind `error`: the one a [`Fatal`] carries, or one read
/// from the underlying I/O error.
pub fn classify(error: &anyhow::Error) -> FailureKind {
    for cause in error.chain() {
        if let Some(fatal) = cause.downcast_ref::<Fatal>() {
            return fatal.1;
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return match io.kind() {
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => FailureKind::DiskFull,
                std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::NotFound
                | std::io::ErrorKind::AlreadyExists
                | std::io::ErrorKind::InvalidInput
                | std::io::ErrorKind::InvalidData
                | std::io::ErrorKind::WriteZero => FailureKind::CannotSaveFile,
                _ => FailureKind::Other,
            };
        }
    }
    FailureKind::Other
}

/// Live counters a UI can poll as often as it likes.
#[derive(Default, Debug)]
pub struct Stats {
    total_bytes: AtomicU64,
    done_bytes: AtomicU64,
    total_files: AtomicU64,
    done_files: AtomicU64,
    /// Bytes read back from disk only to hash them. Grows while a large file is
    /// checked, when the other counters stand still.
    checked: AtomicU64,
    /// The protocol version of the phone at the other end; 0 until known.
    peer_version: std::sync::atomic::AtomicU32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub total_files: u64,
    pub done_files: u64,
}

impl Stats {
    pub fn snapshot(&self) -> Progress {
        Progress {
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            done_bytes: self.done_bytes.load(Ordering::Relaxed),
            total_files: self.total_files.load(Ordering::Relaxed),
            done_files: self.done_files.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn set_peer_version(&self, version: u32) {
        self.peer_version.store(version, Ordering::Relaxed);
    }

    /// The other phone's protocol version, once connected.
    pub fn peer_version(&self) -> u32 {
        self.peer_version.load(Ordering::Relaxed)
    }

    pub(crate) fn set_totals(&self, files: u64, bytes: u64) {
        self.total_files.store(files, Ordering::Relaxed);
        self.total_bytes.store(bytes, Ordering::Relaxed);
    }

    pub(crate) fn set_done(&self, files: u64, bytes: u64) {
        self.done_files.store(files, Ordering::Relaxed);
        self.done_bytes.store(bytes, Ordering::Relaxed);
    }

    pub(crate) fn add_bytes(&self, n: u64) {
        self.done_bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn sub_bytes(&self, n: u64) {
        let mut current = self.done_bytes.load(Ordering::Relaxed);
        while let Err(actual) = self.done_bytes.compare_exchange_weak(
            current,
            current.saturating_sub(n),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            current = actual;
        }
    }

    pub(crate) fn add_file(&self) {
        self.done_files.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn checked(&self) -> u64 {
        self.checked.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub files: u64,
    pub bytes: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Connected and accepted, including after every reconnect. `route` is the
    /// address or cable the sender reached the receiver on; the receiver
    /// leaves it empty.
    Connected {
        peer: String,
        route: String,
    },
    /// Show this code so the user can check the other screen shows the same one.
    Code {
        code: String,
    },
    /// Sender: the receiver has seen us and its user is deciding. No code to compare
    /// (the code came from a scan), so the screen just says it is waiting.
    Waiting,
    /// Sender: the receiver took a batch and it is being sent.
    Started {
        batch: u32,
        files: u64,
        bytes: u64,
    },
    /// Receiver: a batch started arriving.
    Incoming {
        batch: u32,
        files: u64,
        bytes: u64,
    },
    Completed {
        batch: u32,
        summary: Summary,
    },
    /// One batch couldn't be sent or received, for example for lack of space.
    /// The connection stays.
    BatchFailed {
        batch: u32,
        reason: String,
        kind: FailureKind,
    },
    /// The link dropped and is being reconnected; once per drop, and never
    /// before the first `Connected`.
    ConnectionLost {
        reason: String,
        kind: FailureKind,
    },
    /// The other device ended the connection.
    PeerLeft,
    /// Receiver: the phone whose request `id` was shown left before an answer,
    /// so the question no longer applies.
    RequestWithdrawn {
        id: u64,
    },
    /// Something worth a line in the log that isn't an error.
    Note {
        text: String,
    },
    /// The files of a batch that is arriving, as (path, size), so the app can list them.
    /// Cut off after LISTED_FILES; the sender already knows its own.
    Listing {
        batch: u32,
        files: Vec<(String, u64)>,
    },
    /// How the files of atch are doing: (file, bytes so far) for those moving now,
    /// and the ones that just finished or were skipped. Numbers are positions in the
    /// batch's file list.
    Files {
        batch: u32,
        active: Vec<(u32, u64)>,
        finished: Vec<u32>,
        skipped: Vec<u32>,
    },
}

/// Most files listed to the receiving app for one batch.
pub const LISTED_FILES: usize = 5_000;

/// Byte counts for the individual files being moved, by (batch, file), and the
/// ones that just finished or were skipped. Both phones keep one, to show a
/// file list; it is not on the wire.
#[derive(Default)]
pub(crate) struct FileMeter {
    inner: std::sync::Mutex<MeterInner>,
}

#[derive(Default)]
struct MeterInner {
    bytes: std::collections::BTreeMap<(u32, u32), u64>,
    /// Chunks of a file still to go, so its last one says it is finished.
    chunks_left: std::collections::HashMap<(u32, u32), u32>,
    finished: Vec<(u32, u32)>,
    skipped: std::collections::HashSet<(u32, u32)>,
    news_skipped: Vec<(u32, u32)>,
}

/// What a report says about one batch: files moving, finished, skipped.
type PerBatch = (Vec<(u32, u64)>, Vec<u32>, Vec<u32>);

impl FileMeter {
    pub(crate) fn expect(&self, batch: u32, file: u32, chunks: u32) {
        *self.inner.lock().unwrap().chunks_left.entry((batch, file)).or_default() += chunks;
    }

    pub(crate) fn add(&self, batch: u32, file: u32, n: u64) {
        *self.inner.lock().unwrap().bytes.entry((batch, file)).or_default() += n;
    }

    /// Makes sure a file shows at least `n` bytes, for what it already had.
    pub(crate) fn set_at_least(&self, batch: u32, file: u32, n: u64) {
        if n > 0 {
            let mut inner = self.inner.lock().unwrap();
            let b = inner.bytes.entry((batch, file)).or_default();
            *b = (*b).max(n);
        }
    }

    pub(crate) fn sub(&self, batch: u32, file: u32, n: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(b) = inner.bytes.get_mut(&(batch, file)) {
            *b = b.saturating_sub(n);
        }
    }

    /// One chunk is through. The last one of a file finishes it.
    pub(crate) fn chunk_done(&self, batch: u32, file: u32) {
        let mut inner = self.inner.lock().unwrap();
        let key = (batch, file);
        if let Some(left) = inner.chunks_left.get_mut(&key) {
            *left = left.saturating_sub(1);
            if *left == 0 {
                inner.chunks_left.remove(&key);
                inner.bytes.remove(&key);
                if !inner.skipped.contains(&key) {
                    inner.finished.push(key);
                }
            }
        }
    }

    /// Done without chunks coming through, for example because it was already there.
    pub(crate) fn finish(&self, batch: u32, file: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.bytes.remove(&(batch, file));
        inner.chunks_left.remove(&(batch, file));
        inner.finished.push((batch, file));
    }

    /// Returns true the first time a file is skipped.
    pub(crate) fn skip(&self, batch: u32, file: u32) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let key = (batch, file);
        inner.bytes.remove(&key);
        inner.chunks_left.remove(&key);
        let fresh = inner.skipped.insert(key);
        if fresh {
            inner.news_skipped.push(key);
        }
        fresh
    }

    pub(crate) fn is_skipped(&self, batch: u32, file: u32) -> bool {
        self.inner.lock().unwrap().skipped.contains(&(batch, file))
    }

    /// What changed since the last call, one event per batch.
    pub(crate) fn report(&self) -> Vec<Event> {
        let mut inner = self.inner.lock().unwrap();
        let mut by_batch: std::collections::BTreeMap<u32, PerBatch> = Default::default();
        for (&(batch, file), &bytes) in &inner.bytes {
            by_batch.entry(batch).or_default().0.push((file, bytes));
        }
        for (batch, file) in std::mem::take(&mut inner.finished) {
            by_batch.entry(batch).or_default().1.push(file);
        }
        for (batch, file) in std::mem::take(&mut inner.news_skipped) {
            by_batch.entry(batch).or_default().2.push(file);
        }
        by_batch
            .into_iter()
            .map(|(batch, (active, finished, skipped))| Event::Files { batch, active, finished, skipped })
            .collect()
    }

    /// Starts a batch's byte counts over (a reconnect resumes from what the receiver has),
    /// keeping which files were skipped.
    pub(crate) fn forget_counts(&self, batch: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.bytes.retain(|k, _| k.0 != batch);
        inner.chunks_left.retain(|k, _| k.0 != batch);
    }

    /// A batch is over: forget its files.
    pub(crate) fn forget(&self, batch: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.bytes.retain(|k, _| k.0 != batch);
        inner.chunks_left.retain(|k, _| k.0 != batch);
        inner.skipped.retain(|k| k.0 != batch);
    }
}
pub type EventSender = tokio::sync::mpsc::UnboundedSender<Event>;

pub(crate) fn emit(events: &Option<EventSender>, event: Event) {
    if let Some(events) = events {
        let _ = events.send(event);
    }
}

/// Decimal units, for example `1.25 GB`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

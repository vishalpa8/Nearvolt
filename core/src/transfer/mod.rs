mod disk;
mod journal;
pub mod manifest;
pub mod receiver;
pub mod sender;
mod state;

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// An error that retrying won't fix, such as the receiver declining.
#[derive(Debug)]
pub struct Fatal(pub String);

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fatal {}

pub fn is_fatal(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Fatal>().is_some()
}

/// Live counters a UI can poll as often as it likes.
#[derive(Default, Debug)]
pub struct Stats {
    total_bytes: AtomicU64,
    done_bytes: AtomicU64,
    total_files: AtomicU64,
    done_files: AtomicU64,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub files: u64,
    pub bytes: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected { peer: String },
    ConnectionLost { reason: String },
    Resumed { done_bytes: u64 },
    Completed(Summary),
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

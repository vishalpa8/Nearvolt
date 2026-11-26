//! Everything the receiver knows about one incoming transfer: which chunks have
//! arrived, where they are on disk, and the journal that survives crashes.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};
use tokio::sync::watch;

use super::disk;
use super::journal::{Journal, Replay};
use super::{human_bytes, Fatal, Stats, Summary};
use crate::identity::Fingerprint;
use crate::protocol::{
    bit_clear, bit_get, bit_set, bitmap_len, chunk_count, chunk_range, ChunkHeader, ChunkRef, FileHave, Offer,
    MAX_CHUNK_SIZE, MAX_FILES, MIN_CHUNK_SIZE,
};
use crate::safety;

const CHECKPOINT_EVERY: Duration = Duration::from_secs(2);
const PART_SUFFIX: &str = ".nvpart";

/// The receiver derives the ID itself, so a re-offer of the same files from the
/// same sender always maps to the same journal.
pub(super) fn transfer_id(sender: &Fingerprint, offer: &Offer) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(sender);
    hasher.update(&serde_json::to_vec(offer).expect("offers always serialize"));
    hasher.finalize().to_hex().to_string()
}

pub(super) struct RecvFile {
    rel: PathBuf,
    size: u64,
    chunks: u32,
    modified: Option<u64>,
}

pub(super) fn plan_files(offer: &Offer) -> Result<Vec<RecvFile>> {
    ensure!(
        (MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&offer.chunk_size),
        "unsupported chunk size {}",
        offer.chunk_size
    );
    ensure!(!offer.files.is_empty() && offer.files.len() <= MAX_FILES, "invalid number of files");
    let mut cleaned = Vec::with_capacity(offer.files.len());
    for f in &offer.files {
        ensure!(f.size / (offer.chunk_size as u64) < u32::MAX as u64, "{} is too large", f.path);
        cleaned.push(safety::clean_relative_path(&f.path)?);
    }
    let paths = safety::unique_paths(cleaned)?;
    Ok(offer
        .files
        .iter()
        .zip(paths)
        .map(|(f, rel)| RecvFile {
            rel,
            size: f.size,
            chunks: chunk_count(f.size, offer.chunk_size),
            modified: f.modified,
        })
        .collect())
}

struct Slot {
    have: Vec<u8>,
    hashes: Vec<[u8; 32]>,
    remaining: u32,
    handle: Option<Arc<File>>,
    done: bool,
}

/// Shared by every connection the sender makes, so reconnects pick up exactly
/// where the last one stopped.
pub(super) struct TransferState {
    pub(super) id: String,
    root: PathBuf,
    state_dir: PathBuf,
    chunk_size: u32,
    verify_on_disk: bool,
    files: Vec<RecvFile>,
    slots: Vec<Mutex<Slot>>,
    journal: Mutex<Journal>,
    /// Part files written since the last checkpoint.
    dirty: Mutex<HashMap<u32, Arc<File>>>,
    remaining_files: AtomicU64,
    pub(super) complete: watch::Sender<bool>,
    stats: Arc<Stats>,
    started: Instant,
}

impl TransferState {
    /// `root` receives the files; the journal lives in `journal_root/<transfer id>`.
    pub(super) fn open(
        id: String,
        chunk_size: u32,
        files: Vec<RecvFile>,
        root: &Path,
        journal_root: &Path,
        verify_on_disk: bool,
        stats: Arc<Stats>,
    ) -> Result<Arc<Self>> {
        let state_dir = journal_root.join(&id[..32]);
        fs::create_dir_all(&state_dir)?;
        let (journal, replay) = Journal::open(&state_dir.join("journal"))?;
        let slots = files
            .iter()
            .map(|f| {
                Mutex::new(Slot {
                    have: vec![0; bitmap_len(f.chunks)],
                    hashes: vec![[0; 32]; f.chunks as usize],
                    remaining: f.chunks,
                    handle: None,
                    done: false,
                })
            })
            .collect();
        let file_count = files.len() as u64;
        let state = Self {
            id,
            root: root.to_path_buf(),
            state_dir,
            chunk_size,
            verify_on_disk,
            files,
            slots,
            journal: Mutex::new(journal),
            dirty: Mutex::new(HashMap::new()),
            remaining_files: AtomicU64::new(file_count),
            complete: watch::channel(false).0,
            stats,
            started: Instant::now(),
        };
        let with_parts = state.replay(replay);
        state.check_space(&with_parts)?;
        for i in 0..state.files.len() as u32 {
            let ready = {
                let slot = state.slot(i);
                !slot.done && slot.remaining == 0
            };
            if ready {
                state.finish_file(i)?;
            }
        }
        state.publish_progress();
        let state = Arc::new(state);
        spawn_checkpointer(Arc::downgrade(&state));
        Ok(state)
    }

    /// Applies the journal. Returns the files that already have a part file.
    fn replay(&self, replay: Replay) -> HashSet<u32> {
        let count = self.files.len();
        let mut finished = HashSet::new();
        for (i, name) in replay.done {
            if i as usize >= count {
                continue;
            }
            finished.insert(i);
            let f = &self.files[i as usize];
            let on_disk = safety::clean_relative_path(&name)
                .ok()
                .map(|parts| self.root.join(parts.iter().collect::<PathBuf>()))
                .and_then(|path| fs::metadata(path).ok())
                .is_some_and(|meta| meta.len() == f.size);
            if on_disk {
                self.mark_done(i);
            }
        }

        let mut with_parts = HashSet::new();
        let mut part_exists = HashMap::new();
        let mut unverified: HashMap<u32, Vec<(u32, [u8; 32])>> = HashMap::new();
        let mut without_part: HashMap<u32, HashMap<u32, [u8; 32]>> = HashMap::new();
        for rec in replay.chunks {
            let i = rec.file;
            if i as usize >= count || rec.chunk >= self.files[i as usize].chunks || finished.contains(&i) {
                continue;
            }
            if !*part_exists.entry(i).or_insert_with(|| self.part_path(i).exists()) {
                without_part.entry(i).or_default().insert(rec.chunk, rec.hash);
                continue;
            }
            with_parts.insert(i);
            if rec.trusted {
                self.mark(i, rec.chunk, rec.hash);
            } else {
                unverified.entry(i).or_default().push((rec.chunk, rec.hash));
            }
        }

        // Chunks after the last checkpoint may not have reached the disk before a
        // crash, so only count the ones whose bytes still match.
        for (i, chunks) in unverified {
            let Ok(file) = File::open(self.part_path(i)) else { continue };
            let size = self.files[i as usize].size;
            let mut buf = Vec::new();
            for (c, hash) in chunks {
                let (offset, len) = chunk_range(size, self.chunk_size, c);
                buf.resize(len as usize, 0);
                if disk::read_exact_at(&file, &mut buf, offset).is_ok()
                    && blake3::hash(&buf) == blake3::Hash::from(hash)
                {
                    self.mark(i, c, hash);
                }
            }
        }

        // A file whose done record was lost in a crash is already under its final
        // name. Recognize it by its chunk hashes instead of receiving it again.
        for (i, chunks) in without_part {
            let f = &self.files[i as usize];
            if chunks.len() != f.chunks as usize {
                continue;
            }
            let Ok(file) = File::open(self.root.join(&f.rel)) else { continue };
            if file.metadata().map(|m| m.len()).ok() != Some(f.size) {
                continue;
            }
            let hashes: Vec<[u8; 32]> = (0..f.chunks).map(|c| chunks[&c]).collect();
            if matches!(verify_part(&file, f.size, self.chunk_size, &hashes), Ok(bad) if bad.is_empty()) {
                self.mark_done(i);
                let _ = self.journal.lock().unwrap().file_done(i, &rel_string(&f.rel));
            }
        }
        with_parts
    }

    fn mark_done(&self, i: u32) {
        let chunks = self.files[i as usize].chunks;
        let mut slot = self.slot(i);
        slot.done = true;
        slot.remaining = 0;
        slot.hashes = Vec::new();
        for c in 0..chunks {
            bit_set(&mut slot.have, c);
        }
        drop(slot);
        self.remaining_files.fetch_sub(1, Ordering::Relaxed);
    }

    fn check_space(&self, with_parts: &HashSet<u32>) -> Result<()> {
        let needed = (0..self.files.len() as u32)
            .filter(|i| !with_parts.contains(i) && !self.slot(*i).done)
            .fold(0u64, |sum, i| sum.saturating_add(self.files[i as usize].size));
        let free = disk::available_space(&self.root).unwrap_or(u64::MAX);
        if needed > free {
            return Err(Fatal(format!(
                "Not enough free space: this transfer needs {}, but only {} is free.",
                human_bytes(needed),
                human_bytes(free)
            ))
            .into());
        }
        Ok(())
    }

    pub(super) fn expected_len(&self, h: &ChunkHeader) -> Result<u32> {
        let f = self.files.get(h.file as usize).context("chunk for an unknown file")?;
        ensure!(h.chunk < f.chunks, "chunk index out of range");
        let (_, len) = chunk_range(f.size, self.chunk_size, h.chunk);
        ensure!(h.len == len, "chunk has the wrong length");
        Ok(len)
    }

    /// Verifies and writes one chunk. Returns chunks that must be sent again.
    pub(super) fn store_chunk(&self, h: ChunkHeader, data: &[u8]) -> Result<Vec<ChunkRef>> {
        ensure!(blake3::hash(data) == blake3::Hash::from(h.hash), "a chunk failed verification");
        let file = {
            let mut slot = self.slot(h.file);
            if slot.done || bit_get(&slot.have, h.chunk) {
                return Ok(Vec::new());
            }
            self.handle(h.file, &mut slot)?
        };
        let (offset, _) = chunk_range(self.files[h.file as usize].size, self.chunk_size, h.chunk);
        disk::write_all_at(&file, data, offset)?;
        self.dirty.lock().unwrap().insert(h.file, file);
        self.journal.lock().unwrap().chunk(h.file, h.chunk, &h.hash)?;
        let finished = {
            let mut slot = self.slot(h.file);
            if slot.done || bit_get(&slot.have, h.chunk) {
                return Ok(Vec::new());
            }
            bit_set(&mut slot.have, h.chunk);
            slot.hashes[h.chunk as usize] = h.hash;
            slot.remaining -= 1;
            slot.remaining == 0
        };
        self.stats.add_bytes(data.len() as u64);
        if finished {
            self.finish_file(h.file)
        } else {
            Ok(Vec::new())
        }
    }

    /// Flushes, optionally re-verifies from disk, restores the timestamp and moves
    /// the part file to its final name.
    fn finish_file(&self, i: u32) -> Result<Vec<ChunkRef>> {
        let f = &self.files[i as usize];
        let (file, hashes) = {
            let mut slot = self.slot(i);
            let file = self.handle(i, &mut slot)?;
            slot.handle = None;
            let hashes = if self.verify_on_disk { slot.hashes.clone() } else { Vec::new() };
            (file, hashes)
        };
        self.dirty.lock().unwrap().remove(&i);
        file.sync_all()?;
        if self.verify_on_disk {
            let bad = verify_part(&file, f.size, self.chunk_size, &hashes)?;
            if !bad.is_empty() {
                let mut slot = self.slot(i);
                for &c in &bad {
                    if bit_get(&slot.have, c) {
                        bit_clear(&mut slot.have, c);
                        slot.remaining += 1;
                        self.stats.sub_bytes(chunk_range(f.size, self.chunk_size, c).1 as u64);
                    }
                }
                return Ok(bad.into_iter().map(|chunk| ChunkRef { file: i, chunk }).collect());
            }
        }
        if let Some(ms) = f.modified {
            let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
        }
        drop(file);
        let name = self.place(i)?;
        self.journal.lock().unwrap().file_done(i, &name)?;
        {
            let mut slot = self.slot(i);
            slot.done = true;
            slot.hashes = Vec::new();
        }
        self.stats.add_file();
        if self.remaining_files.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.complete.send_replace(true);
        }
        Ok(Vec::new())
    }

    /// Never overwrites an existing file: `photo.jpg` becomes `photo (1).jpg`.
    fn place(&self, i: u32) -> Result<String> {
        let rel = &self.files[i as usize].rel;
        let name = rel.file_name().and_then(|n| n.to_str()).context("invalid file name")?;
        let mut target = rel.clone();
        let mut n = 1;
        while self.root.join(&target).exists() {
            target = rel.with_file_name(safety::numbered(name, n));
            n += 1;
        }
        fs::rename(self.part_path(i), self.root.join(&target))
            .with_context(|| format!("can't save {}", target.display()))?;
        Ok(rel_string(&target))
    }

    fn handle(&self, i: u32, slot: &mut Slot) -> Result<Arc<File>> {
        if let Some(file) = &slot.handle {
            return Ok(file.clone());
        }
        let path = self.part_path(i);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("can't create {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("can't write {}", path.display()))?;
        let size = self.files[i as usize].size;
        if file.metadata()?.len() != size {
            file.set_len(size)?;
        }
        let file = Arc::new(file);
        slot.handle = Some(file.clone());
        Ok(file)
    }

    fn part_path(&self, i: u32) -> PathBuf {
        let path = self.root.join(&self.files[i as usize].rel);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(PART_SUFFIX);
        path.with_file_name(name)
    }

    fn mark(&self, i: u32, c: u32, hash: [u8; 32]) {
        let mut slot = self.slot(i);
        if !bit_get(&slot.have, c) {
            bit_set(&mut slot.have, c);
            slot.hashes[c as usize] = hash;
            slot.remaining -= 1;
        }
    }

    pub(super) fn have_list(&self) -> Vec<FileHave> {
        self.slots
            .iter()
            .map(|slot| {
                let slot = slot.lock().unwrap();
                if slot.done {
                    FileHave::Complete
                } else if slot.have.iter().all(|b| *b == 0) {
                    FileHave::None
                } else {
                    FileHave::Partial { bitmap: slot.have.clone() }
                }
            })
            .collect()
    }

    fn publish_progress(&self) {
        let (mut files, mut bytes) = (0u64, 0u64);
        for (i, f) in self.files.iter().enumerate() {
            let slot = self.slots[i].lock().unwrap();
            if slot.done {
                files += 1;
                bytes += f.size;
                continue;
            }
            for c in 0..f.chunks {
                if bit_get(&slot.have, c) {
                    bytes += chunk_range(f.size, self.chunk_size, c).1 as u64;
                }
            }
        }
        self.stats.set_totals(self.files.len() as u64, self.total_bytes());
        self.stats.set_done(files, bytes);
    }

    fn total_bytes(&self) -> u64 {
        self.files.iter().fold(0u64, |sum, f| sum.saturating_add(f.size))
    }

    pub(super) fn summary(&self) -> Summary {
        Summary { files: self.files.len() as u64, bytes: self.total_bytes(), elapsed: self.started.elapsed() }
    }

    /// Flushes the part files written since the last checkpoint, then marks the
    /// chunk records written before the snapshot as durable. Writers keep going
    /// during the flush: every chunk record counted in `covered` had its data
    /// written and its file marked dirty before the snapshot was taken.
    fn checkpoint(&self) -> io::Result<()> {
        let (dirty, covered) = {
            let journal = self.journal.lock().unwrap();
            let dirty = std::mem::take(&mut *self.dirty.lock().unwrap());
            (dirty, journal.chunk_records())
        };
        if dirty.is_empty() {
            return Ok(());
        }
        for file in dirty.values() {
            file.sync_data()?;
        }
        self.journal.lock().unwrap().checkpoint(covered)
    }

    /// Deletes the journal once the transfer is complete.
    pub(super) fn cleanup(&self) {
        self.journal.lock().unwrap().close();
        let _ = fs::remove_dir_all(&self.state_dir);
        if let Some(parent) = self.state_dir.parent() {
            let _ = fs::remove_dir(parent);
        }
    }

    fn slot(&self, i: u32) -> MutexGuard<'_, Slot> {
        self.slots[i as usize].lock().unwrap()
    }
}

fn rel_string(path: &Path) -> String {
    path.iter().map(|c| c.to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Re-reads a file and returns the chunks whose bytes don't match their hashes.
/// Large files are checked on several threads.
fn verify_part(file: &File, size: u64, chunk_size: u32, hashes: &[[u8; 32]]) -> io::Result<Vec<u32>> {
    const CHUNKS_PER_THREAD: usize = 16;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let per_thread = hashes.len().div_ceil(threads).max(CHUNKS_PER_THREAD);
    let check = |first: usize, group: &[[u8; 32]]| -> io::Result<Vec<u32>> {
        let mut bad = Vec::new();
        let mut buf = Vec::new();
        for (i, expected) in group.iter().enumerate() {
            let c = (first + i) as u32;
            let (offset, len) = chunk_range(size, chunk_size, c);
            buf.resize(len as usize, 0);
            disk::read_exact_at(file, &mut buf, offset)?;
            if blake3::hash(&buf) != blake3::Hash::from(*expected) {
                bad.push(c);
            }
        }
        Ok(bad)
    };
    if hashes.len() <= per_thread {
        return check(0, hashes);
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = hashes
            .chunks(per_thread)
            .enumerate()
            .map(|(n, group)| scope.spawn(move || check(n * per_thread, group)))
            .collect();
        let mut bad = Vec::new();
        for handle in handles {
            bad.extend(handle.join().expect("verify thread panicked")?);
        }
        Ok(bad)
    })
}

fn spawn_checkpointer(weak: Weak<TransferState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CHECKPOINT_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else { break };
            if *state.complete.borrow() {
                break;
            }
            let _ = tokio::task::spawn_blocking(move || state.checkpoint()).await;
        }
    });
}

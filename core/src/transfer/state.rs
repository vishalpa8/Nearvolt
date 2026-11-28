//! Everything the receiver knows about one incoming batch: how much of each file
//! has arrived, where it is on disk, and the journal that survives crashes.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};
use tokio::sync::watch;

use super::disk;
use super::journal::{Journal, Replay};
use super::ranges::Ranges;
use super::{human_bytes, FailureKind, Fatal, FileMeter, Stats, Summary};
use crate::identity::Fingerprint;
use crate::protocol::{FileHave, Offer, MAX_CHUNK, MAX_FILES};
use crate::safety;

const CHECKPOINT_EVERY: Duration = Duration::from_secs(2);
const PART_SUFFIX: &str = ".nvpart";
/// Room left over when a file starts, so the phone isn't filled to the last byte.
const SPACE_MARGIN: u64 = 16 << 20;
/// A file that arrives and fails the check this many times isn't going to arrive
/// right: the storage is probably failing.
const MAX_DAMAGED: u32 = 3;

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
    modified: Option<u64>,
}

pub(super) fn plan_files(offer: &Offer) -> Result<Vec<RecvFile>> {
    ensure!(!offer.files.is_empty() && offer.files.len() <= MAX_FILES, "invalid number of files");
    let cleaned = offer.files.iter().map(|f| safety::clean_relative_path(&f.path)).collect::<Result<Vec<_>>>()?;
    let paths = safety::unique_paths(cleaned)?;
    Ok(offer.files.iter().zip(paths).map(|(f, rel)| RecvFile { rel, size: f.size, modified: f.modified }).collect())
}

#[derive(Default)]
struct Slot {
    /// The parts of the file that arrived whole and passed their check.
    have: Ranges,
    /// Ranges added since the last checkpoint, still to be flushed and journaled.
    pending: Vec<(u64, u64)>,
    /// Some stream is saving the finished file.
    finishing: bool,
    /// The user skipped this file: what arrived of it is gone, and more is dropped.
    skipped: bool,
    /// Written to since the last checkpoint.
    dirty: bool,
    handle: Option<Arc<File>>,
    done: bool,
    /// Where this file's part file really is, when the usual name couldn't be used.
    part: Option<PathBuf>,
    /// How many times the whole file arrived and failed the check.
    damaged: u32,
}

/// One chunk being received, from [`TransferState::begin`]. Its bytes are written
/// as they arrive and checked against the sender's hash at the end; a chunk that
/// is dropped before then, because the stream broke, takes back what it counted.
pub(super) struct Chunk {
    state: Arc<TransferState>,
    index: u32,
    file: Option<Arc<File>>,
    offset: u64,
    len: u64,
    hasher: blake3::Hasher,
    counted: u64,
}

impl Chunk {
    pub(super) fn index(&self) -> u32 {
        self.index
    }

    pub(super) fn offset(&self) -> u64 {
        self.offset
    }

    pub(super) fn len(&self) -> u64 {
        self.len
    }

    /// Writes the next bytes, which start at `pos` in the file.
    pub(super) fn write(&mut self, pos: u64, data: &[u8]) -> Result<()> {
        disk::write_all_at(self.file.as_ref().context("chunk already finished")?, data, pos)?;
        self.hasher.update(data);
        self.counted += data.len() as u64;
        self.state.meter.add(0, self.index, data.len() as u64);
        self.state.stats.add_bytes(data.len() as u64);
        Ok(())
    }

    /// All the bytes arrived. Keeps them if they match `expected`; returns false when
    /// they don't, so the sender sends the chunk again. The last chunk of a file
    /// also saves the file under its final name.
    pub(super) fn finish(mut self, expected: [u8; 32]) -> Result<bool> {
        // Every byte is written, and a finished file can't be renamed while a handle is open.
        drop(self.file.take());
        let state = self.state.clone();
        let i = self.index;
        if *self.hasher.finalize().as_bytes() != expected {
            let mut slot = state.slot(i);
            slot.damaged += 1;
            if slot.damaged >= MAX_DAMAGED {
                let message = format!("{} keeps arriving damaged", state.files[i as usize].rel.display());
                return Err(Fatal::new(FailureKind::FileDamaged, message).into());
            }
            return Ok(false);
        }
        let (start, end) = (self.offset, self.offset + self.len);
        let ready = {
            let mut slot = state.slot(i);
            // A file skipped while this chunk was arriving keeps nothing of it.
            if slot.skipped {
                drop(slot);
                // A handle that was open when the file was skipped kept its part file in place.
                state.clear_part(i);
                return Ok(true);
            }
            // Bytes that were already there (a chunk sent twice) don't count twice.
            let fresh = (end - start) - slot.have.overlap(start, end);
            let extra = self.counted - fresh.min(self.counted);
            state.stats.sub_bytes(extra);
            state.meter.sub(0, i, extra);
            self.counted = 0;
            slot.have.insert(start, end);
            slot.pending.push((start, end));
            slot.dirty = true;
            let size = state.files[i as usize].size;
            let whole = slot.have.total() == size;
            let ready =
                whole && !slot.done && !slot.skipped && !slot.finishing && !state.discarded.load(Ordering::Acquire);
            slot.finishing |= ready;
            ready
        };
        if ready {
            state.save(i)?;
        }
        Ok(true)
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        if self.counted > 0 {
            self.state.stats.sub_bytes(self.counted);
            self.state.meter.sub(0, self.index, self.counted);
        }
    }
}

/// Shared by every connection the sender makes, so reconnects pick up exactly
/// where the last one stopped.
pub(super) struct TransferState {
    pub(super) id: String,
    root: PathBuf,
    state_dir: PathBuf,
    files: Vec<RecvFile>,
    slots: Vec<Mutex<Slot>>,
    journal: Mutex<Journal>,
    remaining_files: AtomicU64,
    /// Set when the batch was thrown away; a data stream still running must not
    /// bring its part files back.
    discarded: AtomicBool,
    pub(super) complete: watch::Sender<bool>,
    /// Per-file byte counts for the app's file list. Its batch number is always 0 here.
    pub(super) meter: FileMeter,
    stats: Arc<Stats>,
    started: Instant,
}

impl TransferState {
    /// `root` receives the files; the journal lives in `journal_root/<transfer id>`.
    pub(super) fn open(
        id: String,
        files: Vec<RecvFile>,
        root: &Path,
        journal_root: &Path,
        stats: Arc<Stats>,
    ) -> Result<Arc<Self>> {
        let state_dir = journal_root.join(&id[..32]);
        fs::create_dir_all(&state_dir)?;
        let (journal, replay) = Journal::open(&state_dir.join("journal"))?;
        let state = Self {
            id,
            root: root.to_path_buf(),
            state_dir,
            slots: files.iter().map(|_| Mutex::new(Slot::default())).collect(),
            remaining_files: AtomicU64::new(files.len() as u64),
            discarded: AtomicBool::new(false),
            files,
            journal: Mutex::new(journal),
            complete: watch::channel(false).0,
            meter: FileMeter::default(),
            stats,
            started: Instant::now(),
        };
        state.replay(replay);
        state.check_space()?;
        state.publish_progress();
        if state.remaining_files.load(Ordering::Acquire) == 0 {
            state.complete.send_replace(true);
        }
        let state = Arc::new(state);
        spawn_checkpointer(Arc::downgrade(&state));
        Ok(state)
    }

    /// Finished files stay finished, and partly received ones resume from the
    /// last byte known to be on disk.
    fn replay(&self, replay: Replay) {
        for (i, name) in replay.done {
            let Some(f) = self.files.get(i as usize) else { continue };
            let Ok(parts) = safety::clean_relative_path(&name) else { continue };
            let target = self.root.join(parts.iter().collect::<PathBuf>());
            let part = self.part_path(i);
            // A crash between the journal record and the rename leaves the part file.
            let saved = has_size(&target, f.size) || (has_size(&part, f.size) && fs::rename(&part, &target).is_ok());
            if saved && !self.slot(i).done {
                self.mark_done(i);
            }
        }
        for (i, ranges) in replay.ranges {
            let Some(f) = self.files.get(i as usize) else { continue };
            let mut slot = self.slot(i);
            if !slot.done && has_size(&self.part_path(i), f.size) {
                // Whatever the journal says, nothing past the end of the file counts.
                slot.have = Ranges::from_pairs(ranges.to_pairs().into_iter().map(|(s, e)| (s, e.min(f.size))));
            }
        }
    }

    fn mark_done(&self, i: u32) {
        let mut slot = self.slot(i);
        slot.done = true;
        drop(slot);
        self.remaining_files.fetch_sub(1, Ordering::AcqRel);
    }

    /// What's still to come: for each unfinished file, its size less what's
    /// already on disk. (A part file is created at full size but only takes space
    /// as it's written, so its length says nothing about the room it needs.)
    fn check_space(&self) -> Result<()> {
        let needed = (0..self.files.len() as u32).fold(0u64, |sum, i| {
            let slot = self.slot(i);
            if slot.done || slot.skipped {
                return sum;
            }
            sum.saturating_add(self.files[i as usize].size.saturating_sub(slot.have.total()))
        });
        self.require_space(needed)
    }

    /// Fails with a plain "not enough space" when `needed` more bytes won't fit.
    fn require_space(&self, needed: u64) -> Result<()> {
        let free = disk::available_space(&self.root).unwrap_or(u64::MAX);
        if needed > free {
            return Err(Fatal::new(
                FailureKind::DiskFull,
                format!(
                    "Not enough free space: these files need {}, but only {} is free.",
                    human_bytes(needed),
                    human_bytes(free)
                ),
            )
            .into());
        }
        Ok(())
    }

    pub(super) fn have_list(&self) -> Vec<FileHave> {
        self.slots
            .iter()
            .map(|slot| {
                let slot = slot.lock().unwrap();
                if slot.done || slot.skipped {
                    FileHave::Complete
                } else if slot.have.is_empty() {
                    FileHave::None
                } else {
                    FileHave::Partial { ranges: slot.have.to_pairs() }
                }
            })
            .collect()
    }

    /// Checks a chunk header and gets the part file ready for its bytes. `None` means
    /// there's nothing to keep (the file is saved already, or the chunk is here), so
    /// the data is only read and dropped.
    pub(super) fn begin(self: &Arc<Self>, i: u32, offset: u64, len: u64) -> Result<Option<Chunk>> {
        let f = self.files.get(i as usize).context("data for an unknown file")?;
        let end = offset.checked_add(len).filter(|&end| end <= f.size && len <= MAX_CHUNK);
        let end = end.with_context(|| format!("data for {} has the wrong length", f.rel.display()))?;
        {
            let slot = self.slot(i);
            if slot.done
                || slot.skipped
                || self.discarded.load(Ordering::Acquire)
                || slot.have.contains(offset, end) && len > 0
            {
                return Ok(None);
            }
            self.meter.set_at_least(0, i, slot.have.total());
        }
        // Other apps may have filled the storage since the batch was accepted.
        self.require_space(len.saturating_add(SPACE_MARGIN))?;
        let mut slot = self.slot(i);
        let file = self.handle(i, &mut slot)?;
        Ok(Some(Chunk {
            state: self.clone(),
            index: i,
            file: Some(file),
            offset,
            len,
            hasher: blake3::Hasher::new(),
            counted: 0,
        }))
    }

    /// Every chunk of file `i` is in: flushes it and saves it under its final name.
    fn save(&self, i: u32) -> Result<()> {
        let f = &self.files[i as usize];
        let file = {
            let mut slot = self.slot(i);
            self.handle(i, &mut slot)?
        };
        file.sync_all()?;
        if let Some(ms) = f.modified {
            let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
        }
        let target = self.free_name(i)?;
        self.journal.lock().unwrap().file_done(i, &rel_string(&target))?;
        let part = {
            let mut slot = self.slot(i);
            slot.handle = None;
            slot.dirty = false;
            slot.part.clone().unwrap_or_else(|| self.part_path(i))
        };
        drop(file);
        fs::rename(&part, self.root.join(&target)).with_context(|| format!("can't save {}", target.display()))?;
        self.slot(i).done = true;
        self.meter.finish(0, i);
        self.stats.add_file();
        if self.remaining_files.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.complete.send_replace(true);
        }
        Ok(())
    }
    /// Never overwrites an existing file: `photo.jpg` becomes `photo (1).jpg`.
    fn free_name(&self, i: u32) -> Result<PathBuf> {
        let rel = &self.files[i as usize].rel;
        let name = rel.file_name().and_then(|n| n.to_str()).context("invalid file name")?;
        let mut target = rel.clone();
        let mut n = 1;
        while self.root.join(&target).exists() {
            target = rel.with_file_name(safety::numbered(name, n));
            n += 1;
        }
        Ok(target)
    }

    fn handle(&self, i: u32, slot: &mut Slot) -> Result<Arc<File>> {
        if let Some(file) = &slot.handle {
            return Ok(file.clone());
        }
        ensure!(!self.discarded.load(Ordering::Acquire), "this batch was cancelled");
        let path = self.part_path(i);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("can't create {}", parent.display()))?;
        }
        // A part file left by an earlier install of the app can't be opened by this
        // one, and Android won't let it be deleted either, so another name is used.
        let (file, path) = (0..4)
            .map(|n| if n == 0 { path.clone() } else { alternate(&path, n) })
            .find_map(|candidate| {
                let opened = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&candidate);
                match opened {
                    Ok(file) => Some(Ok((file, candidate))),
                    Err(e) if e.kind() == io::ErrorKind::PermissionDenied => None,
                    Err(e) => Some(Err(e)),
                }
            })
            .unwrap_or_else(|| Err(io::Error::from(io::ErrorKind::PermissionDenied)))
            .with_context(|| format!("can't write {}", path.display()))?;
        let size = self.files[i as usize].size;
        if file.metadata()?.len() != size {
            file.set_len(size)?;
        }
        let file = Arc::new(file);
        slot.handle = Some(file.clone());
        slot.part = Some(path);
        Ok(file)
    }

    fn part_path(&self, i: u32) -> PathBuf {
        let path = self.root.join(&self.files[i as usize].rel);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(PART_SUFFIX);
        path.with_file_name(name)
    }

    fn publish_progress(&self) {
        let (mut files, mut bytes) = (0u64, 0u64);
        for (i, slot) in self.slots.iter().enumerate() {
            let slot = slot.lock().unwrap();
            files += slot.done as u64;
            bytes += if slot.done { self.files[i].size } else { slot.have.total() };
        }
        let (total_files, total_bytes) = self.totals();
        self.stats.set_totals(total_files, total_bytes);
        self.stats.set_done(files, bytes);
    }

    /// Files and bytes this batch is about: the ones the user didn't skip.
    fn totals(&self) -> (u64, u64) {
        let mut files = 0u64;
        let mut bytes = 0u64;
        for (i, f) in self.files.iter().enumerate() {
            if !self.slot(i as u32).skipped {
                files += 1;
                bytes = bytes.saturating_add(f.size);
            }
        }
        (files, bytes)
    }

    pub(super) fn summary(&self) -> Summary {
        let (files, bytes) = self.totals();
        Summary { files, bytes, elapsed: self.started.elapsed() }
    }

    /// Removes the part file of a skipped file.
    fn clear_part(&self, i: u32) {
        remove_part(&self.part_path(i), &self.root);
    }

    /// The user skipped file `i`: its part file goes, later data for it is dropped, and the
    /// batch no longer waits for it. Returns false when there was nothing to skip (it
    /// is saved already, or skipped before).
    pub(super) fn skip(&self, i: u32) -> Result<bool> {
        ensure!((i as usize) < self.files.len(), "skipping an unknown file");
        let part = {
            let mut slot = self.slot(i);
            if slot.done || slot.skipped {
                return Ok(false);
            }
            slot.skipped = true;
            slot.handle = None;
            slot.have = Ranges::default();
            slot.pending.clear();
            slot.part.take()
        };
        if let Some(part) = part {
            remove_part(&part, &self.root);
        }
        remove_part(&self.part_path(i), &self.root);
        self.meter.skip(0, i);
        if self.remaining_files.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.complete.send_replace(true);
        }
        self.publish_progress();
        Ok(true)
    }

    /// Flushes the part files written since the last checkpoint, then records which
    /// ranges of each are on disk. Writers keep going during the flush: the ranges
    /// were taken before it, so it never claims bytes that weren't flushed.
    fn checkpoint(&self) -> io::Result<()> {
        let mut flushed = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            let (file, ranges) = {
                let mut slot = slot.lock().unwrap();
                let Some(file) = slot.handle.clone().filter(|_| slot.dirty && !slot.done) else { continue };
                slot.dirty = false;
                (file, std::mem::take(&mut slot.pending))
            };
            if let Err(e) = file.sync_data() {
                // Not flushed, so they must be recorded next time.
                let mut slot = slot.lock().unwrap();
                slot.pending.extend(ranges);
                slot.dirty = true;
                return Err(e);
            }
            flushed.push((i as u32, ranges));
        }
        if flushed.is_empty() {
            return Ok(());
        }
        let mut journal = self.journal.lock().unwrap();
        for (i, ranges) in flushed {
            for (start, end) in ranges {
                journal.range(i, start, end)?;
            }
        }
        journal.sync()
    }

    /// Deletes the journal once the batch is complete.
    /// The batch was ended on purpose, so nothing is left to resume: removes the
    /// half-received files, their journal and any folder made only for them.
    /// Files that arrived complete and verified stay where they were saved.
    pub(super) fn discard(&self) {
        self.discarded.store(true, Ordering::Release);
        for i in 0..self.files.len() as u32 {
            let mut slot = self.slot(i);
            if slot.done {
                continue;
            }
            slot.handle = None;
            let part = slot.part.take().unwrap_or_else(|| self.part_path(i));
            drop(slot);
            remove_part(&part, &self.root);
            remove_part(&self.part_path(i), &self.root);
        }
        self.cleanup();
    }

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

fn has_size(path: &Path, size: u64) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.len() == size)
}

/// `a.mp4.nvpart` becomes `a.mp4.2.nvpart`.
fn alternate(part: &Path, n: u32) -> PathBuf {
    let name = part.file_name().unwrap_or_default().to_string_lossy();
    let stem = name.strip_suffix(PART_SUFFIX).unwrap_or(&name);
    part.with_file_name(format!("{stem}.{}{PART_SUFFIX}", n + 1))
}

fn rel_string(path: &Path) -> String {
    path.iter().map(|c| c.to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Removes what an unfinished transfer left behind and nobody came back for:
/// part files in `dir` and journals in `journal_root` untouched for `max_age`.
/// A phone that vanished mid-transfer, or an app that was killed, leaves these;
/// a week later there's no transfer left to resume. Best effort, and only files
/// Nearvolt itself named are ever touched.
pub(super) fn sweep_stale(dir: &Path, journal_root: &Path, max_age: Duration) {
    let stale = |path: &Path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > max_age)
    };
    // Part files at any depth, since folders keep their structure.
    fn walk(dir: &Path, root: &Path, depth: u32, stale: &dyn Fn(&Path) -> bool) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 12 {
                    walk(&path, root, depth + 1, stale);
                }
            } else if path.to_string_lossy().ends_with(PART_SUFFIX) && stale(&path) {
                remove_part(&path, root);
            }
        }
    }
    walk(dir, dir, 0, &stale);
    if let Ok(entries) = fs::read_dir(journal_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && stale(&path.join("journal")) {
                let _ = fs::remove_dir_all(&path);
            }
        }
    }
}

/// Deletes a part file, then the folders that held only it, from the inside out.
fn remove_part(part: &Path, root: &Path) {
    if fs::remove_file(part).is_err() {
        return;
    }
    let mut dir = part.parent();
    while let Some(d) = dir.filter(|d| *d != root && d.starts_with(root)) {
        if fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn aged(path: &Path, days: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(days * 86_400)).unwrap();
    }

    #[test]
    fn a_week_old_leftover_goes_and_everything_else_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, journals) = (tmp.path().join("Nearvolt"), tmp.path().join("journals"));
        fs::create_dir_all(dir.join("trip/day1")).unwrap();
        fs::create_dir_all(dir.join("my empty folder")).unwrap();
        fs::create_dir_all(dir.join("recent")).unwrap();
        fs::write(dir.join("trip/day1/old.mp4.nvpart"), b"old").unwrap();
        fs::write(dir.join("trip/photo.jpg"), b"finished").unwrap();
        fs::write(dir.join("recent/new.mp4.nvpart"), b"new").unwrap();
        aged(&dir.join("trip/day1/old.mp4.nvpart"), 8);
        for (name, days) in [("old", 8), ("new", 1)] {
            fs::create_dir_all(journals.join(name)).unwrap();
            fs::write(journals.join(name).join("journal"), b"x").unwrap();
            aged(&journals.join(name).join("journal"), days);
        }

        sweep_stale(&dir, &journals, Duration::from_secs(7 * 86_400));

        assert!(!dir.join("trip/day1").exists(), "the folder that held only the old part file goes");
        assert!(dir.join("trip/photo.jpg").exists(), "a finished file is never touched");
        assert!(dir.join("recent/new.mp4.nvpart").exists(), "a recent part file is kept for resuming");
        assert!(dir.join("my empty folder").exists(), "folders the sweep didn't empty stay");
        assert!(!journals.join("old").exists());
        assert!(journals.join("new").exists());
    }
    fn open_state(root: &Path, journals: &Path, sizes: &[u64]) -> Arc<TransferState> {
        let offer = Offer {
            files: sizes
                .iter()
                .enumerate()
                .map(|(i, &size)| crate::protocol::FileEntry { path: format!("f{i}.bin"), size, modified: None })
                .collect(),
        };
        let files = plan_files(&offer).unwrap();
        TransferState::open(transfer_id(&[1; 32], &offer), files, root, journals, Arc::new(Stats::default())).unwrap()
    }

    fn deliver(
        state: &Arc<TransferState>,
        file: u32,
        offset: u64,
        data: &[u8],
        hash: Option<[u8; 32]>,
    ) -> Result<bool> {
        let mut chunk = state.begin(file, offset, data.len() as u64)?.expect("a chunk to receive");
        chunk.write(offset, data)?;
        chunk.finish(hash.unwrap_or(*blake3::hash(data).as_bytes()))
    }

    #[tokio::test]
    async fn chunks_in_any_order_make_the_file_and_a_damaged_one_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, journals) = (tmp.path().join("out"), tmp.path().join("j"));
        let state = open_state(&root, &journals, &[30]);
        let data: Vec<u8> = (0..30).collect();

        assert!(deliver(&state, 0, 20, &data[20..], None).unwrap());
        // The middle arrives damaged: refused, nothing counted, and the file isn't made.
        assert!(!deliver(&state, 0, 10, &data[10..20], Some([9; 32])).unwrap());
        assert!(!root.join("f0.bin").exists());
        assert_eq!(state.have_list(), vec![FileHave::Partial { ranges: vec![(20, 30)] }]);
        assert!(deliver(&state, 0, 0, &data[..10], None).unwrap());
        assert!(deliver(&state, 0, 10, &data[10..20], None).unwrap());
        assert_eq!(std::fs::read(root.join("f0.bin")).unwrap(), data);
        assert_eq!(state.have_list(), vec![FileHave::Complete]);
        // A late copy of a chunk that is already in is read and dropped.
        assert!(state.begin(0, 0, 10).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_chunk_that_keeps_arriving_damaged_ends_the_batch_with_a_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let state = open_state(&tmp.path().join("out"), &tmp.path().join("j"), &[10]);
        let data = [7u8; 10];
        for _ in 0..MAX_DAMAGED - 1 {
            assert!(!deliver(&state, 0, 0, &data, Some([1; 32])).unwrap());
        }
        let error = deliver(&state, 0, 0, &data, Some([1; 32])).unwrap_err();
        assert_eq!(crate::transfer::classify(&error), FailureKind::FileDamaged);
    }

    #[tokio::test]
    async fn a_stream_that_breaks_mid_chunk_leaves_nothing_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let state = open_state(&tmp.path().join("out"), &tmp.path().join("j"), &[100]);
        let mut chunk = state.begin(0, 0, 100).unwrap().unwrap();
        chunk.write(0, &[1; 40]).unwrap();
        assert_eq!(state.stats.snapshot().done_bytes, 40);
        drop(chunk);
        assert_eq!(state.stats.snapshot().done_bytes, 0);
        assert_eq!(state.have_list(), vec![FileHave::None]);
    }

    #[tokio::test]
    async fn headers_that_do_not_fit_the_offer_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let state = open_state(&tmp.path().join("out"), &tmp.path().join("j"), &[100]);
        assert!(state.begin(1, 0, 10).is_err(), "unknown file");
        assert!(state.begin(0, 95, 10).is_err(), "past the end");
        assert!(state.begin(0, u64::MAX, 2).is_err(), "overflow");
        assert!(state.begin(0, 0, MAX_CHUNK + 1).is_err(), "too long");
    }

    #[tokio::test]
    async fn after_a_crash_the_receiver_resumes_with_exactly_the_flushed_ranges() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, journals) = (tmp.path().join("out"), tmp.path().join("j"));
        let data: Vec<u8> = (0..60).collect();
        {
            let state = open_state(&root, &journals, &[60]);
            assert!(deliver(&state, 0, 40, &data[40..], None).unwrap());
            assert!(deliver(&state, 0, 0, &data[..10], None).unwrap());
            state.checkpoint().unwrap();
            // Arrives after the last checkpoint, so a crash loses it.
            assert!(deliver(&state, 0, 10, &data[10..20], None).unwrap());
            state.journal.lock().unwrap().close();
        }
        let state = open_state(&root, &journals, &[60]);
        assert_eq!(state.have_list(), vec![FileHave::Partial { ranges: vec![(0, 10), (40, 60)] }]);
        assert!(deliver(&state, 0, 10, &data[10..40], None).unwrap());
        assert_eq!(std::fs::read(root.join("f0.bin")).unwrap(), data);
    }

    #[tokio::test]
    async fn an_empty_file_is_made_by_its_empty_chunk() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("out");
        let state = open_state(&root, &tmp.path().join("j"), &[0, 5]);
        assert!(deliver(&state, 0, 0, &[], None).unwrap());
        assert_eq!(std::fs::metadata(root.join("f0.bin")).unwrap().len(), 0);
    }
}

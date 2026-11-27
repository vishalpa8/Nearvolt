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
use super::{human_bytes, FailureKind, Fatal, Stats, Summary};
use crate::identity::Fingerprint;
use crate::protocol::{FileHave, Offer, MAX_FILES};
use crate::safety;

const CHECKPOINT_EVERY: Duration = Duration::from_secs(2);
const PART_SUFFIX: &str = ".nvpart";
const READ_BLOCK: usize = 1 << 20;
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
    /// Bytes written so far, from the start of the file.
    written: u64,
    /// Counts the streams that have started this file. A write from a stream that
    /// was replaced, after a reconnect, still lands but no longer moves `written`.
    epoch: u64,
    /// Written to since the last checkpoint.
    dirty: bool,
    handle: Option<Arc<File>>,
    done: bool,
    /// Where this file's part file really is, when the usual name couldn't be used.
    part: Option<PathBuf>,
    /// How many times the whole file arrived and failed the check.
    damaged: u32,
}

/// One stream's turn at writing a file, from [`TransferState::begin`].
pub(super) struct Writing {
    file: Arc<File>,
    epoch: u64,
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
        for (i, durable) in replay.durable {
            let Some(f) = self.files.get(i as usize) else { continue };
            let mut slot = self.slot(i);
            if !slot.done && has_size(&self.part_path(i), f.size) {
                slot.written = durable.min(f.size);
            }
        }
    }

    fn mark_done(&self, i: u32) {
        let mut slot = self.slot(i);
        slot.done = true;
        slot.written = self.files[i as usize].size;
        drop(slot);
        self.remaining_files.fetch_sub(1, Ordering::AcqRel);
    }

    /// What's still to come: for each unfinished file, its size less what's
    /// already on disk. (A part file is created at full size but only takes space
    /// as it's written, so its length says nothing about the room it needs.)
    fn check_space(&self) -> Result<()> {
        let needed = (0..self.files.len() as u32).fold(0u64, |sum, i| {
            let slot = self.slot(i);
            if slot.done {
                return sum;
            }
            sum.saturating_add(self.files[i as usize].size.saturating_sub(slot.written))
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
                match (slot.done, slot.written) {
                    (true, _) => FileHave::Complete,
                    (false, 0) => FileHave::None,
                    (false, offset) => FileHave::Partial { offset },
                }
            })
            .collect()
    }

    /// Checks a file header and gets the part file ready for bytes from `offset`.
    /// `None` means the file is already saved, so the data is only read and dropped.
    pub(super) fn begin(&self, i: u32, offset: u64, len: u64) -> Result<Option<Writing>> {
        let f = self.files.get(i as usize).context("data for an unknown file")?;
        ensure!(offset.checked_add(len) == Some(f.size), "data for {} has the wrong length", f.rel.display());
        let slot = self.slot(i);
        if slot.done || self.discarded.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(offset <= slot.written, "data for {} skips ahead", f.rel.display());
        // Other apps may have filled the storage since the batch was accepted.
        drop(slot);
        self.require_space(len.saturating_add(SPACE_MARGIN))?;
        let mut slot = self.slot(i);
        if offset < slot.written {
            self.stats.sub_bytes(slot.written - offset);
            slot.written = offset;
        }
        slot.epoch += 1;
        let epoch = slot.epoch;
        Ok(Some(Writing { file: self.handle(i, &mut slot)?, epoch }))
    }

    /// Writes the next bytes of file `i`, which start at `pos`.
    pub(super) fn write(&self, i: u32, writing: &Writing, pos: u64, data: &[u8]) -> Result<()> {
        disk::write_all_at(&writing.file, data, pos)?;
        let mut slot = self.slot(i);
        if slot.epoch == writing.epoch {
            slot.written = pos + data.len() as u64;
            slot.dirty = true;
            drop(slot);
            self.stats.add_bytes(data.len() as u64);
        }
        Ok(())
    }

    /// The whole file has arrived: flushes it, re-reads it from disk and checks
    /// it against the sender's hash, then saves it under its final name. Returns
    /// false when the bytes on disk don't match, so the sender sends it again.
    pub(super) fn finish(&self, i: u32, expected: [u8; 32]) -> Result<bool> {
        let f = &self.files[i as usize];
        let file = {
            let mut slot = self.slot(i);
            if slot.done || self.discarded.load(Ordering::Acquire) {
                return Ok(true);
            }
            ensure!(slot.written == f.size, "{} ended early", f.rel.display());
            self.handle(i, &mut slot)?
        };
        file.sync_all()?;
        if hash_file(&file, f.size, &self.stats)? != expected {
            let mut slot = self.slot(i);
            self.stats.sub_bytes(slot.written);
            slot.written = 0;
            slot.damaged += 1;
            if slot.damaged >= MAX_DAMAGED {
                let message = format!("{} keeps arriving damaged", f.rel.display());
                return Err(Fatal::new(FailureKind::FileDamaged, message).into());
            }
            // The next attempt starts from nothing, so the journal must not claim
            // the bytes that were just found wrong.
            let _ = self.journal.lock().unwrap().durable(i, 0);
            return Ok(false);
        }
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
        self.stats.add_file();
        if self.remaining_files.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.complete.send_replace(true);
        }
        Ok(true)
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
        for slot in &self.slots {
            let slot = slot.lock().unwrap();
            files += slot.done as u64;
            bytes += slot.written;
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

    /// Flushes the part files written since the last checkpoint, then records how
    /// much of each is on disk. Writers keep going during the flush: the recorded
    /// length was taken before it, so it never claims bytes that weren't flushed.
    fn checkpoint(&self) -> io::Result<()> {
        let mut flushed = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            let (file, written) = {
                let mut slot = slot.lock().unwrap();
                let Some(file) = slot.handle.clone().filter(|_| slot.dirty && !slot.done) else { continue };
                slot.dirty = false;
                (file, slot.written)
            };
            file.sync_data()?;
            flushed.push((i as u32, written));
        }
        if flushed.is_empty() {
            return Ok(());
        }
        let mut journal = self.journal.lock().unwrap();
        for (i, written) in flushed {
            journal.durable(i, written)?;
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

/// BLAKE3 of the first `size` bytes of `file`, read back from disk.
pub(super) fn hash_file(file: &File, size: u64, stats: &Stats) -> io::Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    hash_range(file, 0, size, &mut hasher, stats)?;
    Ok(*hasher.finalize().as_bytes())
}

/// Feeds bytes `start..end` of `file` into `hasher`, counting them as checked.
pub(super) fn hash_range(
    file: &File,
    start: u64,
    end: u64,
    hasher: &mut blake3::Hasher,
    stats: &Stats,
) -> io::Result<()> {
    let mut buf = vec![0u8; READ_BLOCK.min((end - start) as usize)];
    let mut pos = start;
    while pos < end {
        let n = (end - pos).min(READ_BLOCK as u64) as usize;
        disk::read_exact_at(file, &mut buf[..n], pos)?;
        hasher.update(&buf[..n]);
        stats.add_checked(n as u64);
        pos += n as u64;
    }
    Ok(())
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
}

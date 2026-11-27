//! The list of files to send and where to read each one from.

use std::fs::{self, File};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::protocol::{FileEntry, Offer, MAX_FILES};
use crate::transfer::{FailureKind, Fatal};

/// Opens files the platform gave access to, such as Android picker results. Each
/// file is opened only when the sender reaches it, so even huge selections never
/// run into the open-file limit.
pub trait SourceOpener: Send + Sync {
    fn open(&self, index: u32) -> io::Result<File>;
}

/// What to send.
pub enum FileSet {
    /// Files and folders on disk. Folders keep their name as the top-level folder.
    Paths(Vec<PathBuf>),
    /// Files described by the platform and opened through `opener`.
    Opened { files: Vec<FileEntry>, opener: Arc<dyn SourceOpener> },
}

enum Sources {
    Paths(Vec<PathBuf>),
    /// Files opened by the platform. `base` is where this batch starts in the
    /// platform's own numbering, since a big selection is cut into several batches.
    Opener {
        opener: Arc<dyn SourceOpener>,
        base: u32,
    },
}

pub struct Manifest {
    pub offer: Offer,
    sources: Sources,
}

/// Most files in one batch. The offer lists every file, and one control message may
/// not pass 64 MiB, so a very large selection is sent as several batches.
pub const MAX_FILES_PER_BATCH: usize = 100_000;

/// And most bytes of file names in one offer, for folders with very long paths.
const MAX_OFFER_BYTES: usize = 16 << 20;

/// How a selection is cut into batches: the number of files in each, in order. The
/// app asks too, so it can list the same batches the engine will send.
pub fn plan_batches(files: &[FileEntry], max_files: usize) -> Vec<usize> {
    let max_files = max_files.max(1);
    let mut sizes = Vec::new();
    let (mut count, mut bytes) = (0usize, 0usize);
    for file in files {
        // About what a file costs in the offer: its name, and the size and time.
        let cost = file.path.len() + 64;
        if count > 0 && (count >= max_files || bytes + cost > MAX_OFFER_BYTES) {
            sizes.push(count);
            (count, bytes) = (0, 0);
        }
        count += 1;
        bytes += cost;
    }
    if count > 0 {
        sizes.push(count);
    }
    sizes
}

impl Manifest {
    pub fn build(set: FileSet) -> Result<Self> {
        let (files, sources) = match set {
            FileSet::Paths(paths) => {
                let (files, paths) = scan(&paths)?;
                (files, Sources::Paths(paths))
            }
            FileSet::Opened { files, opener } => (files, Sources::Opener { opener, base: 0 }),
        };
        if files.is_empty() {
            return Err(Fatal::new(FailureKind::Other, "there are no files to send").into());
        }
        if files.len() > MAX_FILES {
            return Err(Fatal::new(FailureKind::TooBig, format!("too many files (limit {MAX_FILES})")).into());
        }
        Ok(Self { offer: Offer { files }, sources })
    }

    /// Cuts this selection into batches of at most `max_files` files (and a bounded
    /// offer size), as [`plan_batches`] says.
    pub fn split(self, max_files: usize) -> Vec<Manifest> {
        let sizes = plan_batches(&self.offer.files, max_files);
        if sizes.len() <= 1 {
            return vec![self];
        }
        let Manifest { offer, sources } = self;
        let mut files = offer.files.into_iter();
        let mut start = 0usize;
        sizes
            .into_iter()
            .map(|size| {
                let chunk: Vec<FileEntry> = files.by_ref().take(size).collect();
                let sources = match &sources {
                    Sources::Paths(paths) => Sources::Paths(paths[start..start + size].to_vec()),
                    Sources::Opener { opener, base } => {
                        Sources::Opener { opener: opener.clone(), base: base + start as u32 }
                    }
                };
                start += size;
                Manifest { offer: Offer { files: chunk }, sources }
            })
            .collect()
    }

    /// Opens file `index` and checks it hasn't changed since it was offered.
    pub fn open(&self, index: u32) -> Result<File> {
        let entry = &self.offer.files[index as usize];
        let unreadable =
            |e: io::Error| Fatal::new(FailureKind::CannotReadFile, format!("can't read {}: {e}", entry.path));
        let (file, check_time) = match &self.sources {
            Sources::Paths(paths) => (File::open(&paths[index as usize]).map_err(unreadable)?, true),
            Sources::Opener { opener, base } => (opener.open(base + index).map_err(unreadable)?, false),
        };
        let meta = file.metadata().map_err(unreadable)?;
        let changed =
            meta.len() != entry.size || (check_time && meta.modified().ok().and_then(unix_millis) != entry.modified);
        if changed {
            let message = format!("{} changed while it was being sent", entry.path);
            return Err(Fatal::new(FailureKind::CannotReadFile, message).into());
        }
        Ok(file)
    }
}

/// Walks the picked paths. Symbolic links are not followed.
fn scan(paths: &[PathBuf]) -> Result<(Vec<FileEntry>, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut sources = Vec::new();
    for path in paths {
        let path = fs::canonicalize(path).with_context(|| format!("can't find {}", path.display()))?;
        let name =
            path.file_name().with_context(|| format!("can't send {}", path.display()))?.to_string_lossy().into_owned();
        let mut stack = vec![(path, name)];
        while let Some((path, rel)) = stack.pop() {
            let meta = fs::symlink_metadata(&path)?;
            if meta.is_file() {
                files.push(FileEntry {
                    path: rel,
                    size: meta.len(),
                    modified: meta.modified().ok().and_then(unix_millis),
                });
                sources.push(path);
            } else if meta.is_dir() {
                let mut entries = fs::read_dir(&path)
                    .with_context(|| format!("can't read {}", path.display()))?
                    .collect::<Result<Vec<_>, _>>()?;
                entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
                for entry in entries {
                    stack.push((entry.path(), format!("{rel}/{}", entry.file_name().to_string_lossy())));
                }
            }
        }
    }
    Ok((files, sources))
}

pub fn unix_millis(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<FileEntry> {
        (0..n).map(|i| FileEntry { path: format!("f{i}"), size: i as u64, modified: None }).collect()
    }

    #[test]
    fn a_selection_is_cut_into_batches_of_at_most_the_limit() {
        assert_eq!(plan_batches(&[], 3), Vec::<usize>::new());
        assert_eq!(plan_batches(&entries(3), 3), vec![3]);
        assert_eq!(plan_batches(&entries(7), 3), vec![3, 3, 1]);
        assert_eq!(plan_batches(&entries(1), 0), vec![1], "a limit of 0 still sends");
    }

    #[test]
    fn very_long_names_make_smaller_batches() {
        let long = |i: usize| FileEntry { path: format!("{i}{}", "x".repeat(4000)), size: 1, modified: None };
        let files: Vec<FileEntry> = (0..10_000).map(long).collect();
        let sizes = plan_batches(&files, MAX_FILES_PER_BATCH);
        assert!(sizes.len() > 1, "10,000 names of 4 KB don't fit one offer");
        assert_eq!(sizes.iter().sum::<usize>(), files.len());
    }

    /// Records which platform index each open asked for.
    struct Recording(std::sync::Mutex<Vec<u32>>);

    impl SourceOpener for Recording {
        fn open(&self, index: u32) -> io::Result<File> {
            self.0.lock().unwrap().push(index);
            Err(io::Error::other("only recording"))
        }
    }

    #[test]
    fn a_split_batch_still_opens_the_right_platform_file() {
        let opener = Arc::new(Recording(Default::default()));
        let set = FileSet::Opened { files: entries(5), opener: opener.clone() };
        let batches = Manifest::build(set).unwrap().split(2);
        assert_eq!(batches.iter().map(|b| b.offer.files.len()).collect::<Vec<_>>(), vec![2, 2, 1]);
        // File 1 of the second batch is the platform's file 3, and so on.
        let _ = batches[1].open(1);
        let _ = batches[2].open(0);
        let _ = batches[0].open(1);
        assert_eq!(*opener.0.lock().unwrap(), vec![3, 4, 1]);
        assert_eq!(batches[1].offer.files[1].path, "f3");
    }
}

//! The list of files to send and where to read each one from.

use std::fs::{self, File};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};

use crate::protocol::{FileEntry, Offer, MAX_FILES};
use crate::transfer::Fatal;

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
    Opener(Arc<dyn SourceOpener>),
}

pub struct Manifest {
    pub offer: Offer,
    sources: Sources,
}

impl Manifest {
    pub fn build(set: FileSet) -> Result<Self> {
        let (files, sources) = match set {
            FileSet::Paths(paths) => {
                let (files, paths) = scan(&paths)?;
                (files, Sources::Paths(paths))
            }
            FileSet::Opened { files, opener } => (files, Sources::Opener(opener)),
        };
        ensure!(!files.is_empty(), "there are no files to send");
        ensure!(files.len() <= MAX_FILES, "too many files (limit {MAX_FILES})");
        Ok(Self { offer: Offer { files }, sources })
    }

    /// Opens file `index` and checks it hasn't changed since it was offered.
    pub fn open(&self, index: u32) -> Result<File> {
        let entry = &self.offer.files[index as usize];
        let unreadable = |e: io::Error| Fatal(format!("can't read {}: {e}", entry.path));
        let (file, check_time) = match &self.sources {
            Sources::Paths(paths) => (File::open(&paths[index as usize]).map_err(unreadable)?, true),
            Sources::Opener(opener) => (opener.open(index).map_err(unreadable)?, false),
        };
        let meta = file.metadata().map_err(unreadable)?;
        let changed =
            meta.len() != entry.size || (check_time && meta.modified().ok().and_then(unix_millis) != entry.modified);
        if changed {
            return Err(Fatal(format!("{} changed while it was being sent", entry.path)).into());
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

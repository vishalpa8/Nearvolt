//! Builds the list of files to send from the paths a user picked.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};

use crate::protocol::{FileEntry, Offer, MAX_FILES};

pub struct Manifest {
    pub offer: Offer,
    /// Source path for each entry in `offer.files`.
    pub sources: Vec<PathBuf>,
}

impl Manifest {
    /// Folders are sent with their name as the top-level folder. Symbolic links are
    /// not followed.
    pub fn build(paths: &[PathBuf], chunk_size: u32) -> Result<Self> {
        let mut files = Vec::new();
        let mut sources = Vec::new();
        for path in paths {
            let path = fs::canonicalize(path).with_context(|| format!("can't find {}", path.display()))?;
            let name = path
                .file_name()
                .with_context(|| format!("can't send {}", path.display()))?
                .to_string_lossy()
                .into_owned();
            let meta = fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                walk(&path, &name, &mut files, &mut sources)?;
            } else if meta.is_file() {
                push(&path, name, &mut files, &mut sources)?;
            }
        }
        ensure!(!files.is_empty(), "there are no files to send");
        ensure!(files.len() <= MAX_FILES, "too many files (limit {MAX_FILES})");
        Ok(Self { offer: Offer { chunk_size, files }, sources })
    }
}

fn walk(root: &Path, prefix: &str, files: &mut Vec<FileEntry>, sources: &mut Vec<PathBuf>) -> Result<()> {
    let mut stack = vec![(root.to_path_buf(), prefix.to_string())];
    while let Some((dir, rel)) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .with_context(|| format!("can't read {}", dir.display()))?
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            let child = format!("{rel}/{}", entry.file_name().to_string_lossy());
            if kind.is_dir() {
                stack.push((entry.path(), child));
            } else if kind.is_file() {
                push(&entry.path(), child, files, sources)?;
            }
        }
    }
    Ok(())
}

fn push(path: &Path, rel: String, files: &mut Vec<FileEntry>, sources: &mut Vec<PathBuf>) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| format!("can't read {}", path.display()))?;
    files.push(FileEntry { path: rel, size: meta.len(), modified: meta.modified().ok().and_then(unix_millis) });
    sources.push(path.to_path_buf());
    Ok(())
}

pub fn unix_millis(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

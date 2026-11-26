//! Append-only record of how much of each file is safely on disk, so a receiver
//! can resume after a crash. See "Receiver journal" in docs/protocol.md.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long to wait for another receiver that still holds this transfer.
const LOCK_WAIT: Duration = Duration::from_secs(30);

const DURABLE: u8 = 1;
const FILE_DONE: u8 = 3;
const DURABLE_LEN: usize = 13;

pub struct Journal {
    file: Option<File>,
}

#[derive(Default)]
pub struct Replay {
    /// For each file, how many bytes from its start are on disk.
    pub durable: HashMap<u32, u64>,
    /// Finished files and the name each was saved under.
    pub done: Vec<(u32, String)>,
}

impl Journal {
    /// Opens and exclusively locks the journal, so two receivers can never write the
    /// same transfer at once. Waits for a previous receiver to let go.
    pub fn open(path: &Path) -> io::Result<(Self, Replay)> {
        let mut file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if started.elapsed() < LOCK_WAIT => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(io::Error::other("another Nearvolt window is receiving this transfer"));
                }
                Err(TryLockError::Error(e)) => return Err(e),
            }
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let (replay, valid_len) = parse(&bytes);
        file.set_len(valid_len as u64)?;
        file.seek(SeekFrom::End(0))?;
        Ok((Self { file: Some(file) }, replay))
    }

    /// Records that the first `bytes` of `file` are on disk. Call only after the
    /// part file was flushed, then [`Journal::sync`].
    pub fn durable(&mut self, file: u32, bytes: u64) -> io::Result<()> {
        let mut rec = [0u8; DURABLE_LEN];
        rec[0] = DURABLE;
        rec[1..5].copy_from_slice(&file.to_le_bytes());
        rec[5..13].copy_from_slice(&bytes.to_le_bytes());
        self.file()?.write_all(&rec)
    }

    pub fn sync(&mut self) -> io::Result<()> {
        self.file()?.sync_data()
    }

    /// Recorded, and flushed, before the part file is renamed to `name`, so a
    /// crash in between still finishes the rename on resume.
    pub fn file_done(&mut self, file: u32, name: &str) -> io::Result<()> {
        let name = name.as_bytes();
        let mut rec = Vec::with_capacity(7 + name.len());
        rec.push(FILE_DONE);
        rec.extend_from_slice(&file.to_le_bytes());
        rec.extend_from_slice(&(name.len() as u16).to_le_bytes());
        rec.extend_from_slice(name);
        let journal = self.file()?;
        journal.write_all(&rec)?;
        journal.sync_data()
    }

    pub fn close(&mut self) {
        self.file = None;
    }

    fn file(&mut self) -> io::Result<&mut File> {
        self.file.as_mut().ok_or_else(|| io::Error::other("journal is closed"))
    }
}

/// Returns the records and the length of the valid prefix. A record cut short by
/// a crash ends the replay.
fn parse(bytes: &[u8]) -> (Replay, usize) {
    let mut replay = Replay::default();
    let mut pos = 0;
    loop {
        match bytes.get(pos) {
            Some(&DURABLE) if bytes.len() >= pos + DURABLE_LEN => {
                let r = &bytes[pos..pos + DURABLE_LEN];
                let file = u32::from_le_bytes(r[1..5].try_into().unwrap());
                let durable = u64::from_le_bytes(r[5..13].try_into().unwrap());
                replay.durable.insert(file, durable);
                pos += DURABLE_LEN;
            }
            Some(&FILE_DONE) if bytes.len() >= pos + 7 => {
                let file = u32::from_le_bytes(bytes[pos + 1..pos + 5].try_into().unwrap());
                let len = u16::from_le_bytes(bytes[pos + 5..pos + 7].try_into().unwrap()) as usize;
                let Some(name) = bytes.get(pos + 7..pos + 7 + len) else { break };
                let Ok(name) = std::str::from_utf8(name) else { break };
                replay.done.push((file, name.to_string()));
                pos += 7 + len;
            }
            _ => break,
        }
    }
    (replay, pos)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn replay_keeps_the_latest_offset_and_drops_a_torn_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        {
            let (mut j, replay) = Journal::open(&path).unwrap();
            assert!(replay.durable.is_empty());
            j.durable(0, 4096).unwrap();
            j.durable(0, 9000).unwrap();
            j.durable(2, 10).unwrap();
            j.sync().unwrap();
            j.file_done(1, "dir/a.txt").unwrap();
        }
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[DURABLE, 9, 9]);
        fs::write(&path, &bytes).unwrap();

        let (_, replay) = Journal::open(&path).unwrap();
        assert_eq!(replay.durable[&0], 9000);
        assert_eq!(replay.durable[&2], 10);
        assert_eq!(replay.done, vec![(1, "dir/a.txt".to_string())]);
        assert_eq!(fs::metadata(&path).unwrap().len() as usize, bytes.len() - 3);
    }
}

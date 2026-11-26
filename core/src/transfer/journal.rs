//! Append-only record of verified chunks, so a receiver can resume after a crash.
//! See "Receiver journal" in docs/protocol.md.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long to wait for another receiver that still holds this transfer.
const LOCK_WAIT: Duration = Duration::from_secs(30);

const CHUNK: u8 = 1;
const CHECKPOINT: u8 = 2;
const FILE_DONE: u8 = 3;
const CHUNK_LEN: usize = 41;
const CHECKPOINT_LEN: usize = 9;

pub struct Journal {
    file: Option<File>,
    chunk_records: u64,
}

pub struct ChunkRecord {
    pub file: u32,
    pub chunk: u32,
    pub hash: [u8; 32],
    /// Covered by a checkpoint, so its data is known to be on disk.
    pub trusted: bool,
}

#[derive(Default)]
pub struct Replay {
    pub chunks: Vec<ChunkRecord>,
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
        let chunk_records = replay.chunks.len() as u64;
        Ok((Self { file: Some(file), chunk_records }, replay))
    }

    pub fn chunk(&mut self, file: u32, chunk: u32, hash: &[u8; 32]) -> io::Result<()> {
        let mut rec = [0u8; CHUNK_LEN];
        rec[0] = CHUNK;
        rec[1..5].copy_from_slice(&file.to_le_bytes());
        rec[5..9].copy_from_slice(&chunk.to_le_bytes());
        rec[9..41].copy_from_slice(hash);
        self.file()?.write_all(&rec)?;
        self.chunk_records += 1;
        Ok(())
    }

    /// Number of chunk records written so far, including replayed ones.
    pub fn chunk_records(&self) -> u64 {
        self.chunk_records
    }

    /// Declares the first `covered` chunk records durable. Call only after every
    /// part file they touched has been flushed to disk.
    pub fn checkpoint(&mut self, covered: u64) -> io::Result<()> {
        let mut rec = [0u8; CHECKPOINT_LEN];
        rec[0] = CHECKPOINT;
        rec[1..9].copy_from_slice(&covered.to_le_bytes());
        let file = self.file()?;
        file.write_all(&rec)?;
        file.sync_data()
    }

    /// Recorded after the file was flushed and renamed. Becomes durable with the
    /// next checkpoint; until then, replay can still recognize the finished file
    /// by its chunk hashes.
    pub fn file_done(&mut self, file: u32, name: &str) -> io::Result<()> {
        let name = name.as_bytes();
        let mut rec = Vec::with_capacity(7 + name.len());
        rec.push(FILE_DONE);
        rec.extend_from_slice(&file.to_le_bytes());
        rec.extend_from_slice(&(name.len() as u16).to_le_bytes());
        rec.extend_from_slice(name);
        self.file()?.write_all(&rec)
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
    let mut trusted_upto = 0;
    let mut pos = 0;
    loop {
        match bytes.get(pos) {
            Some(&CHUNK) if bytes.len() >= pos + CHUNK_LEN => {
                let r = &bytes[pos..pos + CHUNK_LEN];
                replay.chunks.push(ChunkRecord {
                    file: u32::from_le_bytes(r[1..5].try_into().unwrap()),
                    chunk: u32::from_le_bytes(r[5..9].try_into().unwrap()),
                    hash: r[9..41].try_into().unwrap(),
                    trusted: false,
                });
                pos += CHUNK_LEN;
            }
            Some(&CHECKPOINT) if bytes.len() >= pos + CHECKPOINT_LEN => {
                let covered = u64::from_le_bytes(bytes[pos + 1..pos + 9].try_into().unwrap());
                trusted_upto = (covered as usize).min(replay.chunks.len());
                pos += CHECKPOINT_LEN;
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
    for record in &mut replay.chunks[..trusted_upto] {
        record.trusted = true;
    }
    (replay, pos)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn replay_trusts_only_checkpointed_chunks_and_drops_torn_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        {
            let (mut j, replay) = Journal::open(&path).unwrap();
            assert!(replay.chunks.is_empty());
            j.chunk(0, 0, &[1; 32]).unwrap();
            let covered = j.chunk_records();
            j.chunk(0, 1, &[2; 32]).unwrap();
            j.checkpoint(covered).unwrap();
            j.file_done(1, "dir/a.txt").unwrap();
        }
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[CHUNK, 9, 9]);
        fs::write(&path, &bytes).unwrap();

        let (j, replay) = Journal::open(&path).unwrap();
        assert_eq!(replay.chunks.len(), 2);
        assert!(replay.chunks[0].trusted);
        assert!(!replay.chunks[1].trusted);
        assert_eq!(replay.done, vec![(1, "dir/a.txt".to_string())]);
        assert_eq!(j.chunk_records(), 2);
        assert_eq!(fs::metadata(&path).unwrap().len() as usize, bytes.len() - 3);
    }
}

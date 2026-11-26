//! Wire format. See docs/protocol.md.

use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const ALPN: &[u8] = b"nearvolt/1";
pub const PROTOCOL_VERSION: u32 = 1;

/// Both sides send a control message at least this often.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
/// Silence on the control connection for this long means the link is gone.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(4);

pub type SessionId = [u8; 16];

pub const DEFAULT_CHUNK_SIZE: u32 = 4 * 1024 * 1024;
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
pub const MAX_CHUNK_SIZE: u32 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 10_000_000;
const MAX_CONTROL_LEN: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative path with `/` separators.
    pub path: String,
    pub size: u64,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub modified: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub chunk_size: u32,
    pub files: Vec<FileEntry>,
}

impl Offer {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().fold(0u64, |sum, f| sum.saturating_add(f.size))
    }

    pub fn chunk_count(&self, file: usize) -> u32 {
        chunk_count(self.files[file].size, self.chunk_size)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FileHave {
    None,
    Complete,
    Partial { bitmap: Vec<u8> },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkRef {
    pub file: u32,
    pub chunk: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// First message on every connection. `session` is absent on the control
    /// connection and names the session on data connections. Without a ticket
    /// secret, the control connection carries a commitment to the sender's nonce
    /// and both devices confirm a code (see `pairing::pairing_code`).
    Hello {
        version: u32,
        #[serde(default)]
        secret: Option<[u8; 16]>,
        device_name: String,
        #[serde(default)]
        session: Option<SessionId>,
        #[serde(default)]
        commitment: Option<[u8; 32]>,
    },
    Welcome {
        device_name: String,
        session: SessionId,
        #[serde(default)]
        nonce: Option<[u8; 16]>,
    },
    /// The sender's nonce, revealed only after the receiver sent its own.
    Reveal { nonce: [u8; 16] },
    Reject { reason: String },
    Offer(Offer),
    Accept { have: Vec<FileHave> },
    /// Sender heartbeat.
    Ping,
    /// Receiver heartbeat: verified progress so far.
    Progress { files: u64, bytes: u64 },
    Resend { chunks: Vec<ChunkRef> },
    Complete { files: u64, bytes: u64 },
}

pub async fn write_msg<W: AsyncWrite + Unpin>(stream: &mut W, msg: &Control) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    ensure!(body.len() <= MAX_CONTROL_LEN, "control message is too large");
    stream.write_all(&(body.len() as u32).to_le_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn read_msg<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Control> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    ensure!(len <= MAX_CONTROL_LEN, "control message is too large");
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    serde_json::from_slice(&body).context("malformed control message")
}

/// Reads the next chunk header. Returns `false` if the peer closed the connection
/// cleanly between frames.
pub async fn read_header<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut [u8; CHUNK_HEADER_LEN]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = stream.read(&mut buf[filled..]).await?;
        if n == 0 {
            if filled == 0 {
                return Ok(false);
            }
            bail!("the connection closed in the middle of a chunk");
        }
        filled += n;
    }
    Ok(true)
}

pub const CHUNK_HEADER_LEN: usize = 44;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    pub file: u32,
    pub chunk: u32,
    pub len: u32,
    pub hash: [u8; 32],
}

impl ChunkHeader {
    pub fn encode(&self) -> [u8; CHUNK_HEADER_LEN] {
        let mut b = [0u8; CHUNK_HEADER_LEN];
        b[0..4].copy_from_slice(&self.file.to_le_bytes());
        b[4..8].copy_from_slice(&self.chunk.to_le_bytes());
        b[8..12].copy_from_slice(&self.len.to_le_bytes());
        b[12..44].copy_from_slice(&self.hash);
        b
    }

    pub fn decode(b: &[u8; CHUNK_HEADER_LEN]) -> Self {
        Self {
            file: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            chunk: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            len: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            hash: b[12..44].try_into().unwrap(),
        }
    }
}

pub fn chunk_count(size: u64, chunk_size: u32) -> u32 {
    size.div_ceil(chunk_size as u64) as u32
}

/// Byte offset and length of one chunk of a file.
pub fn chunk_range(size: u64, chunk_size: u32, chunk: u32) -> (u64, u32) {
    let offset = chunk as u64 * chunk_size as u64;
    let len = (size - offset).min(chunk_size as u64) as u32;
    (offset, len)
}

pub fn bitmap_len(chunks: u32) -> usize {
    (chunks as usize).div_ceil(8)
}

pub fn bit_get(bitmap: &[u8], i: u32) -> bool {
    bitmap.get(i as usize / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
}

pub fn bit_set(bitmap: &mut [u8], i: u32) {
    bitmap[i as usize / 8] |= 1 << (i % 8);
}

pub fn bit_clear(bitmap: &mut [u8], i: u32) {
    bitmap[i as usize / 8] &= !(1 << (i % 8));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_math() {
        assert_eq!(chunk_count(0, 4), 0);
        assert_eq!(chunk_count(8, 4), 2);
        assert_eq!(chunk_count(9, 4), 3);
        assert_eq!(chunk_range(9, 4, 2), (8, 1));
    }

    #[test]
    fn header_round_trips() {
        let h = ChunkHeader { file: 3, chunk: 9, len: 77, hash: [5; 32] };
        assert_eq!(ChunkHeader::decode(&h.encode()), h);
    }

    #[test]
    fn control_messages_are_tagged() {
        let json = serde_json::to_string(&Control::Accept { have: vec![FileHave::Complete] }).unwrap();
        assert_eq!(json, r#"{"type":"accept","have":[{"state":"complete"}]}"#);
    }
}

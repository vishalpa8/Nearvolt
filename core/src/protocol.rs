//! Wire format. See docs/protocol.md.

use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::transfer::FailureKind;

pub const ALPN: &[u8] = b"nearvolt/1";
pub const PROTOCOL_VERSION: u32 = 4;

/// Both sides send a control message at least this often.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
/// Silence on the control connection for this long means the link is gone.
/// Phones' Wi-Fi can pause for a few seconds, for example while a Wi-Fi Direct
/// group on the same channel closes, and TCP rides that out on its own.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(8);

pub type SessionId = [u8; 16];

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

/// One batch of files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub files: Vec<FileEntry>,
}

impl Offer {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().fold(0u64, |sum, f| sum.saturating_add(f.size))
    }
}

/// What the receiver already has of one offered file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FileHave {
    None,
    Complete,
    /// The first `offset` bytes are on disk; send the rest.
    Partial {
        offset: u64,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// First message on every stream. `session` is absent on the control stream
    /// and names the connection on data streams. Without a ticket secret, the
    /// control stream carries a commitment to the sender's nonce and both devices
    /// confirm a code (see `pairing::pairing_code`).
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
    Reveal {
        nonce: [u8; 16],
    },
    /// The receiver accepted the connection; batches can follow.
    Ready,
    Reject {
        reason: String,
        #[serde(default)]
        kind: Option<FailureKind>,
    },
    /// A batch of files. Batches are numbered from 0 within a connection.
    Offer {
        batch: u32,
        files: Vec<FileEntry>,
    },
    Accept {
        batch: u32,
        have: Vec<FileHave>,
    },
    /// Sender heartbeat.
    Ping,
    /// Receiver heartbeat: progress of the current batch, as written so far.
    /// `checked` counts bytes read back from disk to verify finished files, so a
    /// long check of a large file isn't mistaken for a stall.
    Progress {
        files: u64,
        bytes: u64,
        #[serde(default)]
        checked: u64,
    },
    /// These files failed verification on disk; send them again from the start.
    Resend {
        files: Vec<u32>,
    },
    Complete {
        batch: u32,
        files: u64,
        bytes: u64,
    },
    /// The sender gives up this batch, for example because a file can't be read
    /// any more. The receiver throws away what it has of it; the connection stays.
    Cancel {
        batch: u32,
    },
    /// The receiver can't go on with this batch, for example because its storage
    /// is full. The sender drops the batch; the connection stays.
    Abort {
        batch: u32,
        kind: FailureKind,
    },
    /// Ends the connection on purpose; the other side stops reconnecting.
    Bye,
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

/// Reads the next file header. Returns `false` if the peer closed the stream
/// cleanly between files.
pub async fn read_header<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut [u8; FILE_HEADER_LEN]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = stream.read(&mut buf[filled..]).await?;
        if n == 0 {
            if filled == 0 {
                return Ok(false);
            }
            bail!("the connection closed in the middle of a file header");
        }
        filled += n;
    }
    Ok(true)
}

/// On a data stream, every file is this header, then `len` bytes of the file
/// starting at `offset`, then the BLAKE3 hash of the whole file.
pub const FILE_HEADER_LEN: usize = 24;
pub const HASH_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub batch: u32,
    pub file: u32,
    pub offset: u64,
    pub len: u64,
}

impl FileHeader {
    pub fn encode(&self) -> [u8; FILE_HEADER_LEN] {
        let mut b = [0u8; FILE_HEADER_LEN];
        b[0..4].copy_from_slice(&self.batch.to_le_bytes());
        b[4..8].copy_from_slice(&self.file.to_le_bytes());
        b[8..16].copy_from_slice(&self.offset.to_le_bytes());
        b[16..24].copy_from_slice(&self.len.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; FILE_HEADER_LEN]) -> Self {
        Self {
            batch: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            file: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            offset: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            len: u64::from_le_bytes(b[16..24].try_into().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips() {
        let h = FileHeader { batch: 2, file: 9, offset: 5_000_000_000, len: 77 };
        assert_eq!(FileHeader::decode(&h.encode()), h);
    }

    #[test]
    fn control_messages_are_tagged() {
        let json =
            serde_json::to_string(&Control::Accept { batch: 1, have: vec![FileHave::Partial { offset: 7 }] }).unwrap();
        assert_eq!(json, r#"{"type":"accept","batch":1,"have":[{"state":"partial","offset":7}]}"#);
    }
}

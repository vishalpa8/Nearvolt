//! Nearvolt core engine: device identity, pairing tickets, encrypted transport
//! and verified, resumable file transfer.

pub mod identity;
pub mod pairing;
pub mod protocol;
pub mod safety;
pub mod transfer;
pub mod transport;
pub mod util;

pub use identity::{Fingerprint, Identity};
pub use pairing::{Ticket, WifiDirect};
pub use transfer::manifest::{FileSet, SourceOpener};
pub use transfer::receiver::{Approver, ReceiveOptions, Receiver, Request};
pub use transfer::sender::{connect, send, Command, Peer, SendOptions};
pub use transfer::{human_bytes, Event, EventSender, Fatal, Progress, Stats, Summary};
pub use transport::{Link, MuxLink, RawStream, TcpLink};

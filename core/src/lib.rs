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
pub use transfer::receiver::{Approver, IncomingOffer, ReceiveOptions, Receiver};
pub use transfer::sender::{send, Peer, SendOptions};
pub use transport::{Link, RawStream, TcpLink};
pub use transfer::{human_bytes, Event, EventSender, Fatal, Progress, Stats, Summary};

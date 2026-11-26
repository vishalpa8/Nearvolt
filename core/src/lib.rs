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
pub use pairing::Ticket;
pub use transfer::receiver::{Approver, IncomingOffer, ReceiveOptions, Receiver};
pub use transfer::sender::{send, SendOptions};
pub use transfer::{human_bytes, Event, EventSender, Fatal, Progress, Stats, Summary};

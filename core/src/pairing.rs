//! Pairing tickets: everything a sender needs to reach and trust a receiver,
//! small enough for a QR code or an NFC tap.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use anyhow::{bail, ensure, Context, Result};
use data_encoding::BASE32_NOPAD;
use ring::rand::{SecureRandom, SystemRandom};

use crate::identity::Fingerprint;

const PREFIX: &str = "NV1";
const VERSION: u8 = 1;
const MAX_ADDRS: usize = 16;
pub const MAX_NAME_LEN: usize = 64;

pub type Secret = [u8; 16];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// Fingerprint of the receiver's certificate; the sender accepts no other.
    pub fingerprint: Fingerprint,
    /// One-time secret proving the sender scanned or tapped this ticket.
    pub secret: Secret,
    pub addrs: Vec<SocketAddr>,
    pub name: String,
}

impl Ticket {
    pub fn encode(&self) -> String {
        let mut b = Vec::with_capacity(128);
        b.push(VERSION);
        b.extend_from_slice(&self.fingerprint);
        b.extend_from_slice(&self.secret);
        let addrs = &self.addrs[..self.addrs.len().min(MAX_ADDRS)];
        b.push(addrs.len() as u8);
        for addr in addrs {
            match addr.ip() {
                IpAddr::V4(ip) => {
                    b.push(4);
                    b.extend_from_slice(&ip.octets());
                }
                IpAddr::V6(ip) => {
                    b.push(6);
                    b.extend_from_slice(&ip.octets());
                }
            }
            b.extend_from_slice(&addr.port().to_be_bytes());
        }
        let name = truncate_utf8(&self.name, MAX_NAME_LEN);
        b.push(name.len() as u8);
        b.extend_from_slice(name.as_bytes());
        format!("{PREFIX}{}", BASE32_NOPAD.encode(&b))
    }

    pub fn decode(text: &str) -> Result<Self> {
        let text = text.trim().to_ascii_uppercase();
        let body = text.strip_prefix(PREFIX).context("this is not a Nearvolt ticket")?;
        let bytes = BASE32_NOPAD.decode(body.as_bytes()).context("the ticket is damaged")?;
        let mut r = Reader(&bytes);
        ensure!(r.u8()? == VERSION, "the ticket is from an unsupported version of Nearvolt");
        let fingerprint = r.array::<32>()?;
        let secret = r.array::<16>()?;
        let count = r.u8()? as usize;
        ensure!(count <= MAX_ADDRS, "the ticket is damaged");
        let mut addrs = Vec::with_capacity(count);
        for _ in 0..count {
            let ip = match r.u8()? {
                4 => IpAddr::V4(Ipv4Addr::from(r.array::<4>()?)),
                6 => IpAddr::V6(Ipv6Addr::from(r.array::<16>()?)),
                _ => bail!("the ticket is damaged"),
            };
            let port = u16::from_be_bytes(r.array::<2>()?);
            addrs.push(SocketAddr::new(ip, port));
        }
        let name_len = r.u8()? as usize;
        ensure!(name_len <= MAX_NAME_LEN, "the ticket is damaged");
        let name = String::from_utf8(r.take(name_len)?.to_vec()).context("the ticket is damaged")?;
        ensure!(r.0.is_empty(), "the ticket is damaged");
        Ok(Self { fingerprint, secret, addrs, name })
    }
}

pub fn new_secret() -> Secret {
    let mut secret = [0u8; 16];
    SystemRandom::new()
        .fill(&mut secret)
        .expect("the system random number generator failed");
    secret
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Addresses on this device that a nearby peer could reach, loopback last.
pub fn local_addrs(port: u16) -> Vec<SocketAddr> {
    let mut addrs: Vec<SocketAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.ip() {
            IpAddr::V4(ip) if !ip.is_link_local() => Some(SocketAddr::new(IpAddr::V4(ip), port)),
            _ => None,
        })
        .collect();
    addrs.truncate(MAX_ADDRS - 1);
    addrs.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    addrs
}

pub fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(self.0.len() >= n, "the ticket is damaged");
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_round_trips() {
        let ticket = Ticket {
            fingerprint: [7; 32],
            secret: new_secret(),
            addrs: vec!["192.168.49.1:4433".parse().unwrap(), "[fe80::1]:9".parse().unwrap()],
            name: "Satya's Pixel".into(),
        };
        let text = ticket.encode();
        assert!(text.starts_with("NV1"));
        assert_eq!(Ticket::decode(&text).unwrap(), ticket);
        assert_eq!(Ticket::decode(&text.to_lowercase()).unwrap(), ticket);
    }

    #[test]
    fn damaged_ticket_is_rejected() {
        let text = Ticket { fingerprint: [1; 32], secret: [2; 16], addrs: vec![], name: "x".into() }.encode();
        assert!(Ticket::decode(&text[..text.len() - 3]).is_err());
        assert!(Ticket::decode("hello").is_err());
    }
}

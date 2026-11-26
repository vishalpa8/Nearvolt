//! Pairing tickets: everything a sender needs to reach and trust a receiver,
//! small enough for a QR code, an NFC tap or a Bluetooth read.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use anyhow::{bail, ensure, Context, Result};
use data_encoding::BASE32_NOPAD;
use ring::rand::{SecureRandom, SystemRandom};

use crate::identity::Fingerprint;

const PREFIX: &str = "NV1";
const VERSION: u8 = 1;
const MAX_ADDRS: usize = 16;
const HAS_SECRET: u8 = 1;
const HAS_WIFI: u8 = 2;
pub const MAX_NAME_LEN: usize = 64;

pub type Secret = [u8; 16];
pub type Nonce = [u8; 16];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// Fingerprint of the receiver's certificate; the sender accepts no other.
    pub fingerprint: Fingerprint,
    /// One-time secret proving the sender scanned or tapped this ticket. Tickets
    /// shared over the air (local network, Bluetooth) leave it out, and both
    /// devices confirm a short code instead.
    pub secret: Option<Secret>,
    pub addrs: Vec<SocketAddr>,
    pub name: String,
    /// Wi-Fi Direct group to join when the devices aren't on the same network.
    pub wifi: Option<WifiDirect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiDirect {
    pub ssid: String,
    pub passphrase: String,
    /// MHz; lets the sender join without scanning every channel.
    pub frequency: u16,
}

impl Ticket {
    /// The same ticket without the secret, for sharing over the air.
    pub fn public(&self) -> Self {
        Self { secret: None, ..self.clone() }
    }

    pub fn encode(&self) -> String {
        let mut b = Vec::with_capacity(160);
        b.push(VERSION);
        let flags = if self.secret.is_some() { HAS_SECRET } else { 0 } | if self.wifi.is_some() { HAS_WIFI } else { 0 };
        b.push(flags);
        b.extend_from_slice(&self.fingerprint);
        if let Some(secret) = &self.secret {
            b.extend_from_slice(secret);
        }
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
        put_str(&mut b, &self.name);
        if let Some(wifi) = &self.wifi {
            put_str(&mut b, &wifi.ssid);
            put_str(&mut b, &wifi.passphrase);
            b.extend_from_slice(&wifi.frequency.to_be_bytes());
        }
        format!("{PREFIX}{}", BASE32_NOPAD.encode(&b))
    }

    pub fn decode(text: &str) -> Result<Self> {
        let text = text.trim().to_ascii_uppercase();
        let body = text.strip_prefix(PREFIX).context("this is not a Nearvolt code")?;
        let bytes = BASE32_NOPAD.decode(body.as_bytes()).context("the code is damaged")?;
        let mut r = Reader(&bytes);
        ensure!(r.u8()? == VERSION, "the code is from an unsupported version of Nearvolt");
        let flags = r.u8()?;
        let fingerprint = r.array::<32>()?;
        let secret = if flags & HAS_SECRET != 0 { Some(r.array::<16>()?) } else { None };
        let count = r.u8()? as usize;
        ensure!(count <= MAX_ADDRS, "the code is damaged");
        let mut addrs = Vec::with_capacity(count);
        for _ in 0..count {
            let ip = match r.u8()? {
                4 => IpAddr::V4(Ipv4Addr::from(r.array::<4>()?)),
                6 => IpAddr::V6(Ipv6Addr::from(r.array::<16>()?)),
                _ => bail!("the code is damaged"),
            };
            let port = u16::from_be_bytes(r.array::<2>()?);
            addrs.push(SocketAddr::new(ip, port));
        }
        let name = r.str()?;
        let wifi = if flags & HAS_WIFI != 0 {
            Some(WifiDirect { ssid: r.str()?, passphrase: r.str()?, frequency: u16::from_be_bytes(r.array::<2>()?) })
        } else {
            None
        };
        ensure!(r.0.is_empty(), "the code is damaged");
        Ok(Self { fingerprint, secret, addrs, name, wifi })
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

/// What the sender sends before seeing the receiver's nonce.
pub fn commitment(nonce: &Nonce) -> [u8; 32] {
    *blake3::Hasher::new().update(b"nearvolt commitment").update(nonce).finalize().as_bytes()
}

/// The 4-digit code both screens show when devices pair without a shared secret.
/// The sender commits to its nonce before it sees the receiver's, so a device in
/// the middle can't steer the two codes to match.
pub fn pairing_code(receiver: &Fingerprint, sender: &Fingerprint, sender_nonce: &Nonce, receiver_nonce: &Nonce) -> String {
    let hash = blake3::Hasher::new()
        .update(b"nearvolt pairing code")
        .update(receiver)
        .update(sender)
        .update(sender_nonce)
        .update(receiver_nonce)
        .finalize();
    let n = u32::from_le_bytes(hash.as_bytes()[..4].try_into().unwrap()) % 10_000;
    format!("{n:04}")
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

fn put_str(b: &mut Vec<u8>, s: &str) {
    let s = truncate_utf8(s, MAX_NAME_LEN);
    b.push(s.len() as u8);
    b.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(self.0.len() >= n, "the code is damaged");
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

    fn str(&mut self) -> Result<String> {
        let len = self.u8()? as usize;
        ensure!(len <= MAX_NAME_LEN, "the code is damaged");
        String::from_utf8(self.take(len)?.to_vec()).context("the code is damaged")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> Ticket {
        Ticket {
            fingerprint: [7; 32],
            secret: Some(new_secret()),
            addrs: vec!["192.168.49.1:4433".parse().unwrap(), "[fe80::1]:9".parse().unwrap()],
            name: "Satya's Pixel".into(),
            wifi: Some(WifiDirect { ssid: "DIRECT-nv-Pixel".into(), passphrase: "kq83hd02".into(), frequency: 5180 }),
        }
    }

    #[test]
    fn ticket_round_trips_with_and_without_optional_parts() {
        let full = ticket();
        let text = full.encode();
        assert!(text.starts_with("NV1"));
        assert_eq!(Ticket::decode(&text).unwrap(), full);
        assert_eq!(Ticket::decode(&text.to_lowercase()).unwrap(), full);

        let bare = Ticket { wifi: None, ..full.public() };
        assert_eq!(Ticket::decode(&bare.encode()).unwrap(), bare);
        assert!(bare.encode().len() < text.len());
    }

    #[test]
    fn damaged_ticket_is_rejected() {
        let text = ticket().encode();
        assert!(Ticket::decode(&text[..text.len() - 3]).is_err());
        assert!(Ticket::decode("hello").is_err());
    }

    #[test]
    fn pairing_code_depends_on_every_input() {
        let (r, s, ns, nr) = ([1; 32], [2; 32], [3; 16], [4; 16]);
        let code = pairing_code(&r, &s, &ns, &nr);
        assert_eq!(code.len(), 4);
        assert_eq!(code, pairing_code(&r, &s, &ns, &nr));
        let others = [
            pairing_code(&s, &r, &ns, &nr),
            pairing_code(&r, &s, &nr, &ns),
            pairing_code(&[9; 32], &s, &ns, &nr),
        ];
        assert!(others.iter().any(|other| *other != code));
    }
}

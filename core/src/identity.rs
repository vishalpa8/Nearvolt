use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// BLAKE3 hash of a device certificate (DER).
pub type Fingerprint = [u8; 32];

const MAGIC: &[u8; 5] = b"NVID1";

/// A device's long-term identity: a self-signed Ed25519 certificate and its key.
#[derive(Clone)]
pub struct Identity {
    cert: Vec<u8>,
    key: Vec<u8>,
    fingerprint: Fingerprint,
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
        let params = rcgen::CertificateParams::new(vec!["nearvolt".to_string()])?;
        let cert = params.self_signed(&key_pair)?;
        Ok(Self::from_parts(cert.der().to_vec(), key_pair.serialize_der()))
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                Self::decode(&bytes).with_context(|| format!("reading identity {}", path.display()))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let identity = Self::generate()?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, identity.encode())?;
                Ok(identity)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    pub fn cert_der(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert.clone())
    }

    pub fn key_der(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()))
    }

    fn from_parts(cert: Vec<u8>, key: Vec<u8>) -> Self {
        let fingerprint = *blake3::hash(&cert).as_bytes();
        Self { cert, key, fingerprint }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAGIC.len() + 8 + self.cert.len() + self.key.len());
        out.extend_from_slice(MAGIC);
        for part in [&self.cert, &self.key] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let rest = bytes.strip_prefix(MAGIC).context("not a Nearvolt identity file")?;
        let (cert, rest) = take_part(rest)?;
        let (key, rest) = take_part(rest)?;
        if !rest.is_empty() {
            bail!("identity file has trailing bytes");
        }
        Ok(Self::from_parts(cert.to_vec(), key.to_vec()))
    }
}

/// First 8 hex characters of a fingerprint, for showing to people.
pub fn short_fingerprint(fp: &Fingerprint) -> String {
    fp[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn take_part(bytes: &[u8]) -> Result<(&[u8], &[u8])> {
    if bytes.len() < 4 {
        bail!("identity file is truncated");
    }
    let (len, rest) = bytes.split_at(4);
    let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
    if rest.len() < len {
        bail!("identity file is truncated");
    }
    Ok(rest.split_at(len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_survives_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id");
        let a = Identity::load_or_create(&path).unwrap();
        let b = Identity::load_or_create(&path).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
    }
}

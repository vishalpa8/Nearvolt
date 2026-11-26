//! Byte streams between two devices, secured with TLS 1.3 and pinned certificates.
//!
//! A [`Link`] opens raw streams: real TCP connections over Wi-Fi, or logical
//! streams multiplexed over a USB cable. TLS is applied on top the same way for
//! both, so the session and transfer code never know which link they run on.
//! A session uses one control stream plus several data streams, which spreads
//! encryption across CPU cores.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::identity::{Fingerprint, Identity};
use crate::protocol::ALPN;
use crate::util::AbortOnDrop;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_NAME: &str = "nearvolt";

pub trait Io: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Io for T {}

/// An unencrypted byte stream to the peer.
pub type RawStream = Box<dyn Io>;
pub type TlsStream = tokio_rustls::TlsStream<RawStream>;

/// Opens raw byte streams to one peer.
pub trait Link: Send + Sync {
    fn open_stream(&self) -> impl Future<Output = Result<RawStream>> + Send;
}

/// TCP over whatever network both devices share. Remembers the address that
/// answered first and tries all of them again only if it stops answering.
pub struct TcpLink {
    addrs: Vec<SocketAddr>,
    preferred: Mutex<Option<SocketAddr>>,
}

impl TcpLink {
    pub fn new(addrs: Vec<SocketAddr>) -> Self {
        Self { addrs, preferred: Mutex::new(None) }
    }
}

impl Link for TcpLink {
    async fn open_stream(&self) -> Result<RawStream> {
        let preferred = *self.preferred.lock().unwrap();
        if let Some(addr) = preferred {
            if let Ok(stream) = connect_tcp(addr).await {
                return Ok(stream);
            }
        }
        ensure!(!self.addrs.is_empty(), "the code has no addresses");
        let (stream, addr) = race(&self.addrs).await?;
        *self.preferred.lock().unwrap() = Some(addr);
        Ok(stream)
    }
}

/// Connects to every address at once and keeps the first that answers.
async fn race(addrs: &[SocketAddr]) -> Result<(RawStream, SocketAddr)> {
    let attempts = addrs
        .iter()
        .map(|&addr| Box::pin(async move { connect_tcp(addr).await.map(|stream| (stream, addr)) }));
    Ok(futures::future::select_ok(attempts).await?.0)
}

async fn connect_tcp(addr: SocketAddr) -> Result<RawStream> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .with_context(|| format!("{addr} did not answer"))??;
    tune(&tcp);
    Ok(Box::new(tcp))
}

/// Accepts TCP connections on `addr` and forwards them as raw streams into
/// `streams`. Returns the bound address and the task, which stops when dropped.
pub fn listen_tcp(addr: SocketAddr, streams: mpsc::Sender<RawStream>) -> Result<(SocketAddr, AbortOnDrop<()>)> {
    let std_listener = std::net::TcpListener::bind(addr)?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;
    let local = listener.local_addr()?;
    let task = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            tune(&tcp);
            if streams.send(Box::new(tcp)).await.is_err() {
                break;
            }
        }
    });
    Ok((local, AbortOnDrop(task)))
}

/// Server side of the TLS handshake.
#[derive(Clone)]
pub struct Acceptor(TlsAcceptor);

impl Acceptor {
    pub fn new(identity: &Identity) -> Result<Self> {
        let provider = provider();
        let mut tls = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(AnyClientCert { provider }))
            .with_single_cert(vec![identity.cert_der()], identity.key_der())?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        Ok(Self(TlsAcceptor::from(Arc::new(tls))))
    }

    /// Returns the stream and the fingerprint of the certificate the peer presented.
    pub async fn handshake(&self, raw: RawStream) -> Result<(TlsStream, Fingerprint)> {
        let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, self.0.accept(raw)).await??;
        let stream = TlsStream::Server(tls);
        let fingerprint = peer_fingerprint(&stream).context("the peer sent no certificate")?;
        Ok((stream, fingerprint))
    }
}

/// Client side of the TLS handshake. With a pinned fingerprint it accepts only
/// that receiver; without one (USB, or a nearby list entry) it accepts any, and
/// the devices confirm a code instead.
#[derive(Clone)]
pub struct Connector(TlsConnector);

impl Connector {
    pub fn new(identity: &Identity, pin: Option<Fingerprint>) -> Result<Self> {
        let provider = provider();
        let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(ServerCert { pin, provider }))
            .with_client_auth_cert(vec![identity.cert_der()], identity.key_der())?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        Ok(Self(TlsConnector::from(Arc::new(tls))))
    }

    pub async fn connect(&self, raw: RawStream) -> Result<TlsStream> {
        let name = ServerName::try_from(SERVER_NAME).expect("valid server name");
        let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, self.0.connect(name, raw))
            .await
            .context("the receiver did not finish the handshake")??;
        Ok(TlsStream::Client(tls))
    }
}

/// Fingerprint of the certificate the other side presented.
pub fn peer_fingerprint(stream: &TlsStream) -> Option<Fingerprint> {
    let certs = match stream {
        TlsStream::Client(s) => s.get_ref().1.peer_certificates(),
        TlsStream::Server(s) => s.get_ref().1.peer_certificates(),
    };
    certs.and_then(|certs| certs.first()).map(|cert| *blake3::hash(cert.as_ref()).as_bytes())
}

/// No Nagle delay for control messages, and TCP keep-alive as a backstop for the
/// protocol's own heartbeat.
fn tune(tcp: &TcpStream) {
    let _ = tcp.set_nodelay(true);
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(2))
        .with_interval(Duration::from_secs(1));
    let _ = SockRef::from(tcp).set_tcp_keepalive(&keepalive);
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts only the pinned receiver certificate, or any when nothing is pinned.
/// The handshake signature is verified either way.
#[derive(Debug)]
struct ServerCert {
    pin: Option<Fingerprint>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ServerCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.pin {
            Some(pin) if blake3::hash(end_entity.as_ref()) != blake3::Hash::from(pin) => Err(
                rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure),
            ),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Requires a client certificate (the sender's identity) without a CA. The sender
/// is authorized separately, by the ticket secret or a confirmed code.
#[derive(Debug)]
struct AnyClientCert {
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for AnyClientCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

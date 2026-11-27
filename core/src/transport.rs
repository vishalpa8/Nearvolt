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
use futures::StreamExt;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
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

    /// Where the streams go, for logs: an address, or the cable.
    fn route(&self) -> String {
        String::new()
    }
}

/// TCP over whatever network both devices share. The first connection tries
/// every address at once; after that, only the address that answered is used,
/// so reconnects take the same route instead of switching to another.
pub struct TcpLink {
    addrs: Vec<SocketAddr>,
    preferred: Mutex<Option<SocketAddr>>,
    /// Tried alone first, and only if it doesn't answer is every address tried.
    first: Option<SocketAddr>,
}

impl TcpLink {
    pub fn new(addrs: Vec<SocketAddr>) -> Self {
        Self { addrs, preferred: Mutex::new(None), first: None }
    }

    /// Like [`TcpLink::new`], but `first` gets the first try on its own: the receiver's
    /// address in a Wi-Fi Direct group this phone has just joined, which beats the
    /// router. If it doesn't answer, the other addresses are tried, so a joined group
    /// that turns out unusable doesn't leave the sender stuck.
    pub fn preferring(first: SocketAddr, addrs: Vec<SocketAddr>) -> Self {
        Self { addrs, preferred: Mutex::new(None), first: Some(first) }
    }
}

impl Link for TcpLink {
    async fn open_stream(&self) -> Result<RawStream> {
        let preferred = *self.preferred.lock().unwrap();
        if let Some(addr) = preferred {
            return connect_tcp(addr).await;
        }
        if let Some(addr) = self.first {
            if let Ok(stream) = connect_tcp(addr).await {
                *self.preferred.lock().unwrap() = Some(addr);
                return Ok(stream);
            }
        }
        ensure!(!self.addrs.is_empty(), "the code has no addresses");
        let (stream, addr) = race(&self.addrs).await?;
        *self.preferred.lock().unwrap() = Some(addr);
        Ok(stream)
    }

    fn route(&self) -> String {
        self.preferred.lock().unwrap().map(|addr| addr.to_string()).unwrap_or_default()
    }
}

/// Connects to every address at once and keeps the first that answers.
async fn race(addrs: &[SocketAddr]) -> Result<(RawStream, SocketAddr)> {
    let attempts =
        addrs.iter().map(|&addr| Box::pin(async move { connect_tcp(addr).await.map(|stream| (stream, addr)) }));
    Ok(futures::future::select_ok(attempts).await?.0)
}

async fn connect_tcp(addr: SocketAddr) -> Result<RawStream> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .with_context(|| format!("{addr} did not answer"))??;
    tune(&tcp);
    Ok(Box::new(tcp))
}

/// Logical streams multiplexed over one byte stream, such as a USB cable. The
/// link outlives the cable: after a replug, `attach` the new stream and streams
/// opened from then on use it, so a transfer resumes on its own.
#[derive(Clone, Default)]
pub struct MuxLink(Arc<Mutex<Option<tokio_yamux::Control>>>);

impl MuxLink {
    /// Runs a session over `io` until it closes, passing streams the peer opens
    /// to `accept`. The two ends must pass different `client` values. Returns
    /// the session task, which stops when dropped.
    pub fn attach(&self, io: impl Io, client: bool, accept: impl Fn(RawStream) + Send + 'static) -> AbortOnDrop<()> {
        let config = tokio_yamux::Config::default();
        let mut session = if client {
            tokio_yamux::Session::new_client(io, config)
        } else {
            tokio_yamux::Session::new_server(io, config)
        };
        *self.0.lock().unwrap() = Some(session.control());
        AbortOnDrop(tokio::spawn(async move {
            while let Some(stream) = session.next().await {
                if let Ok(stream) = stream {
                    accept(always_reading(stream));
                }
            }
        }))
    }
}

impl Link for MuxLink {
    async fn open_stream(&self) -> Result<RawStream> {
        let control = self.0.lock().unwrap().clone();
        let stream = control.context("the cable is not connected")?.open_stream().await?;
        Ok(always_reading(stream))
    }

    fn route(&self) -> String {
        "USB cable".into()
    }
}

/// tokio-yamux applies the peer's window updates only while a read is pending,
/// and data streams are write-only once set up, so writes would stop after the
/// first window. A pipe that is always read keeps them moving.
fn always_reading(stream: tokio_yamux::StreamHandle) -> RawStream {
    const WINDOW: usize = tokio_yamux::config::INITIAL_STREAM_WINDOW as usize;
    let (pipe, user) = tokio::io::duplex(WINDOW);
    tokio::spawn(async move {
        let (mut mux_read, mut mux_write) = tokio::io::split(stream);
        let (pipe_read, mut pipe_write) = tokio::io::split(pipe);
        let mut pipe_read = BufReader::with_capacity(WINDOW, pipe_read);
        let inbound = async {
            let _ = tokio::io::copy(&mut mux_read, &mut pipe_write).await;
            let _ = pipe_write.shutdown().await;
        };
        let outbound = async {
            let _ = tokio::io::copy_buf(&mut pipe_read, &mut mux_write).await;
            let _ = mux_write.shutdown().await;
        };
        tokio::join!(inbound, outbound);
    });
    Box::new(user)
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
            let Ok((tcp, _)) = listener.accept().await else {
                // Out of file descriptors and the like: wait instead of spinning.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
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
    let keepalive = TcpKeepalive::new().with_time(Duration::from_secs(2)).with_interval(Duration::from_secs(1));
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
            Some(pin) if blake3::hash(end_entity.as_ref()) != blake3::Hash::from(pin) => {
                Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
            }
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

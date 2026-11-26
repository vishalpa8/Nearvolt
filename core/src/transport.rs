//! TLS 1.3 over TCP with pinned certificates. A session uses one control
//! connection plus several data connections, which spreads encryption across CPU
//! cores and lets the kernel's TCP offloads carry the bulk of the work.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use socket2::{SockRef, TcpKeepalive};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::identity::{Fingerprint, Identity};
use crate::protocol::ALPN;

pub type TlsStream = tokio_rustls::TlsStream<TcpStream>;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_NAME: &str = "proxima";

/// Listens for senders and completes the TLS handshake with each.
pub struct Listener {
    tcp: TcpListener,
    acceptor: Acceptor,
}

impl Listener {
    pub fn bind(identity: &Identity, addr: SocketAddr) -> Result<Self> {
        let provider = provider();
        let mut tls = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(AnyClientCert { provider }))
            .with_single_cert(vec![identity.cert_der()], identity.key_der())?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let std_listener = std::net::TcpListener::bind(addr)?;
        std_listener.set_nonblocking(true)?;
        Ok(Self {
            tcp: TcpListener::from_std(std_listener)?,
            acceptor: Acceptor(TlsAcceptor::from(Arc::new(tls))),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.tcp.local_addr()?)
    }

    /// Accepts a TCP connection. Run the returned acceptor's handshake on its own
    /// task so a slow or hostile peer can't hold up other connections.
    pub async fn accept(&self) -> Result<(TcpStream, Acceptor)> {
        let (tcp, _) = self.tcp.accept().await?;
        Ok((tcp, self.acceptor.clone()))
    }
}

#[derive(Clone)]
pub struct Acceptor(TlsAcceptor);

impl Acceptor {
    /// Returns the stream and the fingerprint of the certificate the peer presented.
    pub async fn handshake(&self, tcp: TcpStream) -> Result<(TlsStream, Fingerprint)> {
        tune(&tcp);
        let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, self.0.accept(tcp)).await??;
        let fingerprint = tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .map(|cert| *blake3::hash(cert.as_ref()).as_bytes())
            .context("the peer sent no certificate")?;
        Ok((TlsStream::Server(tls), fingerprint))
    }
}

/// Connects to one receiver, accepting only the certificate pinned by the ticket.
#[derive(Clone)]
pub struct Connector(TlsConnector);

impl Connector {
    pub fn new(identity: &Identity, peer: &Fingerprint) -> Result<Self> {
        let provider = provider();
        let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedServerCert { expected: *peer, provider }))
            .with_client_auth_cert(vec![identity.cert_der()], identity.key_der())?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        Ok(Self(TlsConnector::from(Arc::new(tls))))
    }

    pub async fn connect(&self, addr: SocketAddr, timeout: Duration) -> Result<TlsStream> {
        let tcp = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .with_context(|| format!("{addr} did not answer"))??;
        tune(&tcp);
        let name = ServerName::try_from(SERVER_NAME).expect("valid server name");
        let tls = tokio::time::timeout(timeout, self.0.connect(name, tcp))
            .await
            .with_context(|| format!("{addr} did not finish the handshake"))??;
        Ok(TlsStream::Client(tls))
    }

    /// Tries every address at once and keeps the first that answers.
    pub async fn connect_any(&self, addrs: &[SocketAddr], timeout: Duration) -> Result<(TlsStream, SocketAddr)> {
        ensure!(!addrs.is_empty(), "the ticket has no addresses");
        let attempts = addrs.iter().map(|&addr| {
            Box::pin(async move { self.connect(addr, timeout).await.map(|stream| (stream, addr)) })
        });
        Ok(futures::future::select_ok(attempts).await?.0)
    }
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

/// Accepts only the receiver certificate whose fingerprint came from the ticket.
#[derive(Debug)]
struct PinnedServerCert {
    expected: Fingerprint,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedServerCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if blake3::hash(end_entity.as_ref()) == blake3::Hash::from(self.expected) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
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
/// is authorized separately by the ticket secret.
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

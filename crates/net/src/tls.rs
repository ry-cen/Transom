//! TLS 1.3 with self-signed P-256 certificates on both sides. Trust is a pin: the SHA-256 of the
//! peer's SubjectPublicKeyInfo. Port of RemoteTLS.swift:24-81 and Scripts/sillclient.py:288-307.

use crate::identity::DeviceIdentity;
use crate::NetError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{AlertDescription, CertificateError, DigitallySignedStruct, SignatureScheme};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use transom_proto::identity::fingerprint_of_certificate;

pub const SESSION_ALPN: &str = "sill/1";
pub const PAIRING_ALPN: &str = "sill-pair/1";
pub const SNI: &str = "sill";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinPolicy {
    Pinned([u8; 32]),
    AnyKey,
}

/// The Mac fingerprint the verifier saw during the handshake.
#[derive(Clone, Default, Debug)]
pub struct PeerKey(Arc<Mutex<Option<[u8; 32]>>>);

impl PeerKey {
    pub fn get(&self) -> Option<[u8; 32]> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, fp: [u8; 32]) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp);
    }
}

#[derive(Debug)]
struct PinVerifier {
    pin: PinPolicy,
    seen: PeerKey,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let Some(fp) = fingerprint_of_certificate(end_entity.as_ref()) else {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::UnsupportedSignatureAlgorithmContext {
                    signature_algorithm_id: Vec::new(),
                    supported_algorithms: Vec::new(),
                },
            ));
        };
        self.seen.set(fp);
        if let PinPolicy::Pinned(p) = self.pin {
            if p != fp {
                return Err(rustls::Error::InvalidCertificate(
                    CertificateError::ApplicationVerificationFailure,
                ));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is not accepted".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

pub fn client_config(
    id: &DeviceIdentity,
    alpn: &str,
    pin: PinPolicy,
    seen: PeerKey,
) -> Result<Arc<rustls::ClientConfig>, NetError> {
    let tls = |e: rustls::Error| NetError::Tls(e.to_string());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinVerifier {
        pin,
        seen,
        provider: provider.clone(),
    };
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_auth_cert(
            vec![CertificateDer::from(id.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_pkcs8_der.clone())),
        )
        .map_err(tls)?;
    cfg.alpn_protocols = vec![alpn.as_bytes().to_vec()];
    cfg.resumption = rustls::client::Resumption::disabled();
    cfg.enable_early_data = false;
    Ok(Arc::new(cfg))
}

/// TLS over `tcp`; after the handshake, checks that the ALPN is `alpn`. Returns the stream and
/// the Mac's fingerprint.
pub async fn connect(
    tcp: TcpStream,
    id: &DeviceIdentity,
    alpn: &str,
    pin: PinPolicy,
) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, [u8; 32]), NetError> {
    tcp.set_nodelay(true)?;
    let seen = PeerKey::default();
    let cfg = client_config(id, alpn, pin, seen.clone())?;
    let name = ServerName::try_from(SNI).map_err(|e| NetError::Tls(e.to_string()))?;
    let connector = tokio_rustls::TlsConnector::from(cfg);
    let started = Instant::now();
    let mut tls = match connector.connect(name, tcp).await {
        Ok(t) => t,
        Err(e) => {
            if let (PinPolicy::Pinned(p), Some(other)) = (pin, seen.get()) {
                if other != p {
                    return Err(NetError::PinMismatch);
                }
            }
            return Err(classify_io_error(e));
        }
    };
    let (_, conn) = tls.get_ref();
    if conn.alpn_protocol() != Some(alpn.as_bytes()) {
        return Err(NetError::Alpn);
    }
    let fp = seen
        .get()
        .ok_or_else(|| NetError::Tls("the Mac sent no certificate".into()))?;
    // A pairing door takes any key, so only a session can be refused after the handshake.
    if alpn == SESSION_ALPN {
        await_post_handshake_rejection(&mut tls, reject_wait_budget(started.elapsed())).await?;
    }
    Ok((tls, fp))
}

/// The shortest time `connect` waits for a post-handshake rejection.
pub(crate) const MIN_REJECT_WAIT: Duration = Duration::from_millis(100);
/// The longest time `connect` waits for a post-handshake rejection.
pub(crate) const MAX_REJECT_WAIT: Duration = Duration::from_millis(500);

/// How long `connect` listens for a rejection after the handshake: 1.5 times the handshake
/// duration, clamped to 100 ms..=500 ms. The floor covers a host task preempted for a
/// scheduler quantum (about 16 ms on Windows) several times over.
pub(crate) fn reject_wait_budget(handshake: Duration) -> Duration {
    (handshake.saturating_mul(3) / 2).clamp(MIN_REJECT_WAIT, MAX_REJECT_WAIT)
}

/// In TLS 1.3 the client finishes its handshake before the host has checked the client key. A
/// host that refuses the key sends a fatal alert and closes. If we wrote our first message at
/// once, the host's TCP stack could answer it with a reset, and Windows throws away received but
/// unread data on a reset, so the alert would be lost and the refusal would look like a dropped
/// connection. So, for `budget`, write nothing and only listen. The bytes go straight from the
/// socket into rustls, without the `TlsStream` read path, so that plaintext the host sent first
/// stays buffered inside rustls for the next `poll_read`.
async fn await_post_handshake_rejection(
    tls: &mut tokio_rustls::client::TlsStream<TcpStream>,
    budget: Duration,
) -> Result<(), NetError> {
    let deadline = tokio::time::Instant::now() + budget;
    let mut buf = [0u8; 4096];
    let (tcp, conn) = tls.get_mut();
    if !conn.wants_read() {
        // The handshake's last read already left plaintext in rustls: the host spoke first.
        return Ok(());
    }
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        match tokio::time::timeout(remaining, tcp.readable()).await {
            Err(_) => return Ok(()),
            Ok(Err(e)) => return Err(classify_io_error(e)),
            Ok(Ok(())) => {}
        }
        match tcp.try_read(&mut buf) {
            Ok(0) => return Err(NetError::Closed),
            Ok(n) => {
                if feed_rustls(conn, &buf[..n])? {
                    return Ok(());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(classify_io_error(e)),
        }
    }
}

/// Gives `data`, all of it, to rustls. Ok(true) when plaintext is now waiting to be read or the
/// host closed cleanly; Ok(false) for handshake leftovers and partial records.
fn feed_rustls(conn: &mut rustls::ClientConnection, mut data: &[u8]) -> Result<bool, NetError> {
    let mut done = false;
    while !data.is_empty() {
        let taken = conn
            .read_tls(&mut data)
            .map_err(|e| NetError::Tls(e.to_string()))?;
        if taken == 0 {
            return Err(NetError::Tls("the TLS input buffer is full".into()));
        }
        let state = conn
            .process_new_packets()
            .map_err(|e| classify_tls_error(&e))?;
        done |= state.plaintext_bytes_to_read() > 0 || state.peer_has_closed();
    }
    Ok(done)
}

/// A rustls error to NetError. A fatal alert about our certificate means the Mac does not know us.
fn classify_tls_error(e: &rustls::Error) -> NetError {
    match e {
        rustls::Error::AlertReceived(
            AlertDescription::CertificateUnknown
            | AlertDescription::AccessDenied
            | AlertDescription::BadCertificate
            | AlertDescription::UnknownCA
            | AlertDescription::CertificateRequired,
        ) => NetError::NotPaired,
        other => NetError::Tls(other.to_string()),
    }
}

/// io::Error to NetError. A fatal alert about our certificate means the Mac does not know us.
pub fn classify_io_error(e: std::io::Error) -> NetError {
    use std::io::ErrorKind as K;
    if let Some(inner) = e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
        return classify_tls_error(inner);
    }
    match e.kind() {
        K::UnexpectedEof | K::ConnectionReset | K::BrokenPipe => NetError::Closed,
        K::TimedOut => NetError::Timeout,
        _ => NetError::Io(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_alert_is_not_paired() {
        let e = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(AlertDescription::CertificateUnknown),
        );
        assert!(matches!(classify_io_error(e), NetError::NotPaired));
    }

    #[test]
    fn reject_budget_is_clamped() {
        let ms = Duration::from_millis;
        assert_eq!(reject_wait_budget(Duration::ZERO), ms(100));
        assert_eq!(reject_wait_budget(ms(10)), ms(100));
        assert_eq!(reject_wait_budget(ms(20)), ms(100));
        assert_eq!(reject_wait_budget(ms(100)), ms(150));
        assert_eq!(reject_wait_budget(ms(400)), ms(500));
        assert_eq!(reject_wait_budget(Duration::from_secs(60)), ms(500));
        assert_eq!(reject_wait_budget(Duration::MAX), ms(500));
    }

    #[test]
    fn classify_eof_is_closed() {
        let e = std::io::Error::from(std::io::ErrorKind::UnexpectedEof);
        assert!(matches!(classify_io_error(e), NetError::Closed));
    }
}

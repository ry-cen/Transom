//! An in-process Sill host for tests: it speaks the same TLS, pairing and session protocol as
//! the real one, to the extent Transom uses it.

use crate::session::read_message;
use crate::tls::{PAIRING_ALPN, SESSION_ALPN};
use crate::BoxIo;
use crossbeam_channel::{Receiver, Sender};
use rcgen::PublicKeyData;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::io::Read;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use transom_proto::blobs::CursorShape;
use transom_proto::host_settings::{
    HostSettingsChange, HostSettingsState, CHOICES_CAPTURE_SCALE, CHOICES_MAX_FPS, QUALITY_PRESETS,
};
use transom_proto::identity::{
    b64url_decode, b64url_encode, fingerprint_of_certificate, fingerprint_of_spki, mac_id,
};
use transom_proto::messages::{
    from_json, to_json, AppInfo, ClientStats, Goodbye, Hello, InputEvent, LaunchApp, PairRequest,
    PairResult, StreamSource, Viewport, WindowCommand, WindowInfo, WindowList,
};
use transom_proto::pairing::{code_key, mac_proof, recognition_tag, verify_device_proof};
use transom_proto::params::ParameterSets;
use transom_proto::video::VideoPacket;
use transom_proto::wire::{
    encode_message, kind, now_timestamp, MAX_CLIENT_PAYLOAD, MAX_PAIRING_PAYLOAD,
};

#[derive(Clone)]
pub struct MockConfig {
    pub mac_name: String,
    pub tls: bool,
    /// A session only for paired keys (a CertificateUnknown alert otherwise).
    pub require_pairing: bool,
    pub code: String,
    pub qr_secret: [u8; 16],
    pub tries: i64,
    pub ask_reason: String,
    pub windows: Vec<WindowInfo>,
    /// Looped at 30 fps while a source is active.
    pub video: Vec<VideoPacket>,
    pub cursor: Option<CursorShape>,
    /// Sent about 200 ms after the first select of a connection, then the connection closes.
    pub goodbye_after_select: Option<Goodbye>,
    /// Only the first session connection gets the goodbye.
    pub goodbye_only_first: bool,
    /// Send a kind 2 header announcing 5 MiB right after the hello.
    pub oversized_after_hello: bool,
    /// On a TLS connection that fails its handshake (a rejected client key), count every byte
    /// the client sent, keep reading the socket for about 300 ms after the alert, and report the
    /// total as `MockEvent::RejectedClientBytes`. The probe holds a second handle on the socket
    /// for about 300 ms, so in this mode the client sees the alert without a prompt close.
    pub count_rejected_bytes: bool,
    /// Wait this long after reading the ClientHello, before answering it. Makes the client's
    /// measured handshake (and so its post-handshake wait) longer.
    pub handshake_delay: Option<Duration>,
    /// Send the first WindowList right after the handshake, before reading the client's hello,
    /// and no second one after it.
    pub host_speaks_first: bool,
    /// Kind 3 pictures sent after the first window list: (window id, JPEG).
    pub thumbnails: Vec<(u32, Vec<u8>)>,
    /// Kind 4 icons sent after the first window list: (bundle id, PNG).
    pub icons: Vec<(String, Vec<u8>)>,
    /// Kind 5, sent after the first window list when not empty.
    pub apps: Vec<AppInfo>,
    /// Kind 16 sent after the thumbnails; also the state kind 17 changes are laid over, after
    /// Sill's acceptance rule; a refused field keeps its value.
    pub host_settings: Option<HostSettingsState>,
    /// Kind 17 changes are recorded as `MockEvent::ChangeSettings` but never applied or
    /// answered (for the device's pick timeout).
    pub settings_silent: bool,
    /// Where to listen; default 127.0.0.1:0. mockhost --bonjour uses 0.0.0.0:0.
    pub listen: SocketAddr,
}

impl Default for MockConfig {
    fn default() -> Self {
        let window = |id: u32, title: &str| WindowInfo {
            id,
            title: title.to_string(),
            app_name: "Mock App".to_string(),
            bundle_id: "com.example.mock".to_string(),
            width: 800,
            height: 600,
        };
        MockConfig {
            mac_name: "Mock Mac".to_string(),
            tls: true,
            require_pairing: true,
            code: "482913557208".to_string(),
            qr_secret: [0x22; 16],
            tries: 3,
            ask_reason: "shown".to_string(),
            windows: vec![window(101, "First"), window(102, "Second")],
            video: Vec::new(),
            cursor: None,
            goodbye_after_select: None,
            goodbye_only_first: false,
            oversized_after_hello: false,
            count_rejected_bytes: false,
            handshake_delay: None,
            host_speaks_first: false,
            thumbnails: Vec::new(),
            icons: Vec::new(),
            apps: Vec::new(),
            host_settings: None,
            settings_silent: false,
            listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        }
    }
}

/// The host's acceptance rule for a change (Sill's DeviceSettings): the fields it takes. A
/// field outside its choices, or a virtual display that is unavailable, is dropped.
fn accepted(c: &HostSettingsChange, virtual_display_available: bool) -> HostSettingsChange {
    HostSettingsChange {
        token: c.token,
        max_fps: c.max_fps.filter(|v| CHOICES_MAX_FPS.contains(v)),
        bitrate: c
            .bitrate
            .filter(|v| QUALITY_PRESETS.iter().any(|(b, _)| b == v)),
        capture_scale: c
            .capture_scale
            .filter(|v| CHOICES_CAPTURE_SCALE.contains(v)),
        prioritize_speed: c.prioritize_speed,
        virtual_display: c
            .virtual_display
            .filter(|v| !*v || virtual_display_available),
        direct_wireless: c.direct_wireless,
    }
}

#[derive(Clone, Debug)]
pub enum MockEvent {
    FirstKind(u8),
    Hello(Hello),
    Viewport(Viewport),
    Select(StreamSource),
    Input(InputEvent),
    Stats(ClientStats),
    Ping,
    Pair(PairRequest),
    ChangeSettings(HostSettingsChange),
    /// Kind 7: the bundle id to launch.
    Launch(String),
    WindowCommand(WindowCommand),
    Unknown(u8),
    /// Total bytes (ClientHello included) a client sent on a connection whose TLS handshake
    /// failed; only with `MockConfig::count_rejected_bytes`.
    RejectedClientBytes(usize),
    Disconnected,
}

pub struct MockHost {
    pub addr: SocketAddr,
    pub fingerprint: [u8; 32],
    pub recognition_key: [u8; 32],
    events: Receiver<MockEvent>,
    paired: Arc<Mutex<Vec<[u8; 32]>>>,
    accept: tokio::task::JoinHandle<()>,
}

impl Drop for MockHost {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl MockHost {
    pub async fn start(cfg: MockConfig) -> std::io::Result<MockHost> {
        let io_err = |e: &dyn std::fmt::Display| std::io::Error::other(e.to_string());
        let kp =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| io_err(&e))?;
        let fingerprint = fingerprint_of_spki(&kp.subject_public_key_info())
            .ok_or_else(|| io_err(&"unexpected key encoding"))?;
        let mut params =
            rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|e| io_err(&e))?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "mock-host");
        let cert = params.self_signed(&kp).map_err(|e| io_err(&e))?;
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = kp.serialize_der();
        let mut recognition_key = [0u8; 32];
        getrandom::fill(&mut recognition_key).map_err(|e| io_err(&e))?;

        let paired: Arc<Mutex<Vec<[u8; 32]>>> = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server_config =
            |accept: Accept, alpn: &str| -> std::io::Result<Arc<rustls::ServerConfig>> {
                let verifier = Arc::new(MockVerifier {
                    accept,
                    provider: provider.clone(),
                });
                let mut c = rustls::ServerConfig::builder_with_provider(provider.clone())
                    .with_protocol_versions(&[&rustls::version::TLS13])
                    .map_err(|e| io_err(&e))?
                    .with_client_cert_verifier(verifier)
                    .with_single_cert(
                        vec![cert_der.clone()],
                        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.clone())),
                    )
                    .map_err(|e| io_err(&e))?;
                c.alpn_protocols = vec![alpn.as_bytes().to_vec()];
                Ok(Arc::new(c))
            };
        let pair_config = server_config(Accept::Any, PAIRING_ALPN)?;
        let session_config = server_config(
            if cfg.require_pairing {
                Accept::Paired(paired.clone())
            } else {
                Accept::Any
            },
            SESSION_ALPN,
        )?;

        let listener = TcpListener::bind(cfg.listen).await?;
        let addr = listener.local_addr()?;
        let (events_tx, events) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared {
            tries: Mutex::new(cfg.tries),
            cfg,
            paired: paired.clone(),
            events: events_tx,
            fingerprint,
            recognition_key,
            goodbye_used: AtomicBool::new(false),
            pair_config,
            session_config,
        });
        let accept = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(tcp, shared).await;
                });
            }
        });
        Ok(MockHost {
            addr,
            fingerprint,
            recognition_key,
            events,
            paired,
            accept,
        })
    }

    pub fn events(&self) -> &Receiver<MockEvent> {
        &self.events
    }

    /// Pre-pairs a key.
    pub fn pair_key(&self, fp: [u8; 32]) {
        let mut p = self.paired.lock().unwrap_or_else(|e| e.into_inner());
        if !p.contains(&fp) {
            p.push(fp);
        }
    }

    pub fn paired(&self) -> Vec<[u8; 32]> {
        self.paired
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The Bonjour `r` value for this Mac.
    pub fn txt_tag(&self) -> String {
        recognition_tag(&self.recognition_key, [1, 2, 3, 4, 5, 6])
    }
}

struct Shared {
    cfg: MockConfig,
    tries: Mutex<i64>,
    paired: Arc<Mutex<Vec<[u8; 32]>>>,
    events: Sender<MockEvent>,
    fingerprint: [u8; 32],
    recognition_key: [u8; 32],
    goodbye_used: AtomicBool,
    pair_config: Arc<rustls::ServerConfig>,
    session_config: Arc<rustls::ServerConfig>,
}

impl Shared {
    fn emit(&self, e: MockEvent) {
        let _ = self.events.send(e);
    }
}

enum Accept {
    Any,
    Paired(Arc<Mutex<Vec<[u8; 32]>>>),
}

#[derive(Debug)]
struct MockVerifier {
    accept: Accept,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl std::fmt::Debug for Accept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Accept::Any => f.write_str("Any"),
            Accept::Paired(_) => f.write_str("Paired"),
        }
    }
}

impl ClientCertVerifier for MockVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let Some(fp) = fingerprint_of_certificate(end_entity.as_ref()) else {
            return Err(rustls::Error::General("not a P-256 certificate".into()));
        };
        let ok = match &self.accept {
            Accept::Any => true,
            Accept::Paired(list) => list.lock().unwrap_or_else(|e| e.into_inner()).contains(&fp),
        };
        if ok {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Other(rustls::OtherError(Arc::new(
                    std::io::Error::other("this key is not paired"),
                ))),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is not accepted".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
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

/// A TCP stream that counts the bytes read from it.
struct CountingIo {
    inner: TcpStream,
    count: Arc<AtomicUsize>,
}

impl AsyncRead for CountingIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(r, Poll::Ready(Ok(()))) {
            let n = buf.filled().len() - before;
            self.count.fetch_add(n, Ordering::SeqCst);
        }
        r
    }
}

impl AsyncWrite for CountingIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Reads what the client still sends on a duplicate of the socket, for up to 300 ms or until it
/// closes. Returns the byte count.
async fn drain_probe(mut probe: std::net::TcpStream) -> usize {
    let end = tokio::time::Instant::now() + Duration::from_millis(300);
    let mut total = 0;
    let mut buf = [0u8; 4096];
    while tokio::time::Instant::now() < end {
        match probe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(_) => break,
        }
    }
    total
}

async fn handle_connection(tcp: TcpStream, shared: Arc<Shared>) -> std::io::Result<()> {
    tcp.set_nodelay(true)?;
    if !shared.cfg.tls {
        handle_session(Box::new(tcp), shared).await;
        return Ok(());
    }
    // A duplicate handle on the socket, so the bytes a rejected client still sends can be read
    // after the TLS acceptor has given up (and dropped) its own handle.
    let (tcp, mut probe) = if shared.cfg.count_rejected_bytes {
        let std_tcp = tcp.into_std()?;
        let probe = std_tcp.try_clone()?;
        probe.set_nonblocking(true)?;
        (TcpStream::from_std(std_tcp)?, Some(probe))
    } else {
        (tcp, None)
    };
    let count = Arc::new(AtomicUsize::new(0));
    let counting = CountingIo {
        inner: tcp,
        count: count.clone(),
    };
    let start =
        tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), counting)
            .await?;
    let offered: Vec<Vec<u8>> = start
        .client_hello()
        .alpn()
        .map(|i| i.map(|a| a.to_vec()).collect())
        .unwrap_or_default();
    let wants = |alpn: &str| offered.iter().any(|a| a == alpn.as_bytes());
    let (config, pairing) = if wants(PAIRING_ALPN) {
        (shared.pair_config.clone(), true)
    } else if wants(SESSION_ALPN) {
        (shared.session_config.clone(), false)
    } else {
        return Ok(());
    };
    if let Some(d) = shared.cfg.handshake_delay {
        tokio::time::sleep(d).await;
    }
    let tls = match start.into_stream(config).await {
        Ok(t) => t,
        Err(e) => {
            if let Some(p) = probe.take() {
                let extra = drain_probe(p).await;
                shared.emit(MockEvent::RejectedClientBytes(
                    count.load(Ordering::SeqCst) + extra,
                ));
            }
            return Err(e);
        }
    };
    drop(probe);
    let client_fp = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|c| c.first())
        .and_then(|c| fingerprint_of_certificate(c.as_ref()));
    if pairing {
        handle_pairing(tls, client_fp, shared).await;
    } else {
        handle_session(Box::new(tls), shared).await;
    }
    Ok(())
}

async fn handle_pairing<S>(mut io: S, client_fp: Option<[u8; 32]>, shared: Arc<Shared>)
where
    S: AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let read = tokio::time::timeout(
        Duration::from_secs(10),
        read_message(&mut io, |_| MAX_PAIRING_PAYLOAD, || {}),
    )
    .await;
    let Ok(Ok((header, payload))) = read else {
        return;
    };
    if header.kind != kind::PAIR_REQUEST {
        return;
    }
    let Some(req) = from_json::<PairRequest>(&payload) else {
        return;
    };
    shared.emit(MockEvent::Pair(req.clone()));
    let refused = |reason: &str| PairResult {
        ok: false,
        reason: Some(reason.to_string()),
        ..PairResult::default()
    };
    let result = match req.method.as_str() {
        "ask" => refused(&shared.cfg.ask_reason),
        "cancel" => refused("closed"),
        "code" | "qr" => {
            let secret_is_code = req.method == "code";
            let code = shared.cfg.code.clone();
            let qr = shared.cfg.qr_secret;
            let host_fp = shared.fingerprint;
            let key = tokio::task::spawn_blocking(move || {
                if secret_is_code {
                    code_key(&code, &host_fp).map(|k| k.to_vec())
                } else {
                    Some(qr.to_vec())
                }
            })
            .await
            .ok()
            .flatten();
            let proof = b64url_decode(&req.proof);
            let valid = match (&key, &proof, client_fp) {
                (Some(k), Some(p), Some(dev)) => verify_device_proof(p, k, &dev, &host_fp),
                _ => false,
            };
            match (valid, key, client_fp) {
                (true, Some(k), Some(dev)) => {
                    {
                        let mut p = shared.paired.lock().unwrap_or_else(|e| e.into_inner());
                        if !p.contains(&dev) {
                            p.push(dev);
                        }
                    }
                    PairResult {
                        ok: true,
                        proof: Some(b64url_encode(&mac_proof(&k, &host_fp, &dev))),
                        mac_id: Some(mac_id(&host_fp)),
                        name: Some(shared.cfg.mac_name.clone()),
                        recognition_key: Some(b64url_encode(&shared.recognition_key)),
                        ..PairResult::default()
                    }
                }
                _ => {
                    let left = {
                        let mut t = shared.tries.lock().unwrap_or_else(|e| e.into_inner());
                        *t -= 1;
                        *t
                    };
                    PairResult {
                        tries_left: Some(left),
                        ..refused("code")
                    }
                }
            }
        }
        _ => refused("closed"),
    };
    let msg = encode_message(kind::PAIR_RESULT, now_timestamp(), false, &to_json(&result));
    if io.write_all(&msg).await.is_ok() {
        let _ = io.flush().await;
    }
    let _ = io.shutdown().await;
}

enum Out {
    Data(Vec<u8>),
    Close,
}

struct AbortOnDrop(Vec<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for h in &self.0 {
            h.abort();
        }
    }
}

fn window_list(shared: &Shared, active: StreamSource) -> Vec<u8> {
    let list = WindowList {
        mac_name: shared.cfg.mac_name.clone(),
        windows: shared.cfg.windows.clone(),
        active,
        launch_id: None,
        host_version: Some("mock".to_string()),
        protocol: Some(1),
        gestures: None,
        clipboard: None,
    };
    encode_message(kind::WINDOW_LIST, now_timestamp(), false, &to_json(&list))
}

async fn handle_session(io: BoxIo, shared: Arc<Shared>) {
    session_inner(io, &shared).await;
    shared.emit(MockEvent::Disconnected);
}

async fn session_inner(io: BoxIo, shared: &Arc<Shared>) {
    let (mut rd, mut wr) = tokio::io::split(io);
    if shared.cfg.host_speaks_first {
        let list = window_list(shared, StreamSource::None {});
        if wr.write_all(&list).await.is_err() || wr.flush().await.is_err() {
            return;
        }
    }
    let first = tokio::time::timeout(
        Duration::from_secs(10),
        read_message(&mut rd, |_| MAX_CLIENT_PAYLOAD, || {}),
    )
    .await;
    let Ok(Ok((header, payload))) = first else {
        return;
    };
    shared.emit(MockEvent::FirstKind(header.kind));
    if header.kind == kind::HELLO {
        if let Some(h) = from_json::<Hello>(&payload) {
            shared.emit(MockEvent::Hello(h));
        }
    }

    let (otx, mut orx) = mpsc::unbounded_channel::<Out>();
    let writer = tokio::spawn(async move {
        while let Some(o) = orx.recv().await {
            match o {
                Out::Data(buf) => {
                    if wr.write_all(&buf).await.is_err() || wr.flush().await.is_err() {
                        return;
                    }
                }
                Out::Close => break,
            }
        }
        let _ = wr.shutdown().await;
    });
    let mut guard = AbortOnDrop(vec![writer.abort_handle()]);
    let send = |k: u8, p: &[u8]| {
        let _ = otx.send(Out::Data(encode_message(k, now_timestamp(), false, p)));
    };

    if shared.cfg.oversized_after_hello {
        let mut head = vec![kind::WINDOW_LIST];
        head.extend_from_slice(&now_timestamp().to_bits().to_be_bytes());
        head.push(0);
        head.extend_from_slice(&(5u32 << 20).to_be_bytes());
        let _ = otx.send(Out::Data(head));
        tokio::time::sleep(Duration::from_secs(2)).await;
        return;
    }

    let mut active = StreamSource::None {};
    if !shared.cfg.host_speaks_first {
        let _ = otx.send(Out::Data(window_list(shared, active)));
    }
    if let Some(c) = &shared.cfg.cursor {
        send(kind::CURSOR_SHAPE, &c.encode());
    }
    if !shared.cfg.apps.is_empty() {
        send(kind::APP_LIST, &to_json(&shared.cfg.apps));
    }
    for (bundle_id, png) in &shared.cfg.icons {
        let mut p = u16::try_from(bundle_id.len())
            .unwrap_or(u16::MAX)
            .to_be_bytes()
            .to_vec();
        p.extend_from_slice(bundle_id.as_bytes());
        p.extend_from_slice(png);
        send(kind::APP_ICON, &p);
    }
    for (id, jpeg) in &shared.cfg.thumbnails {
        let mut p = id.to_be_bytes().to_vec();
        p.extend_from_slice(jpeg);
        send(kind::THUMBNAIL, &p);
    }
    let mut settings = shared.cfg.host_settings.clone();
    if let Some(s) = &settings {
        send(kind::HOST_SETTINGS, &to_json(s));
    }

    let closing = Arc::new(Notify::new());
    let mut video_running = false;
    let mut selected_once = false;
    loop {
        let msg = tokio::select! {
            m = read_message(&mut rd, |_| MAX_CLIENT_PAYLOAD, || {}) => m,
            _ = closing.notified() => return,
        };
        let Ok((header, payload)) = msg else {
            return;
        };
        match header.kind {
            kind::SELECT_SOURCE => {
                let Some(s) = from_json::<StreamSource>(&payload) else {
                    continue;
                };
                shared.emit(MockEvent::Select(s));
                active = s;
                let _ = otx.send(Out::Data(window_list(shared, active)));
                if !video_running && !shared.cfg.video.is_empty() {
                    video_running = true;
                    let video = shared.cfg.video.clone();
                    let tx = otx.clone();
                    let task = tokio::spawn(async move {
                        let mut tick = tokio::time::interval(Duration::from_millis(33));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        let mut i = 0usize;
                        loop {
                            tick.tick().await;
                            let Some(packet) = video.get(i % video.len()) else {
                                return;
                            };
                            i += 1;
                            let msg = match packet {
                                VideoPacket::ParameterSets {
                                    nal_length_size,
                                    sets,
                                } => encode_message(
                                    kind::PARAMETER_SETS,
                                    now_timestamp(),
                                    false,
                                    &ParameterSets {
                                        nal_length_size: *nal_length_size,
                                        sets: sets.clone(),
                                    }
                                    .encode(),
                                ),
                                VideoPacket::Frame {
                                    data, is_keyframe, ..
                                } => {
                                    encode_message(kind::FRAME, now_timestamp(), *is_keyframe, data)
                                }
                                VideoPacket::Reset => continue,
                            };
                            if tx.send(Out::Data(msg)).is_err() {
                                return;
                            }
                        }
                    });
                    guard.0.push(task.abort_handle());
                }
                if !selected_once {
                    selected_once = true;
                    if let Some(g) = shared.cfg.goodbye_after_select.clone() {
                        let allowed = !shared.cfg.goodbye_only_first
                            || !shared.goodbye_used.swap(true, Ordering::SeqCst);
                        if allowed {
                            let tx = otx.clone();
                            let closing = closing.clone();
                            let task = tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(200)).await;
                                let _ = tx.send(Out::Data(encode_message(
                                    kind::GOODBYE,
                                    now_timestamp(),
                                    false,
                                    &to_json(&g),
                                )));
                                let _ = tx.send(Out::Close);
                                // Let the writer flush the goodbye and close before the reader
                                // loop gives up the connection.
                                tokio::time::sleep(Duration::from_millis(200)).await;
                                closing.notify_one();
                            });
                            guard.0.push(task.abort_handle());
                        }
                    }
                }
            }
            kind::INPUT => {
                if let Some(ev) = from_json::<InputEvent>(&payload) {
                    shared.emit(MockEvent::Input(ev));
                }
            }
            kind::VIEWPORT => {
                if let Some(v) = from_json::<Viewport>(&payload) {
                    shared.emit(MockEvent::Viewport(v));
                }
            }
            kind::PING => {
                shared.emit(MockEvent::Ping);
                send(kind::PONG, &payload);
            }
            kind::CLIENT_STATS => {
                if let Some(s) = from_json::<ClientStats>(&payload) {
                    shared.emit(MockEvent::Stats(s));
                }
            }
            kind::CHANGE_SETTINGS => {
                let Some(c) = from_json::<HostSettingsChange>(&payload) else {
                    continue;
                };
                shared.emit(MockEvent::ChangeSettings(c.clone()));
                if shared.cfg.settings_silent {
                    continue;
                }
                if let Some(s) = settings.as_mut() {
                    let mut ok = accepted(&c, s.virtual_display_available);
                    if s.settings.direct_wireless.is_none() {
                        ok.direct_wireless = None;
                    }
                    s.settings = ok.applied_to(&s.settings);
                    s.answering = c.token;
                    send(kind::HOST_SETTINGS, &to_json(&*s));
                    s.answering = None;
                }
            }
            kind::LAUNCH_APP => {
                if let Some(l) = from_json::<LaunchApp>(&payload) {
                    shared.emit(MockEvent::Launch(l.bundle_id));
                }
            }
            kind::WINDOW_COMMAND => {
                if let Some(c) = from_json::<WindowCommand>(&payload) {
                    shared.emit(MockEvent::WindowCommand(c));
                }
            }
            other => shared.emit(MockEvent::Unknown(other)),
        }
    }
}

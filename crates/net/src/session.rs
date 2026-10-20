//! One connection to a Sill host: hello, source selection, video, input, ping, stats.
//! Port of the session parts of StreamClient.swift:851-876, 1481-1510, 3617-3777.

use crate::clock::mono_seconds;
use crate::tls::classify_io_error;
use crate::{dial, BoxIo, NetError};
use crossbeam_channel::{Sender, TrySendError};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use transom_proto::blobs::CursorShape;
use transom_proto::host_settings::{HostSettingsChange, HostSettingsState};
use transom_proto::messages::{
    decode_goodbye, from_json, to_json, AppInfo, ClientStats, Goodbye, Hello, InputEvent, MacInfo,
    PointerAction, SignedMacInfo, StreamSource, Viewport, WindowCommand, WindowList,
};
use transom_proto::params::ParameterSets;
use transom_proto::video::VideoPacket;
use transom_proto::wire::{
    encode_message, host_payload_cap, kind, now_timestamp, Header, HEADER_LEN,
};

/// Payloads are read in pieces of at most this many bytes (iOSClient/MessageReader.swift).
const READ_PIECE: usize = 256 * 1024;
const MOVE_INTERVAL: Duration = Duration::from_millis(8);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SourceChoice {
    Auto,
    Desktop,
    Window(u32),
    Keep,
}

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub hello: Hello,
    pub viewport: Option<Viewport>,
    pub source: SourceChoice,
    pub stats_device: String,
}

#[derive(Clone, Debug)]
pub enum SessionEvent {
    Status(String),
    Connected {
        mac_name: String,
        peer: SocketAddr,
        local_peer: bool,
    },
    WindowList(WindowList),
    AppList(Vec<AppInfo>),
    /// Kind 3 (blobs::decode_thumbnail): a window's picture, JPEG bytes not yet decoded.
    Thumbnail {
        id: u32,
        jpeg: Vec<u8>,
    },
    /// Kind 4 (blobs::decode_icon): an app's icon, PNG bytes not yet decoded.
    AppIcon {
        bundle_id: String,
        png: Vec<u8>,
    },
    /// Kind 16: the Mac's settings, on connect, on change, and as the answer to a change.
    HostSettings(HostSettingsState),
    CursorShape(CursorShape),
    MacInfo {
        info: MacInfo,
        fingerprint: [u8; 32],
    },
    Stats {
        rtt_ms: Option<f64>,
        fps: u32,
    },
    /// The session ended and the supervisor retries after `retry_in`. `text` is the CLI's line.
    Ended {
        text: String,
        retry_in: Option<Duration>,
        kind: EndKind,
    },
    /// The supervisor stopped for good. `text` is the CLI's line.
    Finished {
        text: String,
        kind: EndKind,
    },
}

/// Why a session ended, for a UI that words it itself (docs/ui-spec.md section 5.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndKind {
    /// Before admission: not found on the network, dial or TLS failed.
    ConnectFailed,
    /// An admitted session dropped without a goodbye.
    Lost,
    /// The Mac's goodbye. `reason` is the raw string (`goodbye_reason::*` or anything else).
    Goodbye {
        reason: String,
        message: Option<String>,
        minimum_version: Option<String>,
    },
    /// The Mac does not know this computer's key.
    NotPaired,
    /// The Mac presented a different key from the one pinned.
    PinMismatch,
}

impl EndKind {
    /// The kind for the Mac's goodbye.
    pub fn from_goodbye(g: &Goodbye) -> EndKind {
        EndKind::Goodbye {
            reason: g.reason.clone(),
            message: g.message.clone(),
            minimum_version: g.minimum_version.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum SessionCommand {
    Select(StreamSource),
    Viewport(Viewport),
    Input(InputEvent),
    LaunchApp(String),
    WindowCommand(WindowCommand),
    /// Kind 17. Dropped (not queued) while no session runs.
    ChangeSettings(HostSettingsChange),
    /// Ends a reconnect backoff wait now; ignored at any other time.
    RetryNow,
    Close,
}

#[derive(Debug)]
pub enum SessionEnd {
    Goodbye { goodbye: Goodbye, admitted: bool },
    Lost { error: NetError, admitted: bool },
    UserClosed,
}

pub struct Outputs {
    pub events: Sender<SessionEvent>,
    pub video: Sender<VideoPacket>,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

impl Outputs {
    /// Sends the event (ignoring a closed receiver), then wakes the consumer.
    pub fn emit(&self, e: SessionEvent) {
        let _ = self.events.send(e);
        (self.wake)();
    }
}

/// Reads one message: the 14-byte header, then the payload in pieces; `on_bytes` is called after
/// the header and after every piece (the liveness clock counts them all). A header announcing more
/// than `cap(kind)` bytes is a protocol error and nothing is allocated for it.
pub(crate) async fn read_message<R: AsyncRead + Unpin>(
    rd: &mut R,
    cap: impl Fn(u8) -> usize,
    mut on_bytes: impl FnMut(),
) -> Result<(Header, Vec<u8>), NetError> {
    let mut head = [0u8; HEADER_LEN];
    rd.read_exact(&mut head).await.map_err(classify_io_error)?;
    on_bytes();
    let header = Header::parse(&head);
    let len = header.len as usize;
    if len > cap(header.kind) {
        return Err(NetError::Protocol("oversized message".into()));
    }
    let mut payload: Vec<u8> = Vec::new();
    while payload.len() < len {
        let take = (len - payload.len()).min(READ_PIECE);
        let start = payload.len();
        payload.resize(start + take, 0);
        rd.read_exact(&mut payload[start..])
            .await
            .map_err(classify_io_error)?;
        on_bytes();
    }
    Ok((header, payload))
}

enum Inbound {
    Message(u8, Vec<u8>),
    Ended(NetError),
}

#[derive(Default)]
struct Samples {
    frame_age_ms: Vec<f64>,
    frames: u32,
}

struct AbortOnDrop(Vec<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for h in &self.0 {
            h.abort();
        }
    }
}

/// Hands video to the decoder without ever blocking the reader. While `waiting`, deltas are
/// dropped until a keyframe, which is preceded by the last parameter sets if the decoder has not
/// got them.
struct VideoForwarder {
    tx: Sender<VideoPacket>,
    waiting: bool,
    need_sets: bool,
    last_sets: Option<VideoPacket>,
}

impl VideoForwarder {
    fn new(tx: Sender<VideoPacket>) -> Self {
        let _ = tx.try_send(VideoPacket::Reset);
        VideoForwarder {
            tx,
            waiting: true,
            need_sets: true,
            last_sets: None,
        }
    }

    fn try_send(&self, p: VideoPacket) -> Result<(), TrySendError<VideoPacket>> {
        self.tx.try_send(p)
    }

    fn parameter_sets(&mut self, payload: &[u8]) {
        let Some(ps) = ParameterSets::decode(payload) else {
            return;
        };
        let packet = VideoPacket::ParameterSets {
            nal_length_size: ps.nal_length_size,
            sets: ps.sets,
        };
        self.last_sets = Some(packet.clone());
        if self.try_send(packet).is_ok() {
            self.need_sets = false;
        } else {
            self.waiting = true;
            self.need_sets = true;
        }
    }

    fn frame(&mut self, payload: Vec<u8>, is_keyframe: bool, timestamp: f64) {
        if self.waiting && !is_keyframe {
            return;
        }
        if self.waiting && self.need_sets {
            let Some(sets) = self.last_sets.clone() else {
                return;
            };
            if self.try_send(sets).is_err() {
                return;
            }
            self.need_sets = false;
        }
        let packet = VideoPacket::Frame {
            data: payload,
            is_keyframe,
            timestamp,
        };
        if self.try_send(packet).is_err() {
            self.waiting = true;
            self.need_sets = true;
        } else {
            self.waiting = false;
            self.need_sets = false;
        }
    }
}

async fn reader_task<R: AsyncRead + Unpin>(
    mut rd: R,
    inbound: mpsc::UnboundedSender<Inbound>,
    video: Sender<VideoPacket>,
    last_rx: Arc<AtomicU64>,
    samples: Arc<Mutex<Samples>>,
) {
    let mut fwd = VideoForwarder::new(video);
    loop {
        let (header, payload) = match read_message(&mut rd, host_payload_cap, || {
            last_rx.store(mono_seconds().to_bits(), Ordering::Relaxed);
        })
        .await
        {
            Ok(m) => m,
            Err(e) => {
                let _ = inbound.send(Inbound::Ended(e));
                return;
            }
        };
        match header.kind {
            kind::PARAMETER_SETS => fwd.parameter_sets(&payload),
            kind::FRAME => {
                {
                    let age =
                        ((now_timestamp() - header.timestamp) * 1000.0).clamp(0.0, 86_400_000.0);
                    let age = if age.is_finite() { age } else { 0.0 };
                    let mut s = samples.lock().unwrap_or_else(|e| e.into_inner());
                    s.frame_age_ms.push(age);
                    s.frames += 1;
                }
                fwd.frame(payload, header.is_keyframe, header.timestamp);
            }
            kind::WINDOW_LIST
            | kind::APP_LIST
            | kind::THUMBNAIL
            | kind::APP_ICON
            | kind::HOST_SETTINGS
            | kind::PONG
            | kind::CURSOR_SHAPE
            | kind::MAC_INFO
            | kind::GOODBYE => {
                let delivered = inbound.send(Inbound::Message(header.kind, payload)).is_ok();
                if !delivered {
                    return;
                }
            }
            _ => {}
        }
    }
}

async fn writer_task<W: tokio::io::AsyncWrite + Unpin>(
    mut wr: W,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(buf) = rx.recv().await {
        if buf.is_empty() {
            break;
        }
        if wr.write_all(&buf).await.is_err() || wr.flush().await.is_err() {
            return;
        }
    }
    let _ = wr.shutdown().await;
}

/// Lets the writer send what is queued (a closing button-up, for one) before the session is
/// dropped, for at most 250 ms.
async fn drain_writer(
    writer: &mut Option<tokio::task::JoinHandle<()>>,
    wtx: &mpsc::UnboundedSender<Vec<u8>>,
) {
    // An empty buffer tells the writer to finish after everything queued before it.
    let _ = wtx.send(Vec::new());
    if let Some(w) = writer.take() {
        let _ = tokio::time::timeout(Duration::from_millis(250), w).await;
    }
}

/// Accept a kind 18 only when it verifies and its key is this session's key.
pub fn accept_mac_info(
    signed: &SignedMacInfo,
    peer_fp: Option<[u8; 32]>,
) -> Option<(MacInfo, [u8; 32])> {
    let (info, fp) = signed.verified()?;
    if peer_fp == Some(fp) {
        Some((info, fp))
    } else {
        None
    }
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

fn max_of(v: &[f64]) -> Option<f64> {
    v.iter().copied().max_by(|a, b| a.total_cmp(b))
}

fn ms(v: Option<f64>) -> i64 {
    v.map(|x| x.round() as i64).unwrap_or(-1)
}

/// Runs one session on `io` until it ends. `opts.source` and `opts.viewport` are updated in
/// place by `Select` and `Viewport` commands, so a reconnect restores them.
pub async fn run_session(
    io: BoxIo,
    peer: SocketAddr,
    peer_fp: Option<[u8; 32]>,
    opts: &mut SessionOptions,
    cmds: &mut mpsc::UnboundedReceiver<SessionCommand>,
    out: &Outputs,
    on_mac_info: &mut (dyn FnMut(&MacInfo, [u8; 32]) + Send),
) -> SessionEnd {
    let (rd, wr) = tokio::io::split(io);
    let (wtx, wrx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (itx, mut irx) = mpsc::unbounded_channel::<Inbound>();
    let last_rx = Arc::new(AtomicU64::new(mono_seconds().to_bits()));
    let samples = Arc::new(Mutex::new(Samples::default()));

    let writer = tokio::spawn(writer_task(wr, wrx));
    let reader = tokio::spawn(reader_task(
        rd,
        itx,
        out.video.clone(),
        last_rx.clone(),
        samples.clone(),
    ));
    let _guard = AbortOnDrop(vec![writer.abort_handle(), reader.abort_handle()]);
    let mut writer = Some(writer);

    let send = |k: u8, payload: &[u8]| {
        let _ = wtx.send(encode_message(k, now_timestamp(), false, payload));
    };

    send(kind::HELLO, &to_json(&opts.hello));
    if let Some(v) = &opts.viewport {
        send(kind::VIEWPORT, &to_json(v));
    }

    let mut admitted = false;
    let mut goodbye: Option<Goodbye> = None;
    let mut prev_active: Option<StreamSource> = None;
    let mut listed: Vec<u32> = Vec::new();
    let mut last_auto_select: Option<Instant> = None;
    let mut rtt_samples: Vec<f64> = Vec::new();
    let mut recent_rtt: VecDeque<(f64, f64)> = VecDeque::new();
    let mut last_move: Option<Instant> = None;
    let mut pending_move: Option<InputEvent> = None;

    let mut ping_tick = tokio::time::interval(Duration::from_millis(250));
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut stats_tick = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    stats_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let flush_at = match (&pending_move, last_move) {
            (Some(_), Some(t)) => t + MOVE_INTERVAL,
            _ => Instant::now(),
        };
        tokio::select! {
            inbound = irx.recv() => {
                let Some(inbound) = inbound else {
                    return SessionEnd::Lost { error: NetError::Closed, admitted };
                };
                match inbound {
                    Inbound::Ended(err) => {
                        return match goodbye {
                            Some(goodbye) => SessionEnd::Goodbye { goodbye, admitted },
                            None => SessionEnd::Lost { error: err, admitted },
                        };
                    }
                    Inbound::Message(k, payload) => match k {
                        kind::WINDOW_LIST => {
                            let Some(list) = from_json::<WindowList>(&payload) else { continue };
                            let first = !admitted;
                            let active = list.active;
                            out.emit(SessionEvent::WindowList(list.clone()));
                            let mut auto = false;
                            if first {
                                admitted = true;
                                out.emit(SessionEvent::Connected {
                                    mac_name: list.mac_name.clone(),
                                    peer,
                                    local_peer: dial::is_local_address(&peer),
                                });
                                match opts.source {
                                    SourceChoice::Auto => {
                                        if matches!(active, StreamSource::None {}) {
                                            auto = true;
                                        }
                                    }
                                    SourceChoice::Desktop => {
                                        if !matches!(active, StreamSource::Desktop {}) {
                                            auto = true;
                                        }
                                    }
                                    SourceChoice::Window(id) => {
                                        if list.windows.iter().any(|w| w.id == id) {
                                            if active != (StreamSource::Window { id }) {
                                                send(
                                                    kind::SELECT_SOURCE,
                                                    &to_json(&StreamSource::Window { id }),
                                                );
                                                last_auto_select = Some(Instant::now());
                                            }
                                        } else {
                                            out.emit(SessionEvent::Status(format!(
                                                "window {id} is not open on the Mac; showing the desktop"
                                            )));
                                            auto = true;
                                        }
                                    }
                                    SourceChoice::Keep => {}
                                }
                            } else if matches!(active, StreamSource::None {}) {
                                let was_gone = match prev_active {
                                    Some(StreamSource::Desktop {}) => true,
                                    Some(StreamSource::Window { id }) => !listed.contains(&id),
                                    _ => false,
                                };
                                let quiet = last_auto_select
                                    .is_none_or(|t| t.elapsed() > Duration::from_secs(10));
                                if was_gone && quiet {
                                    auto = true;
                                }
                            }
                            if auto {
                                send(kind::SELECT_SOURCE, &to_json(&StreamSource::Desktop {}));
                                last_auto_select = Some(Instant::now());
                            }
                            prev_active = Some(active);
                            listed = list.windows.iter().map(|w| w.id).collect();
                        }
                        kind::APP_LIST => {
                            if let Some(apps) = from_json::<Vec<AppInfo>>(&payload) {
                                out.emit(SessionEvent::AppList(apps));
                            }
                        }
                        kind::THUMBNAIL => {
                            if let Some((id, jpeg)) = transom_proto::blobs::decode_thumbnail(&payload) {
                                out.emit(SessionEvent::Thumbnail { id, jpeg: jpeg.to_vec() });
                            } else {
                                log::debug!("bad thumbnail");
                            }
                        }
                        kind::APP_ICON => {
                            if let Some((bundle_id, png)) = transom_proto::blobs::decode_icon(&payload) {
                                if transom_proto::safetext::label(&bundle_id, 255) == bundle_id {
                                    out.emit(SessionEvent::AppIcon { bundle_id, png: png.to_vec() });
                                } else {
                                    log::debug!("app icon with an unsafe bundle id skipped");
                                }
                            } else {
                                log::debug!("bad app icon");
                            }
                        }
                        kind::HOST_SETTINGS => {
                            if let Some(state) = from_json::<HostSettingsState>(&payload) {
                                out.emit(SessionEvent::HostSettings(state));
                            } else {
                                log::debug!("bad host settings");
                            }
                        }
                        kind::PONG => {
                            if let Some(bytes) = payload.get(..8) {
                                let mut b = [0u8; 8];
                                b.copy_from_slice(bytes);
                                let sent = f64::from_bits(u64::from_be_bytes(b));
                                let rtt = (mono_seconds() - sent) * 1000.0;
                                if rtt.is_finite() && rtt >= 0.0 {
                                    rtt_samples.push(rtt);
                                    recent_rtt.push_back((mono_seconds(), rtt));
                                }
                            }
                        }
                        kind::CURSOR_SHAPE => {
                            if let Some(shape) = CursorShape::decode(&payload) {
                                out.emit(SessionEvent::CursorShape(shape));
                            }
                        }
                        kind::MAC_INFO => {
                            if let Some(signed) = from_json::<SignedMacInfo>(&payload) {
                                if let Some((info, fp)) = accept_mac_info(&signed, peer_fp) {
                                    on_mac_info(&info, fp);
                                    out.emit(SessionEvent::MacInfo { info, fingerprint: fp });
                                }
                            }
                        }
                        kind::GOODBYE => goodbye = Some(decode_goodbye(&payload)),
                        _ => {}
                    },
                }
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else {
                    drain_writer(&mut writer, &wtx).await;
                    return SessionEnd::UserClosed;
                };
                match cmd {
                    SessionCommand::Select(s) => {
                        match s {
                            StreamSource::Desktop {} => opts.source = SourceChoice::Desktop,
                            StreamSource::Window { id } => opts.source = SourceChoice::Window(id),
                            StreamSource::None {} => {}
                        }
                        send(kind::SELECT_SOURCE, &to_json(&s));
                    }
                    SessionCommand::Viewport(v) => {
                        send(kind::VIEWPORT, &to_json(&v));
                        opts.viewport = Some(v);
                    }
                    SessionCommand::Input(ev) => {
                        let is_move = matches!(
                            ev,
                            InputEvent::Pointer { action: PointerAction::Move, .. }
                        );
                        if is_move {
                            let now = Instant::now();
                            if last_move.is_none_or(|t| now.duration_since(t) >= MOVE_INTERVAL) {
                                pending_move = None;
                                send(kind::INPUT, &to_json(&ev));
                                last_move = Some(now);
                            } else {
                                pending_move = Some(ev);
                            }
                        } else {
                            if let Some(p) = pending_move.take() {
                                send(kind::INPUT, &to_json(&p));
                                last_move = Some(Instant::now());
                            }
                            send(kind::INPUT, &to_json(&ev));
                        }
                    }
                    SessionCommand::LaunchApp(bundle_id) => {
                        send(
                            kind::LAUNCH_APP,
                            &to_json(&transom_proto::messages::LaunchApp { bundle_id }),
                        );
                    }
                    SessionCommand::WindowCommand(c) => send(kind::WINDOW_COMMAND, &to_json(&c)),
                    SessionCommand::ChangeSettings(c) => send(kind::CHANGE_SETTINGS, &to_json(&c)),
                    // Only meaningful while idle (supervisor).
                    SessionCommand::RetryNow => {}
                    SessionCommand::Close => {
                        drain_writer(&mut writer, &wtx).await;
                        return SessionEnd::UserClosed;
                    }
                }
            }
            _ = tokio::time::sleep_until(flush_at), if pending_move.is_some() => {
                if let Some(p) = pending_move.take() {
                    send(kind::INPUT, &to_json(&p));
                    last_move = Some(Instant::now());
                }
            }
            _ = ping_tick.tick() => {
                let now = mono_seconds();
                send(kind::PING, &now.to_bits().to_be_bytes());
                while recent_rtt.front().is_some_and(|(t, _)| now - t > 1.0) {
                    recent_rtt.pop_front();
                }
                let worst_s = recent_rtt.iter().map(|(_, r)| *r).fold(0.0, f64::max) / 1000.0;
                let last = f64::from_bits(last_rx.load(Ordering::Relaxed));
                if now - last > (6.0f64).max(4.0 * worst_s) {
                    return SessionEnd::Lost { error: NetError::Timeout, admitted };
                }
            }
            _ = stats_tick.tick(), if admitted => {
                let (mut ages, fps) = {
                    let mut s = samples.lock().unwrap_or_else(|e| e.into_inner());
                    (std::mem::take(&mut s.frame_age_ms), std::mem::take(&mut s.frames))
                };
                let age_max = max_of(&ages);
                let age_med = median(&mut ages);
                let rtt_max = max_of(&rtt_samples);
                let rtt_med = median(&mut rtt_samples);
                rtt_samples.clear();
                let stats = ClientStats {
                    fps: i64::from(fps),
                    frame_age_ms: ms(age_med),
                    rtt_ms: ms(rtt_med),
                    device: opts.stats_device.clone(),
                    frame_age_max_ms: Some(ms(age_max)),
                    rtt_max_ms: Some(ms(rtt_max)),
                };
                send(kind::CLIENT_STATS, &to_json(&stats));
                out.emit(SessionEvent::Stats { rtt_ms: rtt_med, fps });
            }
        }
    }
}

//! What one stream knows about its Mac: a pure fold of `SessionEvent`s (docs/ui-spec.md section
//! 8.4). One per stream: the CLI viewer takes its title and window cycling from it, and the GUI
//! model holds one in its stream view. No I/O, no window, no clock but the `now` it is given.
//!
//! Strings from the Mac are stored as they came; a UI passes them through `safetext::label` when
//! it shows them.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transom_net::session::{EndKind, SessionEvent};
use transom_proto::host_settings::HostSettingsState;
use transom_proto::messages::{AppInfo, MacInfo, StreamSource, WindowInfo};

/// At most this many app icons are kept; later new bundle IDs are dropped (about 100 arrive at
/// connect).
pub const MAX_ICONS: usize = 512;
/// At most this many bytes of icon PNGs are kept in total.
pub const MAX_ICON_BYTES: usize = 32 << 20;

/// A window's picture: JPEG bytes, and a number that changes whenever the picture does.
#[derive(Clone, Debug)]
pub struct Thumb {
    pub jpeg: Arc<[u8]>,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub enum Phase {
    /// Dialing, or between a backoff and the next attempt. `SessionState::status` has the text.
    Connecting,
    /// Admitted: the first window list arrived.
    Connected,
    /// The session ended and the supervisor retries at `retry_at`. `text` is the supervisor's.
    Reconnecting {
        text: String,
        kind: EndKind,
        retry_at: Instant,
    },
    /// The supervisor stopped for good. `text` is the supervisor's.
    Finished { text: String, kind: EndKind },
}

#[derive(Clone, Debug)]
pub struct SessionState {
    pub phase: Phase,
    /// The name the route gave, replaced by the Mac's own at `Connected`.
    pub mac_name: String,
    /// The CLI's status line: the latest `Status`, or the end text with "Retrying in N s.".
    pub status: String,
    pub peer: Option<SocketAddr>,
    /// The Mac is this computer.
    pub local_peer: bool,
    pub windows: Vec<WindowInfo>,
    pub active: StreamSource,
    /// Kept across reconnects.
    pub apps: Vec<AppInfo>,
    /// By window ID; only windows in `windows`.
    pub thumbnails: HashMap<u32, Thumb>,
    /// By bundle ID; kept across reconnects.
    pub icons: HashMap<String, Arc<[u8]>>,
    pub host_settings: Option<HostSettingsState>,
    pub mac_info: Option<(MacInfo, [u8; 32])>,
    pub rtt_ms: Option<f64>,
    pub fps: u32,
    pub connected_at: Option<Instant>,
    pub first_frame_at: Option<Instant>,
    next_generation: u64,
    icon_bytes: usize,
}

impl SessionState {
    pub fn new(mac_name: &str) -> Self {
        SessionState {
            phase: Phase::Connecting,
            mac_name: mac_name.to_string(),
            status: String::new(),
            peer: None,
            local_peer: false,
            windows: Vec::new(),
            active: StreamSource::None {},
            apps: Vec::new(),
            thumbnails: HashMap::new(),
            icons: HashMap::new(),
            host_settings: None,
            mac_info: None,
            rtt_ms: None,
            fps: 0,
            connected_at: None,
            first_frame_at: None,
            next_generation: 1,
            icon_bytes: 0,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.phase, Phase::Connected)
    }

    /// Folds one event in.
    pub fn apply(&mut self, ev: SessionEvent, now: Instant) {
        match ev {
            SessionEvent::Status(s) => {
                self.status = s;
                if matches!(self.phase, Phase::Reconnecting { .. }) {
                    self.phase = Phase::Connecting;
                }
            }
            SessionEvent::Connected {
                mac_name,
                peer,
                local_peer,
            } => {
                if !mac_name.is_empty() {
                    self.mac_name = mac_name;
                }
                self.phase = Phase::Connected;
                self.peer = Some(peer);
                self.local_peer = local_peer;
                self.connected_at = Some(now);
                self.first_frame_at = None;
            }
            SessionEvent::WindowList(l) => {
                self.windows = l.windows;
                self.active = l.active;
                let ids: Vec<u32> = self.windows.iter().map(|w| w.id).collect();
                self.thumbnails.retain(|id, _| ids.contains(id));
            }
            SessionEvent::AppList(apps) => self.apps = apps,
            SessionEvent::Thumbnail { id, jpeg } => {
                if self.windows.iter().any(|w| w.id == id) {
                    let generation = self.next_generation;
                    self.next_generation += 1;
                    self.thumbnails.insert(
                        id,
                        Thumb {
                            jpeg: jpeg.into(),
                            generation,
                        },
                    );
                }
            }
            SessionEvent::AppIcon { bundle_id, png } => self.add_icon(bundle_id, png),
            SessionEvent::HostSettings(s) => {
                let old = self.host_settings.as_ref().and_then(|h| h.link.as_ref());
                if old != s.link.as_ref() {
                    match &s.link {
                        Some(l) => log::info!(
                            "link {}: {} frames withheld/s at {} bit/s per 60 fps, carried {:?} kbit/s, suggested {:?} bit/s at scale {:?}",
                            l.state,
                            l.withheld_per_second,
                            l.bitrate,
                            l.carried_kbps,
                            l.suggested_bitrate,
                            l.suggested_capture_scale
                        ),
                        None => log::info!("link keeping up"),
                    }
                }
                self.host_settings = Some(s);
            }
            SessionEvent::CursorShape(_) => {}
            SessionEvent::MacInfo { info, fingerprint } => {
                self.mac_info = Some((info, fingerprint));
            }
            SessionEvent::Stats { rtt_ms, fps } => {
                self.rtt_ms = rtt_ms;
                self.fps = fps;
            }
            SessionEvent::Ended {
                text,
                retry_in,
                kind,
            } => {
                self.status = match retry_in {
                    Some(d) => {
                        let secs = d.as_secs_f64().ceil() as u64;
                        format!("{text} Retrying in {secs} s.")
                    }
                    None => text.clone(),
                };
                self.phase = Phase::Reconnecting {
                    text,
                    kind,
                    retry_at: now + retry_in.unwrap_or(Duration::ZERO),
                };
                self.clear_connection();
            }
            SessionEvent::Finished { text, kind } => {
                self.status = text.clone();
                self.phase = Phase::Finished { text, kind };
                self.clear_connection();
            }
        }
    }

    /// The first decoded frame of this connection was shown.
    pub fn frame_shown(&mut self, now: Instant) {
        if self.is_connected() && self.first_frame_at.is_none() {
            self.first_frame_at = Some(now);
        }
    }

    fn add_icon(&mut self, bundle_id: String, png: Vec<u8>) {
        let old = self.icons.get(&bundle_id).map_or(0, |b| b.len());
        let is_new = old == 0 && !self.icons.contains_key(&bundle_id);
        if is_new && self.icons.len() >= MAX_ICONS {
            return;
        }
        if self.icon_bytes - old + png.len() > MAX_ICON_BYTES {
            return;
        }
        self.icon_bytes = self.icon_bytes - old + png.len();
        self.icons.insert(bundle_id, png.into());
    }

    fn clear_connection(&mut self) {
        self.windows.clear();
        self.active = StreamSource::None {};
        self.thumbnails.clear();
        self.host_settings = None;
        self.rtt_ms = None;
        self.fps = 0;
        self.connected_at = None;
        self.first_frame_at = None;
    }

    /// "Desktop", "‹App›: ‹Title›", "Window" or "nothing" (viewer.rs `source_text`).
    pub fn source_text(&self) -> String {
        match &self.active {
            StreamSource::Desktop {} => "Desktop".to_string(),
            StreamSource::Window { id } => self.windows.iter().find(|w| w.id == *id).map_or_else(
                || "Window".to_string(),
                |w| format!("{}: {}", w.app_name, w.title),
            ),
            StreamSource::None {} => "nothing".to_string(),
        }
    }

    /// The CLI viewer's window title (viewer.rs `update_title`).
    pub fn title_text(&self, view_only: bool) -> String {
        if self.is_connected() {
            let rtt = self
                .rtt_ms
                .map_or_else(|| "\u{2013}".to_string(), |r| format!("{r:.0}"));
            let view_only = if view_only { " (view only)" } else { "" };
            format!(
                "{} \u{2014} {} \u{2014} {} ms \u{b7} {} fps \u{2014} Transom{}",
                self.mac_name,
                self.source_text(),
                rtt,
                self.fps,
                view_only
            )
        } else {
            format!("{} \u{2014} Transom", self.status)
        }
    }

    /// The window after the active one, wrapping; the first when no window is active; None when
    /// the Mac lists no windows (viewer.rs `shortcut`, KeyW).
    pub fn next_window(&self) -> Option<u32> {
        if self.windows.is_empty() {
            return None;
        }
        let next = match &self.active {
            StreamSource::Window { id } => self
                .windows
                .iter()
                .position(|w| w.id == *id)
                .map_or(0, |i| (i + 1) % self.windows.len()),
            _ => 0,
        };
        Some(self.windows[next].id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_proto::messages::WindowList;

    fn win(id: u32, title: &str) -> WindowInfo {
        WindowInfo {
            id,
            title: title.to_string(),
            app_name: "Mock App".to_string(),
            bundle_id: "com.example.mock".to_string(),
            width: 800,
            height: 600,
        }
    }

    fn list(windows: Vec<WindowInfo>, active: StreamSource) -> SessionEvent {
        SessionEvent::WindowList(WindowList {
            mac_name: "Mock Mac".to_string(),
            windows,
            active,
            launch_id: None,
            host_version: None,
            protocol: None,
            gestures: None,
            clipboard: None,
        })
    }

    fn connected() -> SessionEvent {
        SessionEvent::Connected {
            mac_name: "Mock Mac".to_string(),
            peer: "127.0.0.1:5000".parse().unwrap(),
            local_peer: true,
        }
    }

    fn connected_state(now: Instant) -> SessionState {
        let mut s = SessionState::new("Studio");
        s.apply(
            list(
                vec![win(101, "First"), win(102, "Second")],
                StreamSource::Window { id: 101 },
            ),
            now,
        );
        s.apply(connected(), now);
        s
    }

    #[test]
    fn starts_connecting() {
        let s = SessionState::new("Studio");
        assert!(matches!(s.phase, Phase::Connecting));
        assert_eq!(s.mac_name, "Studio");
        assert_eq!(s.title_text(false), " \u{2014} Transom");
    }

    #[test]
    fn status_sets_text_and_ends_reconnecting() {
        let now = Instant::now();
        let mut s = connected_state(now);
        s.apply(
            SessionEvent::Ended {
                text: "Gone.".into(),
                retry_in: Some(Duration::from_secs(2)),
                kind: EndKind::Lost,
            },
            now,
        );
        assert!(matches!(s.phase, Phase::Reconnecting { .. }));
        s.apply(
            SessionEvent::Status("Connecting to Studio\u{2026}".into()),
            now,
        );
        assert!(matches!(s.phase, Phase::Connecting));
        assert_eq!(s.status, "Connecting to Studio\u{2026}");
    }

    #[test]
    fn connected_takes_name_peer_and_time() {
        let now = Instant::now();
        let s = connected_state(now);
        assert!(s.is_connected());
        assert_eq!(s.mac_name, "Mock Mac");
        assert!(s.local_peer);
        assert_eq!(s.connected_at, Some(now));
        assert_eq!(s.windows.len(), 2);
    }

    #[test]
    fn thumbnails_only_for_listed_windows_and_dropped_when_gone() {
        let now = Instant::now();
        let mut s = connected_state(now);
        s.apply(
            SessionEvent::Thumbnail {
                id: 101,
                jpeg: vec![1, 2, 3],
            },
            now,
        );
        s.apply(
            SessionEvent::Thumbnail {
                id: 999,
                jpeg: vec![1],
            },
            now,
        );
        assert_eq!(s.thumbnails.len(), 1);
        let g1 = s.thumbnails[&101].generation;
        s.apply(
            SessionEvent::Thumbnail {
                id: 101,
                jpeg: vec![4],
            },
            now,
        );
        assert!(s.thumbnails[&101].generation > g1);
        s.apply(
            list(vec![win(102, "Second")], StreamSource::Desktop {}),
            now,
        );
        assert!(s.thumbnails.is_empty());
    }

    #[test]
    fn icons_are_capped() {
        let now = Instant::now();
        let mut s = SessionState::new("Studio");
        for i in 0..(MAX_ICONS + 5) {
            s.apply(
                SessionEvent::AppIcon {
                    bundle_id: format!("app.{i}"),
                    png: vec![0; 4],
                },
                now,
            );
        }
        assert_eq!(s.icons.len(), MAX_ICONS);
        s.apply(
            SessionEvent::AppIcon {
                bundle_id: "app.0".into(),
                png: vec![0; 8],
            },
            now,
        );
        assert_eq!(s.icons["app.0"].len(), 8);
        s.apply(
            SessionEvent::AppIcon {
                bundle_id: "app.1".into(),
                png: vec![0; MAX_ICON_BYTES],
            },
            now,
        );
        assert_eq!(s.icons["app.1"].len(), 4);
    }

    #[test]
    fn ended_clears_connection_and_keeps_icons_and_apps() {
        let now = Instant::now();
        let mut s = connected_state(now);
        s.apply(
            SessionEvent::AppList(vec![AppInfo {
                name: "Notes".into(),
                bundle_id: "com.apple.Notes".into(),
            }]),
            now,
        );
        s.apply(
            SessionEvent::AppIcon {
                bundle_id: "com.apple.Notes".into(),
                png: vec![1],
            },
            now,
        );
        s.apply(
            SessionEvent::Thumbnail {
                id: 101,
                jpeg: vec![1],
            },
            now,
        );
        s.apply(
            SessionEvent::Stats {
                rtt_ms: Some(12.0),
                fps: 60,
            },
            now,
        );
        s.frame_shown(now);
        s.apply(
            SessionEvent::Ended {
                text: "Mock Mac quit Sill.".into(),
                retry_in: Some(Duration::from_millis(1500)),
                kind: EndKind::Goodbye {
                    reason: "quit".into(),
                    message: None,
                    minimum_version: None,
                },
            },
            now,
        );
        match &s.phase {
            Phase::Reconnecting { retry_at, kind, .. } => {
                assert_eq!(*retry_at, now + Duration::from_millis(1500));
                assert!(matches!(kind, EndKind::Goodbye { reason, .. } if reason == "quit"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(s.status, "Mock Mac quit Sill. Retrying in 2 s.");
        assert!(s.windows.is_empty());
        assert!(s.thumbnails.is_empty());
        assert!(s.host_settings.is_none());
        assert_eq!(s.first_frame_at, None);
        assert_eq!(s.rtt_ms, None);
        assert_eq!(s.apps.len(), 1);
        assert_eq!(s.icons.len(), 1);
    }

    #[test]
    fn finished_is_final() {
        let now = Instant::now();
        let mut s = connected_state(now);
        s.apply(
            SessionEvent::Finished {
                text: "Mock Mac removed this computer.".into(),
                kind: EndKind::NotPaired,
            },
            now,
        );
        assert!(matches!(
            s.phase,
            Phase::Finished {
                kind: EndKind::NotPaired,
                ..
            }
        ));
        assert_eq!(s.status, "Mock Mac removed this computer.");
    }

    #[test]
    fn frame_shown_only_once_and_only_connected() {
        let now = Instant::now();
        let mut s = SessionState::new("Studio");
        s.frame_shown(now);
        assert_eq!(s.first_frame_at, None);
        s.apply(connected(), now);
        s.frame_shown(now);
        let later = now + Duration::from_secs(1);
        s.frame_shown(later);
        assert_eq!(s.first_frame_at, Some(now));
    }

    #[test]
    fn titles_match_the_cli_viewer() {
        let now = Instant::now();
        let mut s = connected_state(now);
        assert_eq!(
            s.title_text(true),
            "Mock Mac \u{2014} Mock App: First \u{2014} \u{2013} ms \u{b7} 0 fps \u{2014} Transom (view only)"
        );
        s.apply(
            SessionEvent::Stats {
                rtt_ms: Some(17.6),
                fps: 60,
            },
            now,
        );
        s.apply(list(vec![win(101, "First")], StreamSource::Desktop {}), now);
        assert_eq!(
            s.title_text(false),
            "Mock Mac \u{2014} Desktop \u{2014} 18 ms \u{b7} 60 fps \u{2014} Transom"
        );
    }

    #[test]
    fn next_window_cycles() {
        let now = Instant::now();
        let mut s = connected_state(now);
        assert_eq!(s.next_window(), Some(102));
        s.apply(
            list(
                vec![win(101, "First"), win(102, "Second")],
                StreamSource::Window { id: 102 },
            ),
            now,
        );
        assert_eq!(s.next_window(), Some(101));
        s.apply(
            list(
                vec![win(101, "First"), win(102, "Second")],
                StreamSource::Desktop {},
            ),
            now,
        );
        assert_eq!(s.next_window(), Some(101));
        s.apply(list(vec![], StreamSource::Desktop {}), now);
        assert_eq!(s.next_window(), None);
    }
}

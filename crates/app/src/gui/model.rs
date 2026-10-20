//! The graphical app's state: a pure `Model` changed only by `Model::update(msg, now)`, which
//! returns the `Effect`s for the effects runner (docs/ui-spec.md section 8.2). No egui, no
//! winit, no I/O, no clock but the `now` it is given.
//!
//! The types in this file are frozen by the Phase 1 brief (wave A). Wave B fills in
//! `Model::update` and `Model::next_deadline` (WP-B3) and must not change a type, a field or a
//! signature here; a package that needs one stops and reports.

use crate::session_state::{Phase, SessionState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use transom_net::discovery::{BrowseEvent, FoundMac};
use transom_net::pairing::Paired;
use transom_net::session::{SessionCommand, SessionEvent};
use transom_net::store::SavedMac;
use transom_net::supervisor::Route;
use transom_net::NetError;
use transom_proto::messages::PairResult;
use transom_proto::pairlink::PairLink;

/// The bar stays shown this long after the first frame (spec 3.6).
pub const BAR_REVEAL: Duration = Duration::from_secs(3);
/// Hovering the handle this long shows the bar (spec 3.6).
pub const BAR_HOVER_DELAY: Duration = Duration::from_millis(250);
/// The bar hides this long after the pointer leaves it (spec 3.6).
pub const BAR_HIDE_DELAY: Duration = Duration::from_millis(1000);
/// "Paired with ‹Mac›." shows this long before connecting (spec 3.4, 3.5).
pub const PAIRED_PAUSE: Duration = Duration::from_millis(800);
/// The waiting card (CONN2) shows when no frame came this long after connecting (spec 3.6).
pub const WAITING_AFTER: Duration = Duration::from_millis(1500);
/// LIST3 shows when no Mac was found this long after start-up (spec 3.2).
pub const HINT_AFTER: Duration = Duration::from_secs(5);
/// A toast's lifetime (spec 3.6).
pub const TOAST_TIME: Duration = Duration::from_secs(4);
/// SET25 shows when no settings came this long after connecting (spec 3.8; Phase 2 panel).
pub const SETTINGS_WAIT: Duration = Duration::from_secs(2);
/// The UI zoom's range (spec 6.1).
pub const ZOOM_MIN: f32 = 0.75;
pub const ZOOM_MAX: f32 = 2.0;

#[derive(Clone, Debug)]
pub struct Model {
    pub screen: Screen,
    pub saved: Vec<SavedMac>,
    pub found: Vec<FoundMac>,
    /// Recomputed by `refresh_rows` whenever saved, found or prefs change (macs::rows).
    pub rows_mine: Vec<MacRow>,
    pub rows_other: Vec<MacRow>,
    pub prefs: Prefs,
    pub identity: IdentityInfo,
    pub banner: Option<Banner>,
    pub dialog: Option<Dialog>,
    /// The Macs screen's status line: empty, or a result line (LIST24, PAIR8). The view shows
    /// LIST2 while it is empty and `ever_found` is false.
    pub status: String,
    /// A Mac was found at least once since start-up.
    pub ever_found: bool,
    pub started_at: Instant,
    pub toasts: Vec<Toast>,
    /// The window is full screen (as the model last asked, or as the shell reported).
    pub fullscreen: bool,
    /// Bumped with `draft_seed` whenever the model wants text fields reset or filled
    /// (`seed_drafts`); `view::apply_seed` copies the seed into `Drafts` once per epoch.
    pub draft_epoch: u64,
    pub draft_seed: DraftSeed,
    /// The generation number the next card gets.
    pub next_gen: u64,
}

/// Text field contents the model wants the view to put in its drafts. None leaves a field as
/// it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftSeed {
    pub code: Option<String>,
    pub address: Option<String>,
    pub link: Option<String>,
    pub remote_host: Option<String>,
    pub device_name: Option<String>,
    pub hello_version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowWord {
    Home,
    Remote,
    NotFound,
    NotPaired,
    UpdateSill,
    Unencrypted,
}

/// One row of the Macs screen (spec 3.3), computed by `macs::rows`.
#[derive(Clone, Debug)]
pub struct MacRow {
    pub key: RowKey,
    /// Safe-text filtered.
    pub name: String,
    pub word: RowWord,
    /// The second line: an IP, `host:port · via`, or empty.
    pub detail: String,
    pub saved: Option<SavedMac>,
    pub found: Option<FoundMac>,
    /// A remote address exists (spec 3.9), so "Connect Remotely" is offered.
    pub has_remote: bool,
}

/// One per app, so the variants' sizes do not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Screen {
    Macs { card: Option<Card> },
    Stream(StreamView),
}

#[derive(Clone, Debug)]
pub enum Card {
    Pair(PairCard),
    Add(AddCard),
    /// An unsaved Mac with an open door: [Pair], [Connect without pairing] (DOOR1), [Cancel].
    OpenDoor {
        found: FoundMac,
    },
}

/// Pairing at home (spec 3.4).
#[derive(Clone, Debug)]
pub struct PairCard {
    /// The found Mac's name (safe-text filtered by discovery).
    pub mac_name: String,
    pub addrs: Vec<SocketAddr>,
    pub state: PairState,
    pub gen: u64,
    /// The fingerprint from the ask; the code is sent with this key pinned.
    pub pin: Option<[u8; 32]>,
    /// The ask was answered "shown" (PAIR3), not "openOnMac" (PAIR4). Cancel then closes the
    /// Mac's pairing window.
    pub shown: bool,
}

#[derive(Clone, Debug)]
pub enum PairState {
    Asking,
    /// Typing the code. `problem` is the pairing card's inline text (PAIR5, PAIR6, PAIR9); the
    /// Add card keeps its problems in `AddCard::problem` and leaves this None.
    Code {
        problem: Option<String>,
    },
    Working,
    /// Saved; connect at `at + PAIRED_PAUSE`.
    Paired {
        mac: SavedMac,
        at: Instant,
    },
    /// [Try Again] [Close].
    Failed {
        text: String,
    },
}

/// Add a Mac (spec 3.5).
#[derive(Clone, Debug)]
pub struct AddCard {
    pub link_mode: bool,
    pub problem: Option<(AddField, String)>,
    /// A parsed link waiting for its confirmation (ADD10).
    pub pending_link: Option<PairLink>,
    /// `Code { problem: None }` while typing, then Working, Paired or Failed. The Add card pairs
    /// with any key; the Mac's proof of the code checks the key (spec 3.5).
    pub state: PairState,
    pub gen: u64,
    /// The address as submitted (trimmed), remembered as the Mac's remote address on success.
    pub typed_address: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddField {
    Address,
    Code,
    Link,
    Card,
}

#[derive(Clone, Debug)]
pub struct StreamView {
    /// The saved Mac's ID; None for an unsaved Mac.
    pub mac_id: Option<String>,
    pub route_desc: RouteDesc,
    pub session: SessionState,
    pub bar: BarState,
    pub panel: Option<StreamPanel>,
    /// BAR12 was shown in this session.
    pub window_toast_shown: bool,
}

/// How the stream was reached, for SET1-SET3 and BAR5. The address is `session.peer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteDesc {
    Home,
    /// `via` of the first remote candidate; None for a manual address.
    Remote {
        via: Option<String>,
    },
    Unpinned,
    Plain,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarState {
    pub pinned: bool,
    /// The pointer has been over the handle area or the shown bar since then.
    pub hover_since: Option<Instant>,
    /// The pointer left the shown bar then.
    pub left_at: Option<Instant>,
    /// Shown until then (set to first frame + `BAR_REVEAL`).
    pub reveal_until: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamPanel {
    MacSettings,
    /// Phase 2.
    Apps,
    /// Phase 2.
    ThumbMenu {
        id: u32,
    },
}

#[derive(Clone, Debug)]
pub enum Dialog {
    Preferences {
        tab: PrefsTab,
    },
    MacDetails {
        mac_id: String,
        /// Inline text under the remote address field (ADD12).
        problem: Option<String>,
    },
    ConfirmForget {
        mac_id: String,
        then_pair: bool,
    },
    ConfirmPlain {
        found: FoundMac,
    },
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefsTab {
    General,
    Input,
    Streaming,
    Advanced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub text: String,
    pub actions: Vec<BannerAction>,
    pub mac_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerAction {
    PairAgain,
    Pair,
    ConnectAtHome,
    ForgetAndPair,
    GetSill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub until: Instant,
    pub action: Option<ToastAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// Phase 2.
    LowerQuality,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityInfo {
    pub fingerprint_b64: String,
    pub fingerprint_hex: String,
    pub config_dir: String,
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// A deadline from `next_deadline` passed.
    Tick,
    Browse(BrowseEvent),
    Session(SessionEvent),
    /// A decoded frame was drawn.
    FrameShown,
    /// The store's Macs, re-read by the effects runner after every SaveMac or Forget, and after
    /// every `SessionEvent::MacInfo` (the supervisor rewrites macs.json then).
    SavedMacs(Vec<SavedMac>),
    ClickRow {
        key: RowKey,
    },
    RowMenu {
        key: RowKey,
        item: RowMenuItem,
    },
    /// [Pair] on the open-door card.
    PairOpenDoor,
    OpenAdd,
    /// Cancel or Close on any card (and Esc).
    CancelCard,
    /// [Try Again] on a failed card.
    RetryCard,
    SubmitCode(String),
    SubmitAddress {
        address: String,
        code: String,
    },
    SubmitLink(String),
    ConfirmLink,
    ToggleLinkMode,
    PairAsked(u64, Result<(PairResult, [u8; 32]), PairFailure>),
    PairDone(u64, Result<Paired, PairFailure>),
    Connect(ConnectPlan),
    Disconnect,
    RetryNow,
    SelectDesktop,
    SelectWindow(u32),
    NextWindow,
    /// The handle, or chord+B.
    ToggleBar,
    /// The pointer is (true) or is no longer (false) over the handle area or the shown bar.
    BarHover(bool),
    OpenPanel(StreamPanel),
    ClosePanel,
    ToggleFullscreen,
    /// The shell saw the window enter or leave full screen.
    FullscreenChanged(bool),
    OpenDialog(Dialog),
    CloseDialog,
    ConfirmDialog,
    BannerAction(BannerAction),
    DismissBanner,
    SetPrefs(Prefs),
    SetRemoteHost {
        mac_id: String,
        host: Option<String>,
    },
    Copy(String),
    OpenUrl(String),
    ToastAction(ToastAction),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKey {
    /// A saved Mac, by Mac ID.
    Saved(String),
    /// A found, unsaved Mac, by Bonjour fullname.
    Found(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowMenuItem {
    Connect,
    ConnectHome,
    ConnectRemote,
    Details,
    Forget,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PairFailure {
    Refused {
        reason: String,
        tries_left: Option<i64>,
        retry_after: Option<f64>,
        message: Option<String>,
    },
    /// Timeout, connect failure, closed connection or any other error while pairing (PAIR16).
    /// The text is for the log only, never shown.
    Net(String),
    /// The Mac's proof of the code was bad (PAIR15).
    BadProof,
}

impl PairFailure {
    pub fn from_net(e: &NetError) -> PairFailure {
        match e {
            NetError::PairRefused {
                reason,
                tries_left,
                retry_after,
                message,
            } => PairFailure::Refused {
                reason: reason.clone(),
                tries_left: *tries_left,
                retry_after: *retry_after,
                message: message.clone(),
            },
            NetError::BadMacProof => PairFailure::BadProof,
            other => PairFailure::Net(other.to_string()),
        }
    }
}

/// What the effects runner does. Never log an `Effect` or `Msg` with `{:?}`: codes and link
/// secrets are inside.
#[derive(Clone, Debug)]
pub enum Effect {
    /// Ask at a home door (HOME_DIAL_TIMEOUT, any key); answer `Msg::PairAsked`.
    Ask {
        gen: u64,
        addrs: Vec<SocketAddr>,
    },
    /// Pair with a code; pin None = any key; `remote` picks the dial timeout. Answer
    /// `Msg::PairDone`.
    PairCode {
        gen: u64,
        addrs: Vec<SocketAddr>,
        pin: Option<[u8; 32]>,
        code: String,
        remote: bool,
    },
    /// The Add card: the runner resolves `host:port` itself (dial::resolve), then pairs as
    /// `PairCode` with `remote: true` and no pin. Answer `Msg::PairDone`.
    PairCodeAt {
        gen: u64,
        host: String,
        port: u16,
        code: String,
    },
    /// Pair with a confirmed link (pairflow::link_candidates, then pairflow::pair_link).
    PairLink {
        gen: u64,
        link: PairLink,
    },
    /// Best effort; no answer.
    CancelAsk {
        addrs: Vec<SocketAddr>,
        pin: [u8; 32],
    },
    /// Then `Msg::SavedMacs`.
    SaveMac(SavedMac),
    /// By Mac ID. Then `Msg::SavedMacs`.
    Forget(String),
    /// Writes prefs.json, and applies the input options to a running stream at once.
    SavePrefs(Prefs),
    StartStream {
        route: Route,
        mac_name: String,
    },
    StopStream,
    Session(SessionCommand),
    SetFullscreen(bool),
    ReleaseAll,
    CopyText(String),
    OpenUrl(String),
}

#[derive(Clone, Debug)]
pub enum ConnectPlan {
    Home(SavedMac),
    Remote {
        mac: SavedMac,
        hosts: Vec<(String, u16)>,
    },
    Unpinned(FoundMac),
    Plain(FoundMac),
}

/// prefs.json (spec 8.6). Every field defaults; unknown fields are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub version: i64,
    pub device_name: Option<String>,
    /// "pc" or "mac"; None = the OS default.
    pub keymap: Option<String>,
    pub allow_local_pointer: bool,
    pub allow_local_keyboard: bool,
    pub fps: i64,
    pub show_stats: bool,
    pub allow_plain: bool,
    pub hello_version: Option<String>,
    pub log_file: bool,
    pub log_debug: bool,
    pub ui_zoom: f32,
    /// By Mac ID.
    pub macs: BTreeMap<String, MacPrefs>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            version: 1,
            device_name: None,
            keymap: None,
            allow_local_pointer: false,
            allow_local_keyboard: false,
            fps: 60,
            show_stats: true,
            allow_plain: false,
            hello_version: None,
            log_file: false,
            log_debug: false,
            ui_zoom: 1.0,
            macs: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MacPrefs {
    /// The manually entered remote address, `host` or `host:port`.
    pub remote_host: Option<String>,
}

impl StreamView {
    pub fn new(mac_id: Option<String>, route_desc: RouteDesc, mac_name: &str) -> StreamView {
        StreamView {
            mac_id,
            route_desc,
            session: SessionState::new(mac_name),
            bar: BarState::default(),
            panel: None,
            window_toast_shown: false,
        }
    }

    /// The bar is shown (spec 3.6): while not connected, while pinned, while a panel is open,
    /// until `reveal_until`, after `BAR_HOVER_DELAY` of hovering, and for `BAR_HIDE_DELAY` after
    /// the pointer left it.
    pub fn bar_visible(&self, now: Instant) -> bool {
        if !matches!(self.session.phase, Phase::Connected) {
            return true;
        }
        if self.bar.pinned || self.panel.is_some() {
            return true;
        }
        if self.bar.reveal_until.is_some_and(|t| now < t) {
            return true;
        }
        if self
            .bar
            .hover_since
            .is_some_and(|t| now >= t + BAR_HOVER_DELAY)
        {
            return true;
        }
        self.bar.left_at.is_some_and(|t| now < t + BAR_HIDE_DELAY)
    }
}

impl Model {
    pub fn new(identity: IdentityInfo, prefs: Prefs, saved: Vec<SavedMac>, now: Instant) -> Model {
        let mut m = Model {
            screen: Screen::Macs { card: None },
            saved,
            found: Vec::new(),
            rows_mine: Vec::new(),
            rows_other: Vec::new(),
            prefs,
            identity,
            banner: None,
            dialog: None,
            status: String::new(),
            ever_found: false,
            started_at: now,
            toasts: Vec::new(),
            fullscreen: false,
            draft_epoch: 0,
            draft_seed: DraftSeed::default(),
            next_gen: 1,
        };
        m.refresh_rows();
        m
    }

    /// Recomputes `rows_mine` and `rows_other` (macs::rows).
    pub fn refresh_rows(&mut self) {
        let (mine, other) = crate::gui::macs::rows(&self.saved, &self.found, &self.prefs);
        self.rows_mine = mine;
        self.rows_other = other;
    }

    /// Asks the view to reset or fill its text fields (see `DraftSeed`).
    pub fn seed_drafts(&mut self, seed: DraftSeed) {
        self.draft_epoch += 1;
        self.draft_seed = seed;
    }

    /// A new card generation number.
    pub fn take_gen(&mut self) -> u64 {
        let g = self.next_gen;
        self.next_gen += 1;
        g
    }

    pub fn stream(&self) -> Option<&StreamView> {
        match &self.screen {
            Screen::Stream(s) => Some(s),
            Screen::Macs { .. } => None,
        }
    }

    pub fn stream_mut(&mut self) -> Option<&mut StreamView> {
        match &mut self.screen {
            Screen::Stream(s) => Some(s),
            Screen::Macs { .. } => None,
        }
    }

    /// A dialog, or a panel over the stream, is open: the keyboard belongs to Transom (spec 6.1).
    pub fn overlay_open(&self) -> bool {
        self.dialog.is_some() || self.stream().is_some_and(|s| s.panel.is_some())
    }

    /// Changes the model for `msg` and returns what the effects runner must do. WP-B3.
    pub fn update(&mut self, msg: Msg, now: Instant) -> Vec<Effect> {
        let _ = (msg, now);
        Vec::new()
    }

    /// The next time the model needs a `Msg::Tick` (bar timing, the countdown, the paired
    /// pause, the waiting card, LIST3, toasts). WP-B3.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let _ = now;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_default_and_tolerate_unknown_fields() {
        let p: Prefs = serde_json::from_str(r#"{"fps":120,"future":true}"#).unwrap();
        assert_eq!(p.fps, 120);
        assert_eq!(p.version, 1);
        assert!(p.show_stats);
        assert_eq!(p.ui_zoom, 1.0);
        let d: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(d, Prefs::default());
    }

    #[test]
    fn bar_visibility_rules() {
        let now = Instant::now();
        let mut v = StreamView::new(None, RouteDesc::Unpinned, "Mock Mac");
        assert!(v.bar_visible(now), "shown while not connected");
        v.session.apply(
            SessionEvent::Connected {
                mac_name: "Mock Mac".into(),
                peer: "127.0.0.1:1".parse().unwrap(),
                local_peer: false,
            },
            now,
        );
        assert!(!v.bar_visible(now));
        v.bar.reveal_until = Some(now + BAR_REVEAL);
        assert!(v.bar_visible(now));
        assert!(!v.bar_visible(now + BAR_REVEAL));
        v.bar.reveal_until = None;
        v.bar.hover_since = Some(now);
        assert!(!v.bar_visible(now + Duration::from_millis(100)));
        assert!(v.bar_visible(now + BAR_HOVER_DELAY));
        v.bar.hover_since = None;
        v.bar.left_at = Some(now);
        assert!(v.bar_visible(now + Duration::from_millis(900)));
        assert!(!v.bar_visible(now + BAR_HIDE_DELAY));
        v.bar.left_at = None;
        v.panel = Some(StreamPanel::MacSettings);
        assert!(v.bar_visible(now));
        v.panel = None;
        v.bar.pinned = true;
        assert!(v.bar_visible(now));
    }

    #[test]
    fn overlay_open_and_gens() {
        let now = Instant::now();
        let mut m = Model::new(IdentityInfo::default(), Prefs::default(), Vec::new(), now);
        assert!(!m.overlay_open());
        m.dialog = Some(Dialog::About);
        assert!(m.overlay_open());
        m.dialog = None;
        m.screen = Screen::Stream(StreamView::new(None, RouteDesc::Plain, "M"));
        assert!(!m.overlay_open());
        m.stream_mut().unwrap().panel = Some(StreamPanel::MacSettings);
        assert!(m.overlay_open());
        assert_eq!(m.take_gen(), 1);
        assert_eq!(m.take_gen(), 2);
        m.seed_drafts(DraftSeed {
            code: Some(String::new()),
            ..Default::default()
        });
        assert_eq!(m.draft_epoch, 1);
    }
}

//! The graphical app's state: a pure `Model` changed only by `Model::update(msg, now)`, which
//! returns the `Effect`s for the effects runner (docs/ui-spec.md section 8.2). No egui, no
//! winit, no I/O, no clock but the `now` it is given.
//!
//! The types in this file are frozen by the Phase 1 brief (wave A). Wave B fills in
//! `Model::update` and `Model::next_deadline` (WP-B3) and must not change a type, a field or a
//! signature here; a package that needs one stops and reports.

use crate::gui::pairing::{self, AskOutcome, PairOutcome};
use crate::gui::{macs, texts};
use crate::session_state::{Phase, SessionState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use transom_net::dial::REMOTE_DEFAULT_PORT;
use transom_net::discovery::{BrowseEvent, FoundMac};
use transom_net::pairing::Paired;
use transom_net::session::{EndKind, SessionCommand, SessionEvent};
use transom_net::store::SavedMac;
use transom_net::supervisor::Route;
use transom_net::NetError;
use transom_proto::address::parse_address;
use transom_proto::host_settings::{HostSettingsChange, SettingsField};
use transom_proto::messages::goodbye_reason as gr;
use transom_proto::messages::{PairResult, StreamSource};
use transom_proto::pairing::HomeDoor;
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
/// A row shows a spinner once its pick has been pending this long, so a quick Mac never
/// flickers (D12; Sill's PendingSpinner).
pub const SPINNER_DELAY: Duration = Duration::from_millis(300);
/// The tick for an expired pick lands this far past the timeout, so it is strictly past (D4).
pub const PICK_EXPIRY_SLACK: Duration = Duration::from_millis(50);
/// A report must be clear this long before the next "behind" counts as a new spell (D7; Sill's
/// `LinkLine.linger`).
pub const LINK_LINGER: Duration = Duration::from_secs(2);
/// BAR20 shows at most once in this time (D7).
pub const LINK_TOAST_GAP: Duration = Duration::from_secs(30);
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
    /// The next kind 17 token: strictly increasing for the life of the process, never reset
    /// (Sill's settingsToken).
    pub next_settings_token: i64,
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
    /// A "behind" spell is running: a state arrived whose link is behind, and it has not been
    /// clear for `LINK_LINGER` since (D7). Not reset on Ended or reconnect.
    pub link_spell: bool,
    /// When the running spell's report first turned clear.
    pub link_clear_since: Option<Instant>,
    /// When BAR20 last showed (D7).
    pub link_toast_at: Option<Instant>,
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
    /// A Mac Settings control (spec 3.8 B/C): the fields it sets, as absolute values; the token
    /// is ignored. Goes through the ledger; only what changes the shown value is sent.
    ChangeSettings(HostSettingsChange),
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
            link_spell: false,
            link_clear_since: None,
            link_toast_at: None,
        }
    }

    /// The row for `field` has a pick pending for `SPINNER_DELAY` or more (D12).
    pub fn pick_spinner(&self, field: SettingsField, now: Instant) -> bool {
        self.session
            .settings
            .pending_since(field)
            .is_some_and(|t| now >= t + SPINNER_DELAY)
    }

    /// No state has come `SETTINGS_WAIT` after connecting (SET25).
    pub fn settings_overdue(&self, now: Instant) -> bool {
        self.session.settings.host().is_none()
            && self.session.is_connected()
            && self
                .session
                .connected_at
                .is_some_and(|c| now >= c + SETTINGS_WAIT)
    }

    /// What [Lower Quality] would send, if the button applies now (D6).
    pub fn lower_quality_change(&self) -> Option<HostSettingsChange> {
        let h = self.session.settings.host()?;
        let link = h.link.as_ref().filter(|l| l.is_behind())?;
        if h.away
            .as_ref()
            .is_some_and(|a| a.this_connection_away && !a.away_running)
        {
            return None;
        }
        let c = link.suggestion()?;
        let shown = self.session.settings.displayed()?;
        (c.applied_to(&shown) != shown).then_some(c)
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
    pub fn new(
        identity: IdentityInfo,
        mut prefs: Prefs,
        saved: Vec<SavedMac>,
        now: Instant,
    ) -> Model {
        prefs.ui_zoom = if prefs.ui_zoom.is_finite() {
            prefs.ui_zoom.clamp(ZOOM_MIN, ZOOM_MAX)
        } else {
            1.0
        };
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
            next_settings_token: 1,
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

    /// Changes the model for `msg` and returns what the effects runner must do (spec 8.2).
    pub fn update(&mut self, msg: Msg, now: Instant) -> Vec<Effect> {
        match msg {
            Msg::Tick => self.on_tick(now),
            Msg::Browse(BrowseEvent::Found(f)) => {
                match self.found.iter_mut().find(|e| e.fullname == f.fullname) {
                    Some(e) => *e = f,
                    None => self.found.push(f),
                }
                self.ever_found = true;
                self.refresh_rows();
                Vec::new()
            }
            Msg::Browse(BrowseEvent::Removed { fullname }) => {
                self.found.retain(|e| e.fullname != fullname);
                self.refresh_rows();
                Vec::new()
            }
            Msg::SavedMacs(v) => {
                self.saved = v;
                self.refresh_rows();
                Vec::new()
            }
            Msg::FrameShown => {
                if let Some(s) = self.stream_mut() {
                    let first = s.session.first_frame_at.is_none();
                    s.session.frame_shown(now);
                    if first && s.session.first_frame_at.is_some() {
                        s.bar.reveal_until = Some(now + BAR_REVEAL);
                    }
                }
                Vec::new()
            }
            Msg::Session(ev) => self.on_session(ev, now),
            // Nothing may replace a card whose pairing is in flight, or its late success
            // would be dropped without saving the Mac.
            Msg::ClickRow { .. } | Msg::RowMenu { .. } | Msg::OpenAdd
                if self.pairing_in_flight() =>
            {
                Vec::new()
            }
            Msg::BannerAction(a) if a != BannerAction::GetSill && self.pairing_in_flight() => {
                Vec::new()
            }
            Msg::ClickRow { key } => self.click_row(&key, now),
            Msg::RowMenu { key, item } => self.row_menu(&key, item, now),
            Msg::PairOpenDoor => {
                if let Screen::Macs {
                    card: Some(Card::OpenDoor { found }),
                } = &self.screen
                {
                    let found = found.clone();
                    return self.start_pair(found);
                }
                Vec::new()
            }
            Msg::OpenAdd => {
                self.open_add();
                Vec::new()
            }
            Msg::CancelCard => {
                let mut out = Vec::new();
                if let Screen::Macs { card } = &mut self.screen {
                    // Pairing in flight cannot be cancelled: a late success must still save
                    // the Mac (the Pair and the Add card alike).
                    if card_is_working(card.as_ref()) {
                        return out;
                    }
                    if let Some(Card::Pair(p)) = card {
                        if let (PairState::Code { .. }, true, Some(pin)) =
                            (&p.state, p.shown, p.pin)
                        {
                            out.push(Effect::CancelAsk {
                                addrs: p.addrs.clone(),
                                pin,
                            });
                        }
                    }
                    *card = None;
                }
                out
            }
            Msg::RetryCard => self.retry_card(),
            Msg::PairAsked(gen, r) => {
                self.on_pair_asked(gen, &r);
                Vec::new()
            }
            Msg::SubmitCode(s) => self.submit_code(&s),
            Msg::SubmitAddress { address, code } => self.submit_address(&address, &code),
            Msg::SubmitLink(s) => {
                if let Some(a) = self.add_card_in_code() {
                    match PairLink::parse(&s) {
                        Err(_) => a.problem = Some((AddField::Link, texts::NOT_A_LINK.to_string())),
                        Ok(l) => {
                            a.pending_link = Some(l);
                            a.problem = None;
                        }
                    }
                }
                Vec::new()
            }
            Msg::ConfirmLink => {
                if let Some(a) = self.add_card_in_code() {
                    if let Some(l) = a.pending_link.clone() {
                        a.state = PairState::Working;
                        return vec![Effect::PairLink {
                            gen: a.gen,
                            link: l,
                        }];
                    }
                }
                Vec::new()
            }
            Msg::ToggleLinkMode => {
                if let Screen::Macs {
                    card: Some(Card::Add(a)),
                } = &mut self.screen
                {
                    a.link_mode = !a.link_mode;
                    a.problem = None;
                    a.pending_link = None;
                }
                Vec::new()
            }
            Msg::PairDone(gen, r) => self.on_pair_done(gen, &r, now),
            Msg::Connect(plan) => self.connect(plan),
            Msg::Disconnect => {
                if self.stream().is_some() {
                    self.screen = Screen::Macs { card: None };
                    return vec![Effect::StopStream];
                }
                Vec::new()
            }
            Msg::RetryNow => vec![Effect::Session(SessionCommand::RetryNow)],
            Msg::SelectDesktop => vec![Effect::Session(SessionCommand::Select(
                StreamSource::Desktop {},
            ))],
            Msg::SelectWindow(id) => self.select_window(id, now),
            Msg::NextWindow => match self.stream().and_then(|s| s.session.next_window()) {
                Some(id) => self.select_window(id, now),
                None => Vec::new(),
            },
            Msg::ToggleBar => {
                if let Some(s) = self.stream_mut() {
                    if s.bar.pinned {
                        s.bar.pinned = false;
                        s.bar.hover_since = None;
                        s.bar.left_at = None;
                        s.bar.reveal_until = None;
                    } else {
                        s.bar.pinned = true;
                    }
                }
                Vec::new()
            }
            Msg::BarHover(over) => {
                if let Some(s) = self.stream_mut() {
                    if over {
                        s.bar.hover_since.get_or_insert(now);
                        s.bar.left_at = None;
                    } else {
                        if s.bar_visible(now) {
                            s.bar.left_at = Some(now);
                        }
                        s.bar.hover_since = None;
                    }
                }
                Vec::new()
            }
            Msg::OpenPanel(p) => match self.stream_mut() {
                Some(s) => {
                    s.panel = Some(p);
                    vec![Effect::ReleaseAll]
                }
                None => Vec::new(),
            },
            Msg::ClosePanel => {
                if let Some(s) = self.stream_mut() {
                    s.panel = None;
                }
                Vec::new()
            }
            Msg::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                vec![Effect::SetFullscreen(self.fullscreen)]
            }
            Msg::FullscreenChanged(b) => {
                self.fullscreen = b;
                Vec::new()
            }
            Msg::OpenDialog(d) => self.open_dialog(d),
            Msg::CloseDialog => {
                self.dialog = None;
                Vec::new()
            }
            Msg::ConfirmDialog => self.confirm_dialog(now),
            Msg::BannerAction(a) => self.banner_action(a),
            Msg::DismissBanner => {
                self.banner = None;
                Vec::new()
            }
            Msg::SetPrefs(p) => self.set_prefs(p),
            Msg::SetRemoteHost { mac_id, host } => self.set_remote_host(mac_id, host),
            Msg::Copy(s) => vec![Effect::CopyText(s)],
            Msg::OpenUrl(u) => vec![Effect::OpenUrl(u)],
            Msg::ChangeSettings(c) => self.pick_settings(&c, now),
            Msg::ToastAction(ToastAction::LowerQuality) => self.lower_quality(now),
        }
    }

    fn on_tick(&mut self, now: Instant) -> Vec<Effect> {
        self.toasts.retain(|t| t.until > now);
        if let Some(s) = self.stream_mut() {
            let t = s.session.settings_timeout();
            if s.session.settings.expire(now, t) {
                log::info!("settings: no answer within {t:?}; showing the Mac's values");
            }
        }
        let plan = match &self.screen {
            Screen::Macs {
                card: Some(Card::Pair(p)),
            } => match &p.state {
                PairState::Paired { mac, at } if now >= *at + PAIRED_PAUSE => {
                    Some(ConnectPlan::Home(mac.clone()))
                }
                _ => None,
            },
            Screen::Macs {
                card: Some(Card::Add(a)),
            } => match &a.state {
                PairState::Paired { mac, at } if now >= *at + PAIRED_PAUSE => {
                    let home = macs::found_for(mac, &self.found).is_some();
                    let remote = macs::plan_remote(mac, &self.prefs);
                    Some(if a.link_mode && home {
                        ConnectPlan::Home(mac.clone())
                    } else {
                        remote.unwrap_or_else(|| ConnectPlan::Home(mac.clone()))
                    })
                }
                _ => None,
            },
            _ => None,
        };
        match plan {
            Some(p) => self.update(Msg::Connect(p), now),
            None => Vec::new(),
        }
    }

    fn on_session(&mut self, ev: SessionEvent, now: Instant) -> Vec<Effect> {
        let Some(s) = self.stream_mut() else {
            return Vec::new();
        };
        let fin = match &ev {
            SessionEvent::Finished { text, kind } => Some((text.clone(), kind.clone())),
            _ => None,
        };
        let refusals = s.session.settings_refusals;
        let is_settings = matches!(ev, SessionEvent::HostSettings(_));
        s.session.apply(ev, now);
        let refused = s.session.settings_refusals != refusals;
        let mut link_toast = None;
        if is_settings {
            let behind = s
                .session
                .settings
                .host()
                .and_then(|h| h.link.as_ref())
                .is_some_and(|l| l.is_behind());
            if behind {
                let new_spell =
                    !s.link_spell || s.link_clear_since.is_some_and(|t| now >= t + LINK_LINGER);
                s.link_clear_since = None;
                if new_spell {
                    s.link_spell = true;
                    if s.link_toast_at.is_none_or(|t| now >= t + LINK_TOAST_GAP) {
                        s.link_toast_at = Some(now);
                        link_toast =
                            Some(s.lower_quality_change().map(|_| ToastAction::LowerQuality));
                    }
                }
            } else if s.link_spell && s.link_clear_since.is_none() {
                s.link_clear_since = Some(now);
            }
        }
        let mac_name = s.session.mac_name.clone();
        let mac_id = s.mac_id.clone();
        if refused {
            self.show_toast(texts::kept_setting(&mac_name), None, now);
        }
        if let Some(action) = link_toast {
            self.show_toast(texts::link_behind(&mac_name), action, now);
        }
        let Some((text, kind)) = fin else {
            return Vec::new();
        };
        let found_home = mac_id
            .as_deref()
            .and_then(|id| self.saved.iter().find(|m| m.mac_id == id))
            .is_some_and(|m| macs::found_for(m, &self.found).is_some());
        let mut actions = match &kind {
            EndKind::NotPaired => vec![BannerAction::PairAgain],
            EndKind::PinMismatch => vec![BannerAction::ForgetAndPair],
            EndKind::Goodbye { reason, .. } => match reason.as_str() {
                gr::REMOVED => vec![BannerAction::PairAgain],
                gr::PAIRING_REQUIRED => vec![BannerAction::Pair],
                gr::UPDATE => vec![BannerAction::GetSill],
                gr::REMOTE_OFF if found_home => vec![BannerAction::ConnectAtHome],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        if mac_id.is_none() {
            actions.retain(|a| *a == BannerAction::GetSill);
        }
        self.banner = Some(Banner {
            text: texts::end_text(&kind, &mac_name, &text),
            actions,
            mac_id,
        });
        self.screen = Screen::Macs { card: None };
        self.dialog = None;
        vec![Effect::StopStream]
    }

    fn find_row(&self, key: &RowKey) -> Option<MacRow> {
        self.rows_mine
            .iter()
            .chain(self.rows_other.iter())
            .find(|r| &r.key == key)
            .cloned()
    }

    fn set_card(&mut self, c: Option<Card>) {
        if let Screen::Macs { card } = &mut self.screen {
            *card = c;
        }
    }

    fn click_row(&mut self, key: &RowKey, now: Instant) -> Vec<Effect> {
        let Some(row) = self.find_row(key) else {
            return Vec::new();
        };
        self.banner = None;
        self.status.clear();
        match row.word {
            RowWord::Home => match row.saved {
                Some(m) => self.update(Msg::Connect(ConnectPlan::Home(m)), now),
                None => Vec::new(),
            },
            RowWord::Remote => match row.saved.and_then(|m| macs::plan_remote(&m, &self.prefs)) {
                Some(p) => self.update(Msg::Connect(p), now),
                None => Vec::new(),
            },
            RowWord::NotFound => match row.saved {
                Some(m) => self.update(
                    Msg::OpenDialog(Dialog::MacDetails {
                        mac_id: m.mac_id,
                        problem: None,
                    }),
                    now,
                ),
                None => Vec::new(),
            },
            RowWord::NotPaired => match row.found {
                Some(f) if f.door == HomeDoor::Open => {
                    self.set_card(Some(Card::OpenDoor { found: f }));
                    Vec::new()
                }
                Some(f) => self.start_pair(f),
                None => Vec::new(),
            },
            RowWord::UpdateSill => Vec::new(),
            RowWord::Unencrypted => {
                if let Some(f) = row.found {
                    self.dialog = Some(Dialog::ConfirmPlain { found: f });
                }
                Vec::new()
            }
        }
    }

    fn row_menu(&mut self, key: &RowKey, item: RowMenuItem, now: Instant) -> Vec<Effect> {
        let Some(row) = self.find_row(key) else {
            return Vec::new();
        };
        match item {
            RowMenuItem::Connect => self.click_row(key, now),
            RowMenuItem::ConnectHome => match row.saved {
                Some(m) if row.found.is_some() => {
                    self.update(Msg::Connect(ConnectPlan::Home(m)), now)
                }
                _ => Vec::new(),
            },
            RowMenuItem::ConnectRemote => {
                match row.saved.and_then(|m| macs::plan_remote(&m, &self.prefs)) {
                    Some(p) => self.update(Msg::Connect(p), now),
                    None => Vec::new(),
                }
            }
            RowMenuItem::Details => match row.saved {
                Some(m) => self.update(
                    Msg::OpenDialog(Dialog::MacDetails {
                        mac_id: m.mac_id,
                        problem: None,
                    }),
                    now,
                ),
                None => Vec::new(),
            },
            RowMenuItem::Forget => {
                if let Some(m) = row.saved {
                    self.dialog = Some(Dialog::ConfirmForget {
                        mac_id: m.mac_id,
                        then_pair: false,
                    });
                }
                Vec::new()
            }
        }
    }

    /// Pairing at home: the card, then the ask.
    fn start_pair(&mut self, found: FoundMac) -> Vec<Effect> {
        let gen = self.take_gen();
        let addrs = found.addresses.clone();
        self.set_card(Some(Card::Pair(PairCard {
            mac_name: found.name,
            addrs: addrs.clone(),
            state: PairState::Asking,
            gen,
            pin: None,
            shown: false,
        })));
        self.seed_drafts(DraftSeed {
            code: Some(String::new()),
            ..Default::default()
        });
        vec![Effect::Ask { gen, addrs }]
    }

    /// Whether the Macs screen shows a Pair or Add card waiting for the Mac's answer.
    fn pairing_in_flight(&self) -> bool {
        matches!(&self.screen, Screen::Macs { card } if card_is_working(card.as_ref()))
    }

    fn open_add(&mut self) {
        let gen = self.take_gen();
        self.set_card(Some(Card::Add(AddCard {
            link_mode: false,
            problem: None,
            pending_link: None,
            state: PairState::Code { problem: None },
            gen,
            typed_address: None,
        })));
        self.seed_drafts(DraftSeed {
            code: Some(String::new()),
            address: Some(String::new()),
            link: Some(String::new()),
            ..Default::default()
        });
        self.banner = None;
    }

    /// Pairing a saved Mac again: at home when it is found, else the Add card with its first
    /// remote address filled in.
    fn pair_again(&mut self, found: Option<FoundMac>, address: Option<String>) -> Vec<Effect> {
        match found {
            Some(f) => self.start_pair(f),
            None => {
                self.open_add();
                if let Some(a) = address {
                    self.seed_drafts(DraftSeed {
                        code: Some(String::new()),
                        address: Some(a),
                        link: Some(String::new()),
                        ..Default::default()
                    });
                }
                Vec::new()
            }
        }
    }

    fn first_remote_text(&self, mac: &SavedMac) -> Option<String> {
        macs::remote_hosts(mac, &self.prefs)
            .first()
            .map(|(h, p, _)| macs::host_port(h, *p))
    }

    fn retry_card(&mut self) -> Vec<Effect> {
        let restart = matches!(
            &self.screen,
            Screen::Macs { card: Some(Card::Pair(p)) } if matches!(p.state, PairState::Failed { .. })
        );
        if restart {
            let g = self.take_gen();
            if let Screen::Macs {
                card: Some(Card::Pair(p)),
            } = &mut self.screen
            {
                p.state = PairState::Asking;
                p.gen = g;
                p.pin = None;
                p.shown = false;
                let addrs = p.addrs.clone();
                self.seed_drafts(DraftSeed {
                    code: Some(String::new()),
                    ..Default::default()
                });
                return vec![Effect::Ask { gen: g, addrs }];
            }
        }
        if let Screen::Macs {
            card: Some(Card::Add(a)),
        } = &mut self.screen
        {
            if matches!(a.state, PairState::Failed { .. }) {
                a.state = PairState::Code { problem: None };
                a.problem = None;
                a.pending_link = None;
            }
        }
        Vec::new()
    }

    fn on_pair_asked(&mut self, gen: u64, r: &Result<(PairResult, [u8; 32]), PairFailure>) {
        let Screen::Macs {
            card: Some(Card::Pair(p)),
        } = &mut self.screen
        else {
            return;
        };
        if p.gen != gen || !matches!(p.state, PairState::Asking) {
            return;
        }
        match pairing::after_ask(&p.mac_name, r) {
            AskOutcome::Code { pin, shown } => {
                p.state = PairState::Code { problem: None };
                p.pin = Some(pin);
                p.shown = shown;
                self.seed_drafts(DraftSeed {
                    code: Some(String::new()),
                    ..Default::default()
                });
            }
            AskOutcome::Failed(t) => p.state = PairState::Failed { text: t },
        }
    }

    fn submit_code(&mut self, s: &str) -> Vec<Effect> {
        let Screen::Macs {
            card: Some(Card::Pair(p)),
        } = &mut self.screen
        else {
            return Vec::new();
        };
        if !matches!(p.state, PairState::Code { .. }) {
            return Vec::new();
        }
        match pairing::check_typed_code(s) {
            Err(t) => {
                p.state = PairState::Code { problem: Some(t) };
                Vec::new()
            }
            Ok(code) => {
                p.state = PairState::Working;
                vec![Effect::PairCode {
                    gen: p.gen,
                    addrs: p.addrs.clone(),
                    pin: p.pin,
                    code,
                    remote: false,
                }]
            }
        }
    }

    /// The Add card while it takes input (state Code).
    fn add_card_in_code(&mut self) -> Option<&mut AddCard> {
        match &mut self.screen {
            Screen::Macs {
                card: Some(Card::Add(a)),
            } if matches!(a.state, PairState::Code { .. }) => Some(a),
            _ => None,
        }
    }

    fn submit_address(&mut self, address: &str, code: &str) -> Vec<Effect> {
        let Some(a) = self.add_card_in_code() else {
            return Vec::new();
        };
        let parsed = match parse_address(address) {
            Ok(p) => p,
            Err(e) => {
                a.problem = Some((AddField::Address, texts::bad_address(&e)));
                return Vec::new();
            }
        };
        let code = match pairing::check_typed_code(code) {
            Ok(c) => c,
            Err(t) => {
                a.problem = Some((AddField::Code, t));
                return Vec::new();
            }
        };
        a.typed_address = Some(address.trim().to_string());
        a.problem = None;
        a.state = PairState::Working;
        vec![Effect::PairCodeAt {
            gen: a.gen,
            host: parsed.host,
            port: parsed.port.unwrap_or(REMOTE_DEFAULT_PORT),
            code,
        }]
    }

    fn on_pair_done(
        &mut self,
        gen: u64,
        r: &Result<Paired, PairFailure>,
        now: Instant,
    ) -> Vec<Effect> {
        enum Which {
            Pair(String),
            /// The name and the remote address to remember.
            Add(String, Option<String>),
        }
        let which = match &self.screen {
            Screen::Macs {
                card: Some(Card::Pair(p)),
            } if p.gen == gen && matches!(p.state, PairState::Working) => {
                Which::Pair(p.mac_name.clone())
            }
            Screen::Macs {
                card: Some(Card::Add(a)),
            } if a.gen == gen && matches!(a.state, PairState::Working) => {
                if a.link_mode {
                    match &a.pending_link {
                        Some(l) => {
                            let remote = l
                                .addresses
                                .first()
                                .map(|ad| macs::host_port(&ad.host, ad.port.unwrap_or(l.port)));
                            Which::Add(l.name.clone(), remote)
                        }
                        None => Which::Add(String::new(), None),
                    }
                } else {
                    let t = a.typed_address.clone();
                    Which::Add(t.clone().unwrap_or_default(), t)
                }
            }
            _ => return Vec::new(),
        };
        let name = match &which {
            Which::Pair(n) | Which::Add(n, _) => n.clone(),
        };
        let mut out = Vec::new();
        let new_state = match pairing::after_pair(&name, r) {
            PairOutcome::Paired(mac) => {
                out.push(Effect::SaveMac(mac.clone()));
                if let Which::Add(_, Some(remote)) = &which {
                    self.prefs
                        .macs
                        .entry(mac.mac_id.clone())
                        .or_default()
                        .remote_host = Some(remote.clone());
                    out.push(Effect::SavePrefs(self.prefs.clone()));
                    self.refresh_rows();
                }
                self.status = texts::paired_with(&mac.name);
                PairState::Paired { mac, at: now }
            }
            PairOutcome::RetryCode(t) => {
                self.seed_drafts(DraftSeed {
                    code: Some(String::new()),
                    ..Default::default()
                });
                match which {
                    Which::Pair(_) => PairState::Code { problem: Some(t) },
                    Which::Add(..) => {
                        if let Screen::Macs {
                            card: Some(Card::Add(a)),
                        } = &mut self.screen
                        {
                            a.problem = Some((AddField::Code, t));
                        }
                        PairState::Code { problem: None }
                    }
                }
            }
            PairOutcome::Failed(t) => PairState::Failed { text: t },
        };
        if let Screen::Macs {
            card: Some(Card::Pair(PairCard { state, .. }) | Card::Add(AddCard { state, .. })),
        } = &mut self.screen
        {
            *state = new_state;
        }
        out
    }

    fn connect(&mut self, plan: ConnectPlan) -> Vec<Effect> {
        let (mac_id, name) = macs::plan_mac(&plan);
        let Some((route, desc)) = macs::route_for(&plan, &self.prefs) else {
            self.banner = Some(Banner {
                text: texts::end_text(&EndKind::PinMismatch, &name, ""),
                actions: vec![BannerAction::ForgetAndPair],
                mac_id,
            });
            // The Paired card would retry on every tick.
            let paired = match &self.screen {
                Screen::Macs {
                    card: Some(Card::Pair(p)),
                } => matches!(p.state, PairState::Paired { .. }),
                Screen::Macs {
                    card: Some(Card::Add(a)),
                } => matches!(a.state, PairState::Paired { .. }),
                _ => false,
            };
            if paired {
                self.set_card(None);
            }
            return Vec::new();
        };
        self.banner = None;
        self.status.clear();
        self.dialog = None;
        self.set_card(None);
        let mut out = Vec::new();
        if self.stream().is_some() {
            out.push(Effect::StopStream);
        }
        self.screen = Screen::Stream(StreamView::new(mac_id, desc, &name));
        out.push(Effect::StartStream {
            route,
            mac_name: name,
        });
        out
    }

    /// Shows a toast, or refreshes the live one with the same text instead of stacking (D8).
    fn show_toast(&mut self, text: String, action: Option<ToastAction>, now: Instant) {
        if let Some(t) = self.toasts.iter_mut().find(|t| t.text == text) {
            t.until = now + TOAST_TIME;
            t.action = action;
        } else {
            self.toasts.push(Toast {
                text,
                until: now + TOAST_TIME,
                action,
            });
        }
    }

    /// A Mac Settings pick, through the ledger (spec 3.8 B/C).
    fn pick_settings(&mut self, c: &HostSettingsChange, now: Instant) -> Vec<Effect> {
        let token = self.next_settings_token;
        let Some(s) = self.stream_mut() else {
            return Vec::new();
        };
        match s.session.settings.pick(c, token, now) {
            Some(out) => {
                self.next_settings_token += 1;
                log::info!("settings: asking the Mac for {out:?}");
                vec![Effect::Session(SessionCommand::ChangeSettings(out))]
            }
            None => Vec::new(),
        }
    }

    /// [Lower Quality] on the BAR20 toast: dismiss it and send the current suggestion, if any.
    fn lower_quality(&mut self, now: Instant) -> Vec<Effect> {
        self.toasts
            .retain(|t| t.action != Some(ToastAction::LowerQuality));
        let Some(c) = self.stream().and_then(|s| s.lower_quality_change()) else {
            return Vec::new();
        };
        self.pick_settings(&c, now)
    }

    fn select_window(&mut self, id: u32, now: Instant) -> Vec<Effect> {
        let mut toast = None;
        if let Some(s) = self.stream_mut() {
            if !s.window_toast_shown {
                s.window_toast_shown = true;
                toast = Some(texts::everyone_sees(&s.session.mac_name));
            }
        }
        if let Some(text) = toast {
            self.toasts.push(Toast {
                text,
                until: now + TOAST_TIME,
                action: None,
            });
        }
        vec![Effect::Session(SessionCommand::Select(
            StreamSource::Window { id },
        ))]
    }

    fn open_dialog(&mut self, d: Dialog) -> Vec<Effect> {
        match &d {
            Dialog::Preferences { .. } => self.seed_drafts(DraftSeed {
                device_name: Some(self.prefs.device_name()),
                hello_version: Some(self.prefs.hello_version()),
                ..Default::default()
            }),
            Dialog::MacDetails { mac_id, .. } => {
                let host = self
                    .prefs
                    .macs
                    .get(mac_id)
                    .and_then(|p| p.remote_host.clone())
                    .unwrap_or_default();
                self.seed_drafts(DraftSeed {
                    remote_host: Some(host),
                    ..Default::default()
                });
            }
            _ => {}
        }
        self.dialog = Some(d);
        if self.stream().is_some() {
            vec![Effect::ReleaseAll]
        } else {
            Vec::new()
        }
    }

    fn confirm_dialog(&mut self, now: Instant) -> Vec<Effect> {
        match self.dialog.clone() {
            Some(Dialog::ConfirmForget { mac_id, then_pair }) => {
                self.dialog = None;
                let Some(mac) = self.saved.iter().find(|m| m.mac_id == mac_id).cloned() else {
                    return Vec::new();
                };
                // Everything pairing again needs is read before the manual address goes.
                let found = macs::found_for(&mac, &self.found).cloned();
                let address = self.first_remote_text(&mac);
                let mut out = vec![Effect::Forget(mac_id.clone())];
                self.prefs.macs.remove(&mac_id);
                out.push(Effect::SavePrefs(self.prefs.clone()));
                self.refresh_rows();
                self.status = texts::forgot(&mac.name);
                if then_pair {
                    self.banner = None;
                    out.extend(self.pair_again(found, address));
                }
                out
            }
            Some(Dialog::ConfirmPlain { found }) => {
                self.dialog = None;
                self.update(Msg::Connect(ConnectPlan::Plain(found)), now)
            }
            _ => Vec::new(),
        }
    }

    fn banner_action(&mut self, a: BannerAction) -> Vec<Effect> {
        if a == BannerAction::GetSill {
            self.banner = None;
            return vec![Effect::OpenUrl(texts::GET_SILL_URL.to_string())];
        }
        let mac = self
            .banner
            .as_ref()
            .and_then(|b| b.mac_id.as_deref())
            .and_then(|id| self.saved.iter().find(|m| m.mac_id == id))
            .cloned();
        let Some(mac) = mac else {
            self.banner = None;
            return Vec::new();
        };
        match a {
            BannerAction::PairAgain | BannerAction::Pair => {
                self.banner = None;
                let found = macs::found_for(&mac, &self.found).cloned();
                let address = self.first_remote_text(&mac);
                self.pair_again(found, address)
            }
            BannerAction::ConnectAtHome => {
                self.banner = None;
                self.connect(ConnectPlan::Home(mac))
            }
            BannerAction::ForgetAndPair => {
                self.dialog = Some(Dialog::ConfirmForget {
                    mac_id: mac.mac_id,
                    then_pair: true,
                });
                Vec::new()
            }
            BannerAction::GetSill => Vec::new(),
        }
    }

    fn set_prefs(&mut self, mut p: Prefs) -> Vec<Effect> {
        p.ui_zoom = if p.ui_zoom.is_finite() {
            p.ui_zoom.clamp(ZOOM_MIN, ZOOM_MAX)
        } else {
            1.0
        };
        if p.fps != 120 {
            p.fps = 60;
        }
        let old = std::mem::replace(&mut self.prefs, p);
        let mut seed = DraftSeed::default();
        if old.device_name.is_some() && self.prefs.device_name.is_none() {
            seed.device_name = Some(self.prefs.device_name());
        }
        if old.hello_version.is_some() && self.prefs.hello_version.is_none() {
            seed.hello_version = Some(self.prefs.hello_version());
        }
        if seed != DraftSeed::default() {
            self.seed_drafts(seed);
        }
        self.refresh_rows();
        vec![Effect::SavePrefs(self.prefs.clone())]
    }

    fn set_remote_host(&mut self, mac_id: String, host: Option<String>) -> Vec<Effect> {
        match host {
            Some(h) => {
                let trimmed = h.trim().to_string();
                if let Err(e) = parse_address(&trimmed) {
                    if let Some(Dialog::MacDetails { problem, .. }) = &mut self.dialog {
                        *problem = Some(texts::bad_address(&e));
                    }
                    return Vec::new();
                }
                self.prefs.macs.entry(mac_id).or_default().remote_host = Some(trimmed);
                if let Some(Dialog::MacDetails { problem, .. }) = &mut self.dialog {
                    *problem = None;
                }
            }
            None => {
                if let Some(e) = self.prefs.macs.get_mut(&mac_id) {
                    e.remote_host = None;
                    if *e == MacPrefs::default() {
                        self.prefs.macs.remove(&mac_id);
                    }
                }
                self.seed_drafts(DraftSeed {
                    remote_host: Some(String::new()),
                    ..Default::default()
                });
            }
        }
        self.refresh_rows();
        vec![Effect::SavePrefs(self.prefs.clone())]
    }

    /// The next time the model needs a `Msg::Tick` (bar timing, the countdown, the paired
    /// pause, the waiting card, LIST3, toasts, the oldest unanswered pick's expiry, a pick's
    /// spinner delay, and SET25's wait).
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let mut best: Option<Instant> = None;
        let mut take = |t: Instant| best = Some(best.map_or(t, |b| b.min(t)));
        for t in &self.toasts {
            take(t.until);
        }
        if let Screen::Macs {
            card:
                Some(
                    Card::Pair(PairCard {
                        state: PairState::Paired { at, .. },
                        ..
                    })
                    | Card::Add(AddCard {
                        state: PairState::Paired { at, .. },
                        ..
                    }),
                ),
        } = &self.screen
        {
            take(*at + PAIRED_PAUSE);
        }
        if !self.ever_found && self.started_at + HINT_AFTER > now {
            take(self.started_at + HINT_AFTER);
        }
        if let Screen::Stream(s) = &self.screen {
            match &s.session.phase {
                Phase::Reconnecting { retry_at, .. } => {
                    take((*retry_at).min(now + Duration::from_secs(1)))
                }
                Phase::Connected if s.session.first_frame_at.is_none() => {
                    if let Some(c) = s.session.connected_at {
                        if c + WAITING_AFTER > now {
                            take(c + WAITING_AFTER);
                        }
                    }
                }
                _ => {}
            }
            if let Some(o) = s.session.settings.oldest_pending() {
                take(o + s.session.settings_timeout() + PICK_EXPIRY_SLACK);
            }
            for p in s.session.settings.pending().values() {
                let t = p.sent_at + SPINNER_DELAY;
                if t > now {
                    take(t);
                }
            }
            if s.session.is_connected() && s.session.settings.host().is_none() {
                if let Some(c) = s.session.connected_at {
                    if c + SETTINGS_WAIT > now {
                        take(c + SETTINGS_WAIT);
                    }
                }
            }
            for t in [
                s.bar.hover_since.map(|t| t + BAR_HOVER_DELAY),
                s.bar.left_at.map(|t| t + BAR_HIDE_DELAY),
                s.bar.reveal_until,
            ]
            .into_iter()
            .flatten()
            {
                if t > now {
                    take(t);
                }
            }
        }
        best
    }
}

/// A Pair or Add card whose pairing is in flight.
fn card_is_working(card: Option<&Card>) -> bool {
    match card {
        Some(Card::Pair(p)) => matches!(p.state, PairState::Working),
        Some(Card::Add(a)) => matches!(a.state, PairState::Working),
        _ => false,
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

    // ---- update (WP-B3) ------------------------------------------------------------------

    use crate::gui::macs::fixtures::{found, saved};
    use transom_net::supervisor::{Endpoint, Trust};
    use transom_proto::messages::goodbye_reason as gr;

    const GOOD_CODE: &str = "000000000000";
    const PIN: [u8; 32] = [5; 32];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn model_with(saved_macs: Vec<SavedMac>) -> (Model, Instant) {
        let now = Instant::now();
        (
            Model::new(IdentityInfo::default(), Prefs::default(), saved_macs, now),
            now,
        )
    }

    fn browse(m: &mut Model, f: FoundMac, now: Instant) {
        m.update(Msg::Browse(BrowseEvent::Found(f)), now);
    }

    fn ask_result(reason: &str) -> Result<(PairResult, [u8; 32]), PairFailure> {
        Ok((
            PairResult {
                ok: false,
                reason: Some(reason.into()),
                ..Default::default()
            },
            PIN,
        ))
    }

    fn paired_ok(mac_id: &str) -> Result<Paired, PairFailure> {
        Ok(Paired {
            mac_id: mac_id.into(),
            name: "Mac".into(),
            fingerprint: [7; 32],
            recognition_key: vec![1; 32],
            peer: "127.0.0.1:7455".parse().unwrap(),
        })
    }

    fn refused(reason: &str, tries: Option<i64>) -> PairFailure {
        PairFailure::Refused {
            reason: reason.into(),
            tries_left: tries,
            retry_after: None,
            message: None,
        }
    }

    fn pair_card(m: &Model) -> &PairCard {
        match &m.screen {
            Screen::Macs {
                card: Some(Card::Pair(p)),
            } => p,
            other => panic!("not a pair card: {other:?}"),
        }
    }

    fn add_card(m: &Model) -> &AddCard {
        match &m.screen {
            Screen::Macs {
                card: Some(Card::Add(a)),
            } => a,
            other => panic!("not an add card: {other:?}"),
        }
    }

    fn has_card(m: &Model) -> bool {
        matches!(&m.screen, Screen::Macs { card: Some(_) })
    }

    /// A model that asked Delta and got the code prompt (`shown` as given).
    fn code_prompt(shown: bool) -> (Model, Instant) {
        let (mut m, now) = model_with(Vec::new());
        browse(
            &mut m,
            found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6"),
            now,
        );
        let key = m.rows_other[0].key.clone();
        let fx = m.update(Msg::ClickRow { key }, now);
        let Effect::Ask { gen, .. } = fx[0] else {
            panic!("no ask")
        };
        let reason = if shown { "shown" } else { "openOnMac" };
        m.update(Msg::PairAsked(gen, ask_result(reason)), now);
        (m, now)
    }

    fn connected_model(mac_id: Option<&str>) -> (Model, Instant) {
        let (mut m, now) = model_with(Vec::new());
        m.screen = Screen::Stream(StreamView::new(
            mac_id.map(String::from),
            RouteDesc::Home,
            "Mock Mac",
        ));
        m.update(
            Msg::Session(SessionEvent::Connected {
                mac_name: "Mock Mac".into(),
                peer: "127.0.0.1:1".parse().unwrap(),
                local_peer: false,
            }),
            now,
        );
        (m, now)
    }

    fn finish(m: &mut Model, kind: EndKind, now: Instant) -> Vec<Effect> {
        m.update(
            Msg::Session(SessionEvent::Finished {
                text: "x".into(),
                kind,
            }),
            now,
        )
    }

    fn goodbye(reason: &str) -> EndKind {
        EndKind::Goodbye {
            reason: reason.into(),
            message: None,
            minimum_version: None,
        }
    }

    #[test]
    fn click_not_paired_row_asks() {
        let (mut m, now) = model_with(Vec::new());
        browse(
            &mut m,
            found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6"),
            now,
        );
        assert!(m.ever_found);
        let key = m.rows_other[0].key.clone();
        let fx = m.update(Msg::ClickRow { key }, now);
        assert!(matches!(fx.as_slice(), [Effect::Ask { .. }]));
        let p = pair_card(&m);
        assert!(matches!(p.state, PairState::Asking));
        assert_eq!(p.mac_name, "Delta");
        assert_eq!(m.draft_seed.code.as_deref(), Some(""));
        // An open door asks nothing until [Pair].
        browse(
            &mut m,
            found("Foxtrot", None, HomeDoor::Open, "192.168.1.8"),
            now,
        );
        let key = m.rows_other[1].key.clone();
        assert!(m.update(Msg::ClickRow { key }, now).is_empty());
        assert!(matches!(
            m.screen,
            Screen::Macs {
                card: Some(Card::OpenDoor { .. })
            }
        ));
        let fx = m.update(Msg::PairOpenDoor, now);
        assert!(matches!(fx.as_slice(), [Effect::Ask { .. }]));
    }

    #[test]
    fn ask_shown_then_cancel_sends_cancel_ask() {
        let (mut m, now) = code_prompt(true);
        let p = pair_card(&m);
        assert!(p.shown && p.pin == Some(PIN));
        assert!(matches!(p.state, PairState::Code { problem: None }));
        let fx = m.update(Msg::CancelCard, now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::CancelAsk { pin, .. }] if *pin == PIN
        ));
        assert!(!has_card(&m));
    }

    #[test]
    fn ask_open_on_mac_cancel_sends_nothing() {
        let (mut m, now) = code_prompt(false);
        assert!(!pair_card(&m).shown);
        assert!(m.update(Msg::CancelCard, now).is_empty());
        assert!(!has_card(&m));
    }

    #[test]
    fn stale_generation_is_ignored() {
        let (mut m, now) = model_with(Vec::new());
        browse(
            &mut m,
            found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6"),
            now,
        );
        let key = m.rows_other[0].key.clone();
        m.update(Msg::ClickRow { key }, now);
        let gen = pair_card(&m).gen;
        m.update(Msg::PairAsked(gen + 1, ask_result("shown")), now);
        assert!(matches!(pair_card(&m).state, PairState::Asking));
        m.update(Msg::PairAsked(gen, ask_result("shown")), now);
        m.update(Msg::SubmitCode(GOOD_CODE.into()), now);
        assert!(matches!(pair_card(&m).state, PairState::Working));
        let fx = m.update(Msg::PairDone(gen + 1, paired_ok("M")), now);
        assert!(fx.is_empty());
        assert!(matches!(pair_card(&m).state, PairState::Working));
        // A late answer after the card was closed changes nothing.
        m.update(Msg::PairDone(gen, Err(refused("nope", None))), now);
        m.update(Msg::CancelCard, now);
        assert!(!has_card(&m));
        assert!(m.update(Msg::PairDone(gen, paired_ok("M")), now).is_empty());
    }

    #[test]
    fn cancel_is_ignored_while_pairing_is_working() {
        // The Pair card.
        let (mut m, now) = code_prompt(true);
        let gen = pair_card(&m).gen;
        m.update(Msg::SubmitCode(GOOD_CODE.into()), now);
        assert!(matches!(pair_card(&m).state, PairState::Working));
        assert!(m.update(Msg::CancelCard, now).is_empty());
        assert!(matches!(pair_card(&m).state, PairState::Working));
        let fx = m.update(Msg::PairDone(gen, paired_ok("M")), now);
        assert!(matches!(fx.as_slice(), [Effect::SaveMac(s)] if s.mac_id == "M"));
        // The Add card.
        let (mut m, now) = model_with(Vec::new());
        m.update(Msg::OpenAdd, now);
        let gen = add_card(&m).gen;
        m.update(
            Msg::SubmitAddress {
                address: "127.0.0.1:7455".into(),
                code: GOOD_CODE.into(),
            },
            now,
        );
        assert!(matches!(add_card(&m).state, PairState::Working));
        assert!(m.update(Msg::CancelCard, now).is_empty());
        assert!(matches!(add_card(&m).state, PairState::Working));
        let fx = m.update(Msg::PairDone(gen, paired_ok("M")), now);
        assert!(matches!(fx.first(), Some(Effect::SaveMac(s)) if s.mac_id == "M"));
    }

    #[test]
    fn nothing_replaces_a_card_while_pairing_is_working() {
        let (mut m, now) = model_with(Vec::new());
        m.update(Msg::OpenAdd, now);
        let gen = add_card(&m).gen;
        m.update(
            Msg::SubmitAddress {
                address: "127.0.0.1:7455".into(),
                code: GOOD_CODE.into(),
            },
            now,
        );
        assert!(matches!(add_card(&m).state, PairState::Working));
        // Ctrl/Cmd+N opens a fresh Add card when nothing is in flight; here it must not.
        assert!(m.update(Msg::OpenAdd, now).is_empty());
        assert_eq!(add_card(&m).gen, gen);
        assert!(matches!(add_card(&m).state, PairState::Working));
        let fx = m.update(Msg::PairDone(gen, paired_ok("M")), now);
        assert!(matches!(fx.first(), Some(Effect::SaveMac(s)) if s.mac_id == "M"));
    }

    #[test]
    fn unroutable_paired_mac_clears_the_card_once() {
        let (mut m, now) = model_with(Vec::new());
        m.update(Msg::OpenAdd, now);
        let gen = add_card(&m).gen;
        m.update(
            Msg::SubmitAddress {
                address: "127.0.0.1:7455".into(),
                code: GOOD_CODE.into(),
            },
            now,
        );
        m.update(Msg::PairDone(gen, paired_ok("M")), now);
        // The remote plan cannot be routed when the saved key is unreadable.
        if let Screen::Macs {
            card: Some(Card::Add(a)),
        } = &mut m.screen
        {
            if let PairState::Paired { mac, .. } = &mut a.state {
                mac.fingerprint = "!!".into();
            }
        }
        let fx = m.update(Msg::Tick, now + PAIRED_PAUSE);
        assert!(fx.is_empty());
        assert!(!has_card(&m));
        assert!(m.banner.is_some());
        assert!(m.update(Msg::Tick, now + PAIRED_PAUSE + ms(500)).is_empty());
    }

    #[test]
    fn new_clamps_the_stored_zoom() {
        let (m, _) = model_with(Vec::new());
        assert_eq!(m.prefs.ui_zoom, 1.0);
        for (z, want) in [(9.0, ZOOM_MAX), (0.1, ZOOM_MIN), (f32::NAN, 1.0)] {
            let p = Prefs {
                ui_zoom: z,
                ..Prefs::default()
            };
            let m = Model::new(m.identity.clone(), p, Vec::new(), Instant::now());
            assert_eq!(m.prefs.ui_zoom, want);
        }
    }

    #[test]
    fn submit_code_length_and_typo_inline() {
        let (mut m, now) = code_prompt(true);
        let fx = m.update(Msg::SubmitCode("123".into()), now);
        assert!(fx.is_empty());
        assert!(matches!(
            &pair_card(&m).state,
            PairState::Code { problem: Some(t) } if t == texts::CODE_LENGTH
        ));
        let fx = m.update(Msg::SubmitCode("000000000001".into()), now);
        assert!(fx.is_empty());
        assert!(matches!(
            &pair_card(&m).state,
            PairState::Code { problem: Some(t) } if t == texts::CODE_TYPO
        ));
        let fx = m.update(Msg::SubmitCode("0000 0000 0000".into()), now);
        match fx.as_slice() {
            [Effect::PairCode {
                pin,
                code,
                remote: false,
                ..
            }] => {
                assert_eq!(*pin, Some(PIN));
                assert_eq!(code, GOOD_CODE);
            }
            _ => panic!("no pair code"),
        }
        assert!(matches!(pair_card(&m).state, PairState::Working));
    }

    #[test]
    fn paired_connects_home_after_pause() {
        let mac = saved("M", "Mac", 1);
        let (mut m, now) = model_with(vec![mac]);
        browse(
            &mut m,
            found("Mac", None, HomeDoor::PairingRequired, "192.168.1.6"),
            now,
        );
        let key = m.rows_other[0].key.clone();
        m.update(Msg::ClickRow { key }, now);
        let gen = pair_card(&m).gen;
        m.update(Msg::PairAsked(gen, ask_result("shown")), now);
        m.update(Msg::SubmitCode(GOOD_CODE.into()), now);
        let fx = m.update(Msg::PairDone(gen, paired_ok("M")), now);
        assert!(matches!(fx.as_slice(), [Effect::SaveMac(s)] if s.mac_id == "M"));
        assert_eq!(m.status, texts::paired_with("Mac"));
        assert!(matches!(pair_card(&m).state, PairState::Paired { .. }));
        assert!(m.update(Msg::Tick, now + ms(799)).is_empty());
        assert!(!matches!(m.screen, Screen::Stream(_)));
        let fx = m.update(Msg::Tick, now + PAIRED_PAUSE);
        assert!(matches!(
            fx.as_slice(),
            [Effect::StartStream {
                route: Route::SavedHome { .. },
                ..
            }]
        ));
        assert!(matches!(m.screen, Screen::Stream(_)));
        assert!(m.status.is_empty());
    }

    #[test]
    fn wrong_code_shows_tries_left_and_clears_code() {
        let (mut m, now) = code_prompt(true);
        let gen = pair_card(&m).gen;
        m.update(Msg::SubmitCode(GOOD_CODE.into()), now);
        let epoch = m.draft_epoch;
        m.update(Msg::PairDone(gen, Err(refused("code", Some(2)))), now);
        assert!(matches!(
            &pair_card(&m).state,
            PairState::Code { problem: Some(t) } if t.contains("2 tries left")
        ));
        assert!(m.draft_epoch > epoch);
        assert_eq!(m.draft_seed.code.as_deref(), Some(""));
        // No tries left: the card fails.
        m.update(Msg::SubmitCode(GOOD_CODE.into()), now);
        m.update(Msg::PairDone(gen, Err(refused("code", Some(0)))), now);
        assert!(matches!(pair_card(&m).state, PairState::Failed { .. }));
        // [Try Again] asks again with a new generation.
        let fx = m.update(Msg::RetryCard, now);
        let [Effect::Ask { gen: g2, .. }] = fx.as_slice() else {
            panic!("no ask")
        };
        assert!(*g2 > gen);
        let p = pair_card(&m);
        assert!(matches!(p.state, PairState::Asking) && p.pin.is_none() && !p.shown);
    }

    #[test]
    fn add_bad_address_shows_add12() {
        let (mut m, now) = model_with(Vec::new());
        m.update(Msg::OpenAdd, now);
        assert_eq!(m.draft_seed.address.as_deref(), Some(""));
        let fx = m.update(
            Msg::SubmitAddress {
                address: "not an address".into(),
                code: GOOD_CODE.into(),
            },
            now,
        );
        assert!(fx.is_empty());
        let a = add_card(&m);
        assert!(matches!(
            &a.problem,
            Some((AddField::Address, t)) if t.starts_with("Check the address")
        ));
        assert!(matches!(a.state, PairState::Code { problem: None }));
        m.update(
            Msg::SubmitAddress {
                address: "100.1.2.3".into(),
                code: "12".into(),
            },
            now,
        );
        assert!(matches!(
            &add_card(&m).problem,
            Some((AddField::Code, t)) if t == texts::CODE_LENGTH
        ));
    }

    #[test]
    fn add_success_sets_remote_host_and_connects_remote() {
        let (mut m, now) = model_with(Vec::new());
        m.update(Msg::OpenAdd, now);
        let gen = add_card(&m).gen;
        let fx = m.update(
            Msg::SubmitAddress {
                address: " 127.0.0.1:7455 ".into(),
                code: GOOD_CODE.into(),
            },
            now,
        );
        assert!(matches!(
            fx.as_slice(),
            [Effect::PairCodeAt { host, port: 7455, .. }] if host == "127.0.0.1"
        ));
        let fx = m.update(Msg::PairDone(gen, paired_ok("M")), now);
        assert!(matches!(fx[0], Effect::SaveMac(_)));
        match &fx[1] {
            Effect::SavePrefs(p) => {
                assert_eq!(p.macs["M"].remote_host.as_deref(), Some("127.0.0.1:7455"))
            }
            _ => panic!("no prefs"),
        }
        assert!(m.update(Msg::Tick, now + ms(799)).is_empty());
        let fx = m.update(Msg::Tick, now + PAIRED_PAUSE);
        match fx.as_slice() {
            [Effect::StartStream {
                route:
                    Route::Direct {
                        endpoint: Endpoint::Hosts(h),
                        remote: true,
                        trust: Trust::Pinned(_),
                        ..
                    },
                ..
            }] => assert_eq!(h, &vec![("127.0.0.1".to_string(), 7455)]),
            _ => panic!("no remote stream"),
        }
        assert!(matches!(
            m.stream().map(|s| &s.route_desc),
            Some(RouteDesc::Remote { via: None })
        ));
    }

    #[test]
    fn add_link_bad_shows_add11_and_confirm_starts_pair_link() {
        let (mut m, now) = model_with(Vec::new());
        m.update(Msg::OpenAdd, now);
        m.update(Msg::ToggleLinkMode, now);
        assert!(add_card(&m).link_mode);
        m.update(Msg::SubmitLink("hello".into()), now);
        assert!(matches!(
            &add_card(&m).problem,
            Some((AddField::Link, t)) if t == texts::NOT_A_LINK
        ));
        assert!(m.update(Msg::ConfirmLink, now).is_empty());
        let link = PairLink {
            mac_id: transom_proto::identity::mac_id(&[7u8; 32]),
            fingerprint: [7; 32],
            secret: [1; 16],
            name: "Mac".into(),
            port: 7455,
            addresses: vec![parse_address("100.1.2.3").unwrap()],
        };
        m.update(Msg::SubmitLink(link.url()), now);
        let a = add_card(&m);
        assert!(a.problem.is_none() && a.pending_link.is_some());
        let gen = a.gen;
        let fx = m.update(Msg::ConfirmLink, now);
        assert!(matches!(fx.as_slice(), [Effect::PairLink { .. }]));
        assert!(add_card(&m).pending_link.is_some());
        let fx = m.update(Msg::PairDone(gen, paired_ok("M")), now);
        match &fx[1] {
            Effect::SavePrefs(p) => {
                assert_eq!(p.macs["M"].remote_host.as_deref(), Some("100.1.2.3:7455"))
            }
            _ => panic!("no prefs"),
        }
        // Mode toggling clears the pending link.
        m.update(Msg::CancelCard, now);
        m.update(Msg::OpenAdd, now);
        assert!(add_card(&m).pending_link.is_none());
    }

    #[test]
    fn finished_removed_gives_pair_again_banner() {
        let (mut m, now) = connected_model(Some("M"));
        m.saved = vec![saved("M", "Mock Mac", 1)];
        let fx = finish(&mut m, goodbye(gr::REMOVED), now);
        assert!(matches!(fx.as_slice(), [Effect::StopStream]));
        assert!(matches!(m.screen, Screen::Macs { card: None }));
        let b = m.banner.clone().unwrap();
        assert_eq!(b.actions, vec![BannerAction::PairAgain]);
        assert_eq!(b.text, texts::removed("Mock Mac"));
        assert_eq!(b.mac_id.as_deref(), Some("M"));
        // [Pair Again] with the Mac not found opens the Add card with its address.
        m.prefs.macs.insert(
            "M".into(),
            MacPrefs {
                remote_host: Some("100.1.2.3".into()),
            },
        );
        let fx = m.update(Msg::BannerAction(BannerAction::PairAgain), now);
        assert!(fx.is_empty() && m.banner.is_none());
        assert!(add_card(&m).typed_address.is_none());
        assert_eq!(m.draft_seed.address.as_deref(), Some("100.1.2.3:7455"));
    }

    #[test]
    fn finished_not_paired_and_pin_mismatch_banners() {
        let (mut m, now) = connected_model(Some("M"));
        finish(&mut m, EndKind::NotPaired, now);
        assert_eq!(
            m.banner.as_ref().unwrap().actions,
            vec![BannerAction::PairAgain]
        );
        let (mut m, now) = connected_model(Some("M"));
        m.saved = vec![saved("M", "Mock Mac", 1)];
        finish(&mut m, EndKind::PinMismatch, now);
        assert_eq!(
            m.banner.as_ref().unwrap().actions,
            vec![BannerAction::ForgetAndPair]
        );
        m.update(Msg::BannerAction(BannerAction::ForgetAndPair), now);
        assert!(matches!(
            m.dialog,
            Some(Dialog::ConfirmForget {
                then_pair: true,
                ..
            })
        ));
        assert!(m.banner.is_some());
        let fx = m.update(Msg::ConfirmDialog, now);
        assert!(matches!(fx[0], Effect::Forget(_)));
        assert!(m.banner.is_none());
        assert!(has_card(&m));
        // An unsaved Mac gets no pairing actions, but an update still gets Get Sill.
        let (mut m, now) = connected_model(None);
        finish(&mut m, EndKind::NotPaired, now);
        assert!(m.banner.as_ref().unwrap().actions.is_empty());
        let (mut m, now) = connected_model(None);
        finish(&mut m, goodbye(gr::UPDATE), now);
        assert_eq!(
            m.banner.as_ref().unwrap().actions,
            vec![BannerAction::GetSill]
        );
        let fx = m.update(Msg::BannerAction(BannerAction::GetSill), now);
        assert!(matches!(&fx[0], Effect::OpenUrl(u) if u == texts::GET_SILL_URL));
        let (mut m, now) = connected_model(Some("M"));
        finish(&mut m, goodbye(gr::PAIRING_REQUIRED), now);
        assert_eq!(m.banner.as_ref().unwrap().actions, vec![BannerAction::Pair]);
    }

    #[test]
    fn remote_off_offers_connect_at_home_only_when_found() {
        let (mut m, now) = connected_model(Some("M"));
        m.saved = vec![saved("M", "Mock Mac", 1)];
        finish(&mut m, goodbye(gr::REMOTE_OFF), now);
        assert!(m.banner.as_ref().unwrap().actions.is_empty());

        let (mut m, now) = connected_model(Some("M"));
        m.saved = vec![saved("M", "Mock Mac", 1)];
        browse(
            &mut m,
            found(
                "Mock Mac",
                Some(1),
                HomeDoor::PairingRequired,
                "192.168.1.4",
            ),
            now,
        );
        finish(&mut m, goodbye(gr::REMOTE_OFF), now);
        assert_eq!(
            m.banner.as_ref().unwrap().actions,
            vec![BannerAction::ConnectAtHome]
        );
        let fx = m.update(Msg::BannerAction(BannerAction::ConnectAtHome), now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::StartStream {
                route: Route::SavedHome { .. },
                ..
            }]
        ));
    }

    #[test]
    fn retry_now_goes_to_session() {
        let (mut m, now) = connected_model(None);
        let fx = m.update(Msg::RetryNow, now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::Session(SessionCommand::RetryNow)]
        ));
        let fx = m.update(Msg::SelectDesktop, now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::Session(SessionCommand::Select(
                StreamSource::Desktop {}
            ))]
        ));
    }

    #[test]
    fn select_window_toast_once() {
        let (mut m, now) = connected_model(None);
        let fx = m.update(Msg::SelectWindow(102), now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::Session(SessionCommand::Select(
                StreamSource::Window { id: 102 }
            ))]
        ));
        assert_eq!(m.toasts.len(), 1);
        assert_eq!(m.toasts[0].text, texts::everyone_sees("Mock Mac"));
        assert_eq!(m.toasts[0].until, now + TOAST_TIME);
        m.update(Msg::SelectWindow(101), now);
        assert_eq!(m.toasts.len(), 1);
        m.update(Msg::Tick, now + TOAST_TIME);
        assert!(m.toasts.is_empty());
    }

    use transom_proto::host_settings::HostSettingsState;

    fn settings_state(json_extra: &str) -> HostSettingsState {
        let json = format!(
            r#"{{"settings": {{"maxFPS": 60, "bitrate": 15000000, "captureScale": 2,
            "prioritizeSpeed": false, "virtualDisplay": false}},
            "persistent": true, "virtualDisplayAvailable": false,
            "softwareEncoder": false{json_extra}}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    fn feed(m: &mut Model, hs: HostSettingsState, now: Instant) {
        m.update(Msg::Session(SessionEvent::HostSettings(hs)), now);
    }

    fn fps(v: i64) -> Msg {
        Msg::ChangeSettings(HostSettingsChange {
            max_fps: Some(v),
            ..Default::default()
        })
    }

    fn shown(m: &Model) -> transom_proto::host_settings::StreamSettings {
        m.stream().unwrap().session.settings.displayed().unwrap()
    }

    const BEHIND: &str = r#", "link": {"state": "behind", "withheldPerSecond": 9,
        "bitrate": 15000000, "suggestedBitrate": 8000000, "suggestedCaptureScale": 1}"#;

    #[test]
    fn change_settings_without_state_sends_nothing() {
        let (mut m, now) = connected_model(None);
        assert!(m.update(fps(120), now).is_empty());
        assert_eq!(m.next_settings_token, 1);
    }

    #[test]
    fn change_settings_sends_with_increasing_tokens() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        let fx = m.update(fps(120), now);
        let [Effect::Session(SessionCommand::ChangeSettings(c))] = fx.as_slice() else {
            panic!("{fx:?}");
        };
        assert_eq!(c.token, Some(1));
        assert_eq!(c.max_fps, Some(120));
        let fx = m.update(
            Msg::ChangeSettings(HostSettingsChange {
                bitrate: Some(25_000_000),
                ..Default::default()
            }),
            now,
        );
        let [Effect::Session(SessionCommand::ChangeSettings(c))] = fx.as_slice() else {
            panic!("{fx:?}");
        };
        assert_eq!(c.token, Some(2));
        assert!(m.update(fps(120), now).is_empty());
        assert_eq!(m.next_settings_token, 3);
    }

    #[test]
    fn refused_answer_snaps_back_and_toasts_once() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        let fx = m.update(
            Msg::ChangeSettings(HostSettingsChange {
                virtual_display: Some(true),
                ..Default::default()
            }),
            now,
        );
        assert_eq!(fx.len(), 1);
        assert!(shown(&m).virtual_display);
        feed(&mut m, settings_state(r#", "answering": 1"#), now);
        assert!(!shown(&m).virtual_display);
        assert_eq!(m.toasts.len(), 1);
        assert_eq!(m.toasts[0].text, texts::kept_setting("Mock Mac"));
        assert_eq!(m.toasts[0].action, None);
        feed(&mut m, settings_state(""), now);
        assert_eq!(m.toasts.len(), 1);
    }

    #[test]
    fn accepted_answer_shows_the_value_without_toast() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        m.update(fps(120), now);
        let mut hs = settings_state(r#", "answering": 1"#);
        hs.settings.max_fps = 120;
        feed(&mut m, hs, now);
        assert_eq!(shown(&m).max_fps, 120);
        assert!(m.stream().unwrap().session.settings.pending().is_empty());
        assert!(m.toasts.is_empty());
    }

    #[test]
    fn unanswered_pick_reverts_after_the_timeout() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        m.update(fps(120), now);
        assert_eq!(shown(&m).max_fps, 120);
        let d = m.next_deadline(now).expect("a deadline");
        assert!(d <= now + Duration::from_secs(4) + PICK_EXPIRY_SLACK);
        assert_eq!(
            m.next_deadline(now + Duration::from_secs(2)),
            Some(now + Duration::from_secs(4) + PICK_EXPIRY_SLACK)
        );
        m.update(Msg::Tick, now + Duration::from_secs(4));
        assert_eq!(shown(&m).max_fps, 120);
        m.update(Msg::Tick, now + Duration::from_secs(4) + PICK_EXPIRY_SLACK);
        assert_eq!(shown(&m).max_fps, 60);
        assert!(m.toasts.is_empty());
    }

    #[test]
    fn timeout_scales_with_the_worst_round_trip() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        m.update(
            Msg::Session(SessionEvent::Stats {
                rtt_ms: Some(900.0),
                rtt_max_ms: Some(2000.0),
                fps: 60,
            }),
            now,
        );
        m.update(fps(120), now);
        m.update(Msg::Tick, now + Duration::from_secs(5));
        assert_eq!(shown(&m).max_fps, 120);
        m.update(Msg::Tick, now + Duration::from_secs(8) + PICK_EXPIRY_SLACK);
        assert_eq!(shown(&m).max_fps, 60);
    }

    #[test]
    fn spinner_shows_after_300_ms() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        m.update(fps(120), now);
        let s = m.stream().unwrap();
        assert!(!s.pick_spinner(SettingsField::MaxFps, now + Duration::from_millis(299)));
        assert!(s.pick_spinner(SettingsField::MaxFps, now + Duration::from_millis(300)));
        assert!(!s.pick_spinner(SettingsField::Bitrate, now + Duration::from_secs(1)));
        assert!(m.next_deadline(now).unwrap() <= now + SPINNER_DELAY);
    }

    #[test]
    fn settings_overdue_after_two_seconds() {
        let (mut m, now) = connected_model(None);
        let s = m.stream().unwrap();
        assert!(!s.settings_overdue(now + Duration::from_millis(1999)));
        assert!(s.settings_overdue(now + Duration::from_secs(2)));
        assert!(m.next_deadline(now).unwrap() <= now + SETTINGS_WAIT);
        assert_eq!(
            m.next_deadline(now + Duration::from_millis(1600)),
            Some(now + SETTINGS_WAIT)
        );
        feed(&mut m, settings_state(""), now);
        assert!(!m
            .stream()
            .unwrap()
            .settings_overdue(now + Duration::from_secs(5)));
    }

    #[test]
    fn link_behind_toasts_once_per_spell() {
        let (mut m, now) = connected_model(None);
        let at = |s: u64| now + Duration::from_millis(s * 100);
        feed(&mut m, settings_state(BEHIND), now);
        assert_eq!(m.toasts.len(), 1);
        assert_eq!(m.toasts[0].text, texts::link_behind("Mock Mac"));
        assert_eq!(m.toasts[0].action, Some(ToastAction::LowerQuality));
        let until = m.toasts[0].until;
        feed(&mut m, settings_state(BEHIND), at(10));
        assert_eq!(m.toasts.len(), 1);
        assert_eq!(m.toasts[0].until, until);
        // Linger blocks: the 30 s gap has passed, so only LINK_LINGER can.
        m.update(Msg::Tick, at(400));
        assert!(m.toasts.is_empty());
        feed(&mut m, settings_state(""), at(400));
        feed(&mut m, settings_state(BEHIND), at(415));
        assert!(m.toasts.is_empty());
        feed(&mut m, settings_state(""), at(500));
        feed(&mut m, settings_state(BEHIND), at(530));
        assert_eq!(m.toasts.len(), 1);
    }

    #[test]
    fn link_toast_gap_blocks_a_quick_new_spell() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(BEHIND), now);
        assert_eq!(m.toasts.len(), 1);
        feed(&mut m, settings_state(""), now + Duration::from_secs(1));
        m.update(Msg::Tick, now + Duration::from_secs(4));
        assert!(m.toasts.is_empty());
        feed(&mut m, settings_state(BEHIND), now + Duration::from_secs(4));
        assert!(m.toasts.is_empty());
        let s = m.stream().unwrap();
        assert!(s.link_spell);
        assert_eq!(s.link_toast_at, Some(now));
    }

    #[test]
    fn link_toast_has_no_button_without_a_bitrate_or_when_away_at_home_quality() {
        let (mut m, now) = connected_model(None);
        feed(
            &mut m,
            settings_state(
                r#", "link": {"state": "behind", "withheldPerSecond": 9,
                "bitrate": 15000000, "suggestedCaptureScale": 1}"#,
            ),
            now,
        );
        assert_eq!(m.toasts.len(), 1);
        assert_eq!(m.toasts[0].action, None);

        let (mut m, now) = connected_model(None);
        let away = r#", "away": {"homeBitrate": 15000000, "homeCaptureScale": 2,
            "awayBitrate": 8000000, "awayCaptureScale": 1,
            "thisConnectionAway": true, "awayRunning": false}"#;
        feed(&mut m, settings_state(&format!("{BEHIND}{away}")), now);
        assert_eq!(m.toasts.len(), 1);
        assert_eq!(m.toasts[0].action, None);
    }

    #[test]
    fn lower_quality_applies_the_suggestion() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(BEHIND), now);
        let fx = m.update(Msg::ToastAction(ToastAction::LowerQuality), now);
        let [Effect::Session(SessionCommand::ChangeSettings(c))] = fx.as_slice() else {
            panic!("{fx:?}");
        };
        assert_eq!(c.bitrate, Some(8_000_000));
        assert_eq!(c.capture_scale, Some(1.0));
        assert!(c.token.is_some());
        assert!(m
            .toasts
            .iter()
            .all(|t| t.action != Some(ToastAction::LowerQuality)));
        let fx = m.update(Msg::ToastAction(ToastAction::LowerQuality), now);
        assert!(fx.is_empty());
    }

    #[test]
    fn tokens_continue_after_reconnect() {
        let (mut m, now) = connected_model(None);
        feed(&mut m, settings_state(""), now);
        let fx = m.update(fps(120), now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::Session(SessionCommand::ChangeSettings(c))] if c.token == Some(1)
        ));
        m.update(
            Msg::Session(SessionEvent::Ended {
                text: "x".into(),
                retry_in: Some(Duration::from_secs(1)),
                kind: EndKind::Lost,
            }),
            now,
        );
        assert!(m.stream().unwrap().session.settings.host().is_none());
        m.update(
            Msg::Session(SessionEvent::Connected {
                mac_name: "Mock Mac".into(),
                peer: "127.0.0.1:1".parse().unwrap(),
                local_peer: false,
            }),
            now,
        );
        feed(&mut m, settings_state(""), now);
        let fx = m.update(fps(120), now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::Session(SessionCommand::ChangeSettings(c))] if c.token == Some(2)
        ));
    }

    #[test]
    fn toggle_bar_pins_then_unpins_and_hides() {
        let (mut m, now) = connected_model(None);
        m.update(Msg::ToggleBar, now);
        assert!(m.stream().unwrap().bar.pinned);
        assert!(m.stream().unwrap().bar_visible(now));
        m.stream_mut().unwrap().bar.reveal_until = Some(now + BAR_REVEAL);
        m.update(Msg::BarHover(true), now);
        m.update(Msg::ToggleBar, now);
        let s = m.stream().unwrap();
        assert!(!s.bar.pinned);
        assert!(s.bar.hover_since.is_none() && s.bar.left_at.is_none());
        assert!(s.bar.reveal_until.is_none());
        assert!(!s.bar_visible(now));
    }

    #[test]
    fn bar_hover_and_leave_timing() {
        let (mut m, now) = connected_model(None);
        m.update(Msg::BarHover(true), now);
        m.update(Msg::BarHover(true), now + ms(100));
        assert_eq!(m.stream().unwrap().bar.hover_since, Some(now));
        assert!(!m.stream().unwrap().bar_visible(now + ms(100)));
        assert!(m.stream().unwrap().bar_visible(now + BAR_HOVER_DELAY));
        let t = now + ms(400);
        m.update(Msg::BarHover(false), t);
        let s = m.stream().unwrap();
        assert!(s.bar.hover_since.is_none());
        assert_eq!(s.bar.left_at, Some(t));
        assert!(s.bar_visible(t + ms(999)));
        assert!(!s.bar_visible(t + BAR_HIDE_DELAY));
        // Leaving while the bar was never shown starts no hide timer.
        m.update(Msg::BarHover(true), t + ms(2000));
        m.update(Msg::BarHover(false), t + ms(2100));
        assert_eq!(m.stream().unwrap().bar.left_at, None);
    }

    #[test]
    fn next_deadline_countdown_hint_and_toasts() {
        let (mut m, now) = model_with(Vec::new());
        assert_eq!(m.next_deadline(now), Some(now + HINT_AFTER));
        browse(
            &mut m,
            found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6"),
            now,
        );
        assert_eq!(m.next_deadline(now), None);
        m.toasts.push(Toast {
            text: "t".into(),
            until: now + ms(300),
            action: None,
        });
        assert_eq!(m.next_deadline(now), Some(now + ms(300)));
        m.toasts.clear();
        // Connected with no frame yet: the waiting card.
        let (mut s, now) = connected_model(None);
        s.found
            .push(found("Delta", None, HomeDoor::Open, "192.168.1.6"));
        s.ever_found = true;
        assert_eq!(s.next_deadline(now), Some(now + WAITING_AFTER));
        // Reconnecting: a tick every second until the retry.
        s.update(
            Msg::Session(SessionEvent::Ended {
                text: "Lost.".into(),
                retry_in: Some(Duration::from_secs(4)),
                kind: EndKind::Lost,
            }),
            now,
        );
        assert_eq!(s.next_deadline(now), Some(now + Duration::from_secs(1)));
        assert_eq!(
            s.next_deadline(now + Duration::from_millis(3500)),
            Some(now + Duration::from_secs(4))
        );
        // Bar timers.
        let (mut b, now) = connected_model(None);
        b.ever_found = true;
        b.update(Msg::BarHover(true), now);
        assert_eq!(b.next_deadline(now), Some(now + BAR_HOVER_DELAY));
        assert_eq!(
            b.next_deadline(now + BAR_HOVER_DELAY),
            Some(now + WAITING_AFTER)
        );
    }

    #[test]
    fn forget_confirm_removes_mac_prefs() {
        let mac = saved("M", "Mac", 1);
        let (mut m, now) = model_with(vec![mac]);
        m.prefs.macs.insert(
            "M".into(),
            MacPrefs {
                remote_host: Some("100.1.2.3".into()),
            },
        );
        m.refresh_rows();
        let key = m.rows_mine[0].key.clone();
        m.update(
            Msg::RowMenu {
                key,
                item: RowMenuItem::Forget,
            },
            now,
        );
        assert!(matches!(
            m.dialog,
            Some(Dialog::ConfirmForget {
                then_pair: false,
                ..
            })
        ));
        let fx = m.update(Msg::ConfirmDialog, now);
        assert!(matches!(&fx[0], Effect::Forget(id) if id == "M"));
        assert!(matches!(&fx[1], Effect::SavePrefs(p) if p.macs.is_empty()));
        assert!(m.prefs.macs.is_empty());
        assert!(m.dialog.is_none());
        assert_eq!(m.status, texts::forgot("Mac"));
        // Without a saved Mac the click on a NotFound row opens its details.
        m.update(Msg::SavedMacs(vec![saved("M", "Mac", 1)]), now);
        let key = m.rows_mine[0].key.clone();
        m.update(Msg::ClickRow { key }, now);
        assert!(matches!(
            &m.dialog,
            Some(Dialog::MacDetails { mac_id, problem: None }) if mac_id == "M"
        ));
    }

    #[test]
    fn set_remote_host_validates() {
        let (mut m, now) = model_with(vec![saved("M", "Mac", 1)]);
        m.update(
            Msg::OpenDialog(Dialog::MacDetails {
                mac_id: "M".into(),
                problem: None,
            }),
            now,
        );
        let fx = m.update(
            Msg::SetRemoteHost {
                mac_id: "M".into(),
                host: Some("bad host".into()),
            },
            now,
        );
        assert!(fx.is_empty());
        assert!(matches!(
            &m.dialog,
            Some(Dialog::MacDetails {
                problem: Some(_),
                ..
            })
        ));
        let fx = m.update(
            Msg::SetRemoteHost {
                mac_id: "M".into(),
                host: Some(" 100.1.2.3 ".into()),
            },
            now,
        );
        assert!(matches!(fx.as_slice(), [Effect::SavePrefs(_)]));
        assert_eq!(m.prefs.macs["M"].remote_host.as_deref(), Some("100.1.2.3"));
        assert!(matches!(
            &m.dialog,
            Some(Dialog::MacDetails { problem: None, .. })
        ));
        assert!(m.rows_mine[0].has_remote);
        assert_eq!(m.rows_mine[0].word, RowWord::Remote);
        let fx = m.update(
            Msg::SetRemoteHost {
                mac_id: "M".into(),
                host: None,
            },
            now,
        );
        assert!(matches!(fx.as_slice(), [Effect::SavePrefs(_)]));
        assert!(m.prefs.macs.is_empty());
        assert_eq!(m.draft_seed.remote_host.as_deref(), Some(""));
        assert_eq!(m.rows_mine[0].word, RowWord::NotFound);
    }

    #[test]
    fn set_prefs_clamps_zoom_and_fps() {
        let (mut m, now) = model_with(Vec::new());
        let p = Prefs {
            ui_zoom: 9.0,
            fps: 90,
            ..Prefs::default()
        };
        let fx = m.update(Msg::SetPrefs(p), now);
        assert!(matches!(&fx[0], Effect::SavePrefs(p) if p.ui_zoom == ZOOM_MAX && p.fps == 60));
        let p = Prefs {
            ui_zoom: f32::NAN,
            fps: 120,
            ..Prefs::default()
        };
        m.update(Msg::SetPrefs(p), now);
        assert_eq!((m.prefs.ui_zoom, m.prefs.fps), (1.0, 120));
        let p = Prefs {
            ui_zoom: 0.1,
            ..Prefs::default()
        };
        m.update(Msg::SetPrefs(p), now);
        assert_eq!(m.prefs.ui_zoom, ZOOM_MIN);
        // Clearing the device name puts the default back in the field.
        m.prefs.device_name = Some("Mine".into());
        let epoch = m.draft_epoch;
        m.update(Msg::SetPrefs(Prefs::default()), now);
        assert!(m.draft_epoch > epoch);
        assert_eq!(m.draft_seed.device_name, Some(m.prefs.device_name()));
    }

    #[test]
    fn connect_while_streaming_stops_first() {
        let (mut m, now) = connected_model(Some("M"));
        let f = found("Delta", None, HomeDoor::PairingRequired, "192.168.1.6");
        let fx = m.update(Msg::Connect(ConnectPlan::Unpinned(f)), now);
        assert!(matches!(
            fx.as_slice(),
            [Effect::StopStream, Effect::StartStream { .. }]
        ));
        let s = m.stream().unwrap();
        assert_eq!(s.route_desc, RouteDesc::Unpinned);
        assert!(s.mac_id.is_none());
        assert!(matches!(s.session.phase, Phase::Connecting));
        // A saved Mac whose key cannot be read shows the key-changed banner and stays put.
        let mut bad = saved("B", "Bad", 2);
        bad.fingerprint = "!!".into();
        let (mut m, now) = model_with(Vec::new());
        let fx = m.update(
            Msg::Connect(ConnectPlan::Remote {
                mac: bad,
                hosts: vec![("h".into(), 1)],
            }),
            now,
        );
        assert!(fx.is_empty());
        assert_eq!(
            m.banner.as_ref().unwrap().actions,
            vec![BannerAction::ForgetAndPair]
        );
        assert!(m.stream().is_none());
    }

    #[test]
    fn disconnect_returns_to_macs() {
        let (mut m, now) = connected_model(None);
        m.dialog = Some(Dialog::About);
        let fx = m.update(Msg::Disconnect, now);
        assert!(matches!(fx.as_slice(), [Effect::StopStream]));
        assert!(matches!(m.screen, Screen::Macs { card: None }));
        assert!(m.update(Msg::Disconnect, now).is_empty());
        // Panels and dialogs over the stream release the keys.
        let (mut m, now) = connected_model(None);
        let fx = m.update(Msg::OpenPanel(StreamPanel::MacSettings), now);
        assert!(matches!(fx.as_slice(), [Effect::ReleaseAll]));
        assert!(m.overlay_open());
        m.update(Msg::ClosePanel, now);
        let fx = m.update(Msg::OpenDialog(Dialog::About), now);
        assert!(matches!(fx.as_slice(), [Effect::ReleaseAll]));
        m.update(Msg::CloseDialog, now);
        assert!(!m.overlay_open());
        let fx = m.update(Msg::ToggleFullscreen, now);
        assert!(matches!(fx.as_slice(), [Effect::SetFullscreen(true)]));
        m.update(Msg::FullscreenChanged(false), now);
        assert!(!m.fullscreen);
    }

    #[test]
    fn frame_shown_reveals_bar_once() {
        let (mut m, now) = connected_model(None);
        assert!(!m.stream().unwrap().bar_visible(now));
        m.update(Msg::FrameShown, now);
        let s = m.stream().unwrap();
        assert_eq!(s.bar.reveal_until, Some(now + BAR_REVEAL));
        assert!(s.bar_visible(now));
        assert_eq!(s.session.first_frame_at, Some(now));
        m.update(Msg::FrameShown, now + ms(500));
        assert_eq!(m.stream().unwrap().bar.reveal_until, Some(now + BAR_REVEAL));
        // Not on the Stream screen: nothing happens.
        let (mut m, now) = model_with(Vec::new());
        assert!(m.update(Msg::FrameShown, now).is_empty());
    }
}

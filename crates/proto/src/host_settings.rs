//! The Mac's settings as a device sees and changes them: kind 16 (host to device) and kind 17
//! (device to host). Port of Sources/StreamProtocol/HostSettings.swift. The JSON keys are the
//! Swift property names; every optional field is omitted when None, as Swift's synthesized
//! `Codable` does (`encodeIfPresent`), and unknown keys are ignored.
//!
//! The module also holds `SettingsLedger`, a port of Sill's iOSClient/HostSettingsLedger.swift
//! (its rules are in the doc comment on the type).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The settings a device sees and changes (HostSettings.swift:55-80).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamSettings {
    /// Frame rate limit.
    #[serde(rename = "maxFPS")]
    pub max_fps: i64,
    /// Bits per second per 60 fps.
    pub bitrate: i64,
    /// 2 = Retina, 1 = Standard.
    #[serde(rename = "captureScale")]
    pub capture_scale: f64,
    #[serde(rename = "prioritizeSpeed")]
    pub prioritize_speed: bool,
    #[serde(rename = "virtualDisplay")]
    pub virtual_display: bool,
    /// None: a host without the setting (show no row, never send it).
    #[serde(
        rename = "directWireless",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub direct_wireless: Option<bool>,
}

/// The pipeline as it was last started (HostSettings.swift:82-97).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunningStream {
    pub width: i64,
    pub height: i64,
    pub fps: i64,
    /// Whole Mbps.
    pub mbps: i64,
    #[serde(rename = "onVirtualDisplay")]
    pub on_virtual_display: bool,
}

/// Away from home: the Mac's two qualities (HostSettings.swift:99-126). Every field is required.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AwayQuality {
    #[serde(rename = "homeBitrate")]
    pub home_bitrate: i64,
    #[serde(rename = "homeCaptureScale")]
    pub home_capture_scale: f64,
    #[serde(rename = "awayBitrate")]
    pub away_bitrate: i64,
    #[serde(rename = "awayCaptureScale")]
    pub away_capture_scale: f64,
    #[serde(rename = "thisConnectionAway")]
    pub this_connection_away: bool,
    #[serde(rename = "awayRunning")]
    pub away_running: bool,
}

/// `LinkReport.state` for a link that cannot carry the quality (HostSettings.swift:132).
pub const LINK_BEHIND: &str = "behind";
/// `LinkReport.state` the Mac's card shows; a device never does (HostSettings.swift:133).
pub const LINK_STALLED: &str = "stalled";

/// How the link to this device keeps up (HostSettings.swift:128-162).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkReport {
    pub state: String,
    #[serde(rename = "withheldPerSecond")]
    pub withheld_per_second: i64,
    pub bitrate: i64,
    #[serde(
        rename = "carriedKbps",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub carried_kbps: Option<i64>,
    #[serde(
        rename = "suggestedBitrate",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub suggested_bitrate: Option<i64>,
    #[serde(
        rename = "suggestedCaptureScale",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub suggested_capture_scale: Option<f64>,
}

impl LinkReport {
    /// The one state a device shows (HostSettings.swift:160-161).
    pub fn is_behind(&self) -> bool {
        self.state == LINK_BEHIND
    }

    /// What the Mac suggests picking (AwayCopy.suggestion): only a report with a suggested
    /// bitrate offers one, with the suggested capture scale if there is one. Token None.
    pub fn suggestion(&self) -> Option<HostSettingsChange> {
        let bitrate = self.suggested_bitrate?;
        Some(HostSettingsChange {
            bitrate: Some(bitrate),
            capture_scale: self.suggested_capture_scale,
            ..Default::default()
        })
    }
}

/// Kind 16: on connect (after the window list), on every change, and as the answer to one
/// device's change with `answering` set (HostSettings.swift:164-205).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostSettingsState {
    pub settings: StreamSettings,
    pub persistent: bool,
    #[serde(rename = "virtualDisplayAvailable")]
    pub virtual_display_available: bool,
    #[serde(
        rename = "virtualDisplayNote",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub virtual_display_note: Option<String>,
    #[serde(rename = "softwareEncoder")]
    pub software_encoder: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<RunningStream>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answering: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub away: Option<AwayQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkReport>,
}

/// One of the six settings a device may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SettingsField {
    MaxFps,
    Bitrate,
    CaptureScale,
    PrioritizeSpeed,
    VirtualDisplay,
    DirectWireless,
}

impl SettingsField {
    pub const ALL: [SettingsField; 6] = [
        SettingsField::MaxFps,
        SettingsField::Bitrate,
        SettingsField::CaptureScale,
        SettingsField::PrioritizeSpeed,
        SettingsField::VirtualDisplay,
        SettingsField::DirectWireless,
    ];
}

/// Kind 17: only the fields one control changed, as absolute values (HostSettings.swift:207-249).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostSettingsChange {
    /// Strictly increasing for this process; answered as `answering`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<i64>,
    #[serde(rename = "maxFPS", default, skip_serializing_if = "Option::is_none")]
    pub max_fps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<i64>,
    #[serde(
        rename = "captureScale",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub capture_scale: Option<f64>,
    #[serde(
        rename = "prioritizeSpeed",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub prioritize_speed: Option<bool>,
    #[serde(
        rename = "virtualDisplay",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub virtual_display: Option<bool>,
    #[serde(
        rename = "directWireless",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub direct_wireless: Option<bool>,
}

impl HostSettingsChange {
    /// All setting fields None; the token does not count (HostSettings.swift:229-233).
    pub fn is_empty(&self) -> bool {
        self.max_fps.is_none()
            && self.bitrate.is_none()
            && self.capture_scale.is_none()
            && self.prioritize_speed.is_none()
            && self.virtual_display.is_none()
            && self.direct_wireless.is_none()
    }

    /// The fields this change sets, in `SettingsField::ALL` order.
    pub fn fields(&self) -> Vec<SettingsField> {
        let set = [
            self.max_fps.is_some(),
            self.bitrate.is_some(),
            self.capture_scale.is_some(),
            self.prioritize_speed.is_some(),
            self.virtual_display.is_some(),
            self.direct_wireless.is_some(),
        ];
        SettingsField::ALL
            .into_iter()
            .zip(set)
            .filter_map(|(f, on)| on.then_some(f))
            .collect()
    }

    /// Just `field` of this change, token None.
    pub fn only(&self, field: SettingsField) -> HostSettingsChange {
        let mut r = HostSettingsChange::default();
        match field {
            SettingsField::MaxFps => r.max_fps = self.max_fps,
            SettingsField::Bitrate => r.bitrate = self.bitrate,
            SettingsField::CaptureScale => r.capture_scale = self.capture_scale,
            SettingsField::PrioritizeSpeed => r.prioritize_speed = self.prioritize_speed,
            SettingsField::VirtualDisplay => r.virtual_display = self.virtual_display,
            SettingsField::DirectWireless => r.direct_wireless = self.direct_wireless,
        }
        r
    }

    /// This change with each of `other`'s set fields laid over it; the token stays this one's.
    pub fn adding(&self, other: &HostSettingsChange) -> HostSettingsChange {
        HostSettingsChange {
            token: self.token,
            max_fps: other.max_fps.or(self.max_fps),
            bitrate: other.bitrate.or(self.bitrate),
            capture_scale: other.capture_scale.or(self.capture_scale),
            prioritize_speed: other.prioritize_speed.or(self.prioritize_speed),
            virtual_display: other.virtual_display.or(self.virtual_display),
            direct_wireless: other.direct_wireless.or(self.direct_wireless),
        }
    }

    /// `s` with this change laid over it (HostSettings.swift:235-248).
    pub fn applied_to(&self, s: &StreamSettings) -> StreamSettings {
        let mut r = s.clone();
        if let Some(v) = self.max_fps {
            r.max_fps = v;
        }
        if let Some(v) = self.bitrate {
            r.bitrate = v;
        }
        if let Some(v) = self.capture_scale {
            r.capture_scale = v;
        }
        if let Some(v) = self.prioritize_speed {
            r.prioritize_speed = v;
        }
        if let Some(v) = self.virtual_display {
            r.virtual_display = v;
        }
        if let Some(v) = self.direct_wireless {
            r.direct_wireless = Some(v);
        }
        r
    }
}

/// What a device may pick for `maxFPS` (HostSettings.swift:254-258).
pub const CHOICES_MAX_FPS: [i64; 2] = [60, 120];
/// What a device may pick for `captureScale`: Retina, Standard (HostSettings.swift:256).
pub const CHOICES_CAPTURE_SCALE: [f64; 2] = [2.0, 1.0];
/// The bitrate presets, ascending, with their names (HostSettings.swift:267-284).
pub const QUALITY_PRESETS: [(i64, &str); 7] = [
    (4_000_000, "Low"),
    (8_000_000, "Efficient"),
    (15_000_000, "Balanced"),
    (25_000_000, "High"),
    (40_000_000, "Pro"),
    (80_000_000, "Ultra"),
    (150_000_000, "Extreme"),
];

/// The preset's name for a bitrate, if it is one.
pub fn preset_name(bps: i64) -> Option<&'static str> {
    QUALITY_PRESETS
        .iter()
        .find(|(b, _)| *b == bps)
        .map(|(_, n)| *n)
}

/// "15", or "8.5" for a bitrate that is not whole megabits (HostSettings.swift:313-318).
pub fn mbps_text(bps: i64) -> String {
    if bps % 1_000_000 == 0 {
        format!("{}", bps / 1_000_000)
    } else {
        format!("{:.1}", bps as f64 / 1_000_000.0)
    }
}

/// "Balanced — 15 Mbps", or "Custom — 12 Mbps" (HostSettings.swift:286-299).
pub fn bitrate_title(bps: i64) -> String {
    match preset_name(bps) {
        Some(name) => format!("{name} \u{2014} {} Mbps", bps / 1_000_000),
        None => format!("Custom \u{2014} {} Mbps", mbps_text(bps)),
    }
}

/// The shortest time a pick waits for its answer.
pub const PICK_TIMEOUT_MIN: Duration = Duration::from_secs(4);

/// How long a pick waits for its answer: `max(4 s, 4 x worst round trip)`, or 4 s when the
/// round trip is unmeasured or not a positive finite number. Capped at a day.
pub fn pick_timeout(worst_rtt_ms: Option<f64>) -> Duration {
    match worst_rtt_ms {
        Some(ms) if ms.is_finite() && ms > 0.0 => {
            PICK_TIMEOUT_MIN.max(Duration::from_secs_f64((4.0 * ms / 1000.0).min(86_400.0)))
        }
        _ => PICK_TIMEOUT_MIN,
    }
}

/// One unanswered pick: the single field, the token it was sent under, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingPick {
    pub change: HostSettingsChange,
    pub token: i64,
    pub sent_at: Instant,
}

/// This connection's Mac settings with this device's unanswered picks laid over them. A port of
/// Sill's iOSClient/HostSettingsLedger.swift. The rules:
///
/// 1. No state on this connection: `pick` sends nothing.
/// 2. A pick sends only the fields whose value differs from what is shown (the Mac's state with
///    the pending picks laid over it).
/// 3. Each sent field is held under the pick's token, replacing an older pick for that field.
/// 4. Broadcasts (no `answering`) never clear a pick. An answer clears exactly the picks whose
///    token it carries.
/// 5. A cleared pick whose value differs from the answer's settings was refused: the control
///    returns to the Mac's value and `receive` reports the field.
/// 6. A pick unanswered for longer than the timeout returns to the Mac's value; a late answer or
///    broadcast still applies normally (and is not a refusal, since its pick is gone).
/// 7. Reset at every tear-down; never persisted.
/// 8. Only `pick`, called from a user action, produces a change.
/// 9. A field the Mac did not report (`directWireless` None) is never sent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsLedger {
    host: Option<HostSettingsState>,
    pending: BTreeMap<SettingsField, PendingPick>,
}

impl SettingsLedger {
    /// The Mac's last state (`answering` always None).
    pub fn host(&self) -> Option<&HostSettingsState> {
        self.host.as_ref()
    }

    pub fn pending(&self) -> &BTreeMap<SettingsField, PendingPick> {
        &self.pending
    }

    /// What the controls show: the Mac's settings with every pending pick laid over them.
    pub fn displayed(&self) -> Option<StreamSettings> {
        let host = self.host.as_ref()?;
        Some(
            self.pending
                .values()
                .fold(host.settings.clone(), |s, p| p.change.applied_to(&s)),
        )
    }

    pub fn pending_since(&self, field: SettingsField) -> Option<Instant> {
        self.pending.get(&field).map(|p| p.sent_at)
    }

    pub fn oldest_pending(&self) -> Option<Instant> {
        self.pending.values().map(|p| p.sent_at).min()
    }

    /// A user's pick: the change to send (with `token`), or None when nothing differs from what
    /// is shown or there is no state.
    pub fn pick(
        &mut self,
        change: &HostSettingsChange,
        token: i64,
        now: Instant,
    ) -> Option<HostSettingsChange> {
        let shown = self.displayed()?;
        let mut out = HostSettingsChange {
            token: Some(token),
            ..Default::default()
        };
        for field in change.fields() {
            if field == SettingsField::DirectWireless && shown.direct_wireless.is_none() {
                continue;
            }
            let single = change.only(field);
            if single.applied_to(&shown) == shown {
                continue;
            }
            out = out.adding(&single);
            self.pending.insert(
                field,
                PendingPick {
                    change: single,
                    token,
                    sent_at: now,
                },
            );
        }
        (!out.is_empty()).then_some(out)
    }

    /// The Mac's state arrived. Returns the fields of the answered picks that the Mac refused.
    pub fn receive(&mut self, mut state: HostSettingsState) -> BTreeSet<SettingsField> {
        let answering = state.answering.take();
        let mut refused = BTreeSet::new();
        if let Some(token) = answering {
            self.pending.retain(|field, p| {
                if p.token != token {
                    return true;
                }
                if p.change.applied_to(&state.settings) != state.settings {
                    refused.insert(*field);
                }
                false
            });
        }
        self.host = Some(state);
        refused
    }

    /// Drops picks older than `timeout`; true if any went.
    pub fn expire(&mut self, now: Instant, timeout: Duration) -> bool {
        let before = self.pending.len();
        self.pending
            .retain(|_, p| now.saturating_duration_since(p.sent_at) <= timeout);
        self.pending.len() != before
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(bitrate_title(15_000_000), "Balanced \u{2014} 15 Mbps");
        assert_eq!(bitrate_title(12_000_000), "Custom \u{2014} 12 Mbps");
        assert_eq!(bitrate_title(8_500_000), "Custom \u{2014} 8.5 Mbps");
        assert_eq!(mbps_text(150_000_000), "150");
        assert_eq!(preset_name(4_000_000), Some("Low"));
        assert_eq!(preset_name(5), None);
    }

    #[test]
    fn change_is_empty_ignores_token() {
        let c = HostSettingsChange {
            token: Some(3),
            ..Default::default()
        };
        assert!(c.is_empty());
        let c = HostSettingsChange {
            direct_wireless: Some(true),
            ..Default::default()
        };
        assert!(!c.is_empty());
    }

    #[test]
    fn applied_to_lays_fields_over() {
        let s = StreamSettings {
            max_fps: 60,
            bitrate: 15_000_000,
            capture_scale: 2.0,
            prioritize_speed: false,
            virtual_display: false,
            direct_wireless: None,
        };
        let c = HostSettingsChange {
            max_fps: Some(120),
            direct_wireless: Some(true),
            ..Default::default()
        };
        let r = c.applied_to(&s);
        assert_eq!(r.max_fps, 120);
        assert_eq!(r.bitrate, 15_000_000);
        assert_eq!(r.direct_wireless, Some(true));
    }

    // ---- the ledger (ports of Tests/checks/ledger/main.swift scenarios) ----

    use serde_json::json;

    fn base() -> StreamSettings {
        StreamSettings {
            max_fps: 120,
            bitrate: 15_000_000,
            capture_scale: 2.0,
            prioritize_speed: false,
            virtual_display: false,
            direct_wireless: Some(false),
        }
    }

    fn st(
        settings: StreamSettings,
        answering: Option<i64>,
        vd_available: bool,
        note: Option<&str>,
    ) -> HostSettingsState {
        HostSettingsState {
            settings,
            persistent: true,
            virtual_display_available: vd_available,
            virtual_display_note: note.map(str::to_string),
            software_encoder: false,
            stream: None,
            answering,
            away: None,
            link: None,
        }
    }

    fn with_base() -> SettingsLedger {
        let mut l = SettingsLedger::default();
        l.receive(st(base(), None, true, None));
        l
    }

    fn bitrate(v: i64) -> HostSettingsChange {
        HostSettingsChange {
            bitrate: Some(v),
            ..Default::default()
        }
    }

    fn fps(v: i64) -> HostSettingsChange {
        HostSettingsChange {
            max_fps: Some(v),
            ..Default::default()
        }
    }

    fn dw(v: bool) -> HostSettingsChange {
        HostSettingsChange {
            direct_wireless: Some(v),
            ..Default::default()
        }
    }

    fn vd(v: bool) -> HostSettingsChange {
        HostSettingsChange {
            virtual_display: Some(v),
            ..Default::default()
        }
    }

    fn tok(token: i64, c: HostSettingsChange) -> HostSettingsChange {
        HostSettingsChange {
            token: Some(token),
            ..c
        }
    }

    #[test]
    fn no_state_sends_nothing() {
        let mut l = SettingsLedger::default();
        assert_eq!(l.pick(&fps(60), 1, Instant::now()), None);
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed(), None);
    }

    #[test]
    fn pick_shows_at_once_and_its_answer_clears_it() {
        let t0 = Instant::now();
        let mut l = with_base();
        let out = l.pick(&bitrate(25_000_000), 1, t0);
        assert_eq!(out, Some(tok(1, bitrate(25_000_000))));
        assert_eq!(l.displayed().unwrap().bitrate, 25_000_000);
        assert_eq!(l.pending_since(SettingsField::Bitrate), Some(t0));
        assert_eq!(l.oldest_pending(), Some(t0));
        let mut h = base();
        h.bitrate = 25_000_000;
        assert!(l.receive(st(h.clone(), None, true, None)).is_empty());
        assert!(l.pending().contains_key(&SettingsField::Bitrate));
        assert!(l.receive(st(h, Some(1), true, None)).is_empty());
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed().unwrap().bitrate, 25_000_000);
    }

    #[test]
    fn picking_the_shown_value_sends_nothing() {
        let t0 = Instant::now();
        let mut l = with_base();
        assert_eq!(l.pick(&fps(120), 1, t0), None);
        assert!(l.pending().is_empty());
        let mixed = HostSettingsChange {
            max_fps: Some(120),
            bitrate: Some(8_000_000),
            ..Default::default()
        };
        assert_eq!(l.pick(&mixed, 2, t0), Some(tok(2, bitrate(8_000_000))));
    }

    #[test]
    fn double_pick_60_120_60_never_shows_120() {
        let t0 = Instant::now();
        let mut l = with_base();
        l.pick(&fps(60), 1, t0);
        l.pick(&fps(120), 2, t0 + Duration::from_millis(100));
        l.pick(&fps(60), 3, t0 + Duration::from_millis(200));
        let mut h = base();
        h.max_fps = 60;
        l.receive(st(h.clone(), None, true, None));
        l.receive(st(h.clone(), Some(1), true, None));
        assert_eq!(l.displayed().unwrap().max_fps, 60);
        h.max_fps = 120;
        l.receive(st(h.clone(), None, true, None));
        let r2 = l.receive(st(h.clone(), Some(2), true, None));
        assert_eq!(l.displayed().unwrap().max_fps, 60);
        assert!(r2.is_empty());
        h.max_fps = 60;
        l.receive(st(h.clone(), None, true, None));
        l.receive(st(h, Some(3), true, None));
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed().unwrap().max_fps, 60);
    }

    #[test]
    fn tapping_back_while_pending_sends() {
        let t0 = Instant::now();
        let mut l = with_base();
        l.pick(&fps(60), 1, t0);
        let back = l.pick(&fps(120), 2, t0 + Duration::from_millis(100));
        assert_eq!(back, Some(tok(2, fps(120))));
        assert_eq!(l.pick(&fps(120), 3, t0 + Duration::from_millis(200)), None);
    }

    #[test]
    fn refused_field_snaps_back() {
        let note = "Start SillHost with --virtual-display to use it.";
        let mut l = SettingsLedger::default();
        l.receive(st(base(), None, false, Some(note)));
        l.pick(&vd(true), 1, Instant::now());
        assert!(l.displayed().unwrap().virtual_display);
        let refused = l.receive(st(base(), Some(1), false, Some(note)));
        assert_eq!(refused, BTreeSet::from([SettingsField::VirtualDisplay]));
        assert!(!l.displayed().unwrap().virtual_display);
        assert_eq!(
            l.host().unwrap().virtual_display_note.as_deref(),
            Some(note)
        );
    }

    #[test]
    fn partial_refusal_keeps_the_accepted_field() {
        let mut l = with_base();
        let both = HostSettingsChange {
            bitrate: Some(8_000_000),
            virtual_display: Some(true),
            ..Default::default()
        };
        l.pick(&both, 1, Instant::now());
        let mut h = base();
        h.bitrate = 8_000_000;
        let refused = l.receive(st(h, Some(1), true, None));
        assert_eq!(refused, BTreeSet::from([SettingsField::VirtualDisplay]));
        let d = l.displayed().unwrap();
        assert_eq!(d.bitrate, 8_000_000);
        assert!(!d.virtual_display);
    }

    #[test]
    fn broadcast_keeps_a_pending_pick() {
        let mut l = with_base();
        let scale = HostSettingsChange {
            capture_scale: Some(1.0),
            ..Default::default()
        };
        l.pick(&scale, 1, Instant::now());
        let mut other = base();
        other.bitrate = 40_000_000;
        l.receive(st(other, None, true, None));
        let d = l.displayed().unwrap();
        assert_eq!(d.capture_scale, 1.0);
        assert_eq!(d.bitrate, 40_000_000);
    }

    #[test]
    fn timeout_reverts_and_late_answer_applies() {
        let t0 = Instant::now();
        let four = Duration::from_secs(4);
        let mut l = with_base();
        l.pick(&bitrate(25_000_000), 1, t0);
        assert!(!l.expire(t0 + four, four));
        assert!(l.pending().contains_key(&SettingsField::Bitrate));
        assert!(l.expire(t0 + Duration::from_millis(4010), four));
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed().unwrap().bitrate, 15_000_000);
        let mut h = base();
        h.bitrate = 25_000_000;
        assert!(l.receive(st(h, Some(1), true, None)).is_empty());
        assert_eq!(l.displayed().unwrap().bitrate, 25_000_000);
    }

    #[test]
    fn reset_forgets_everything() {
        let mut l = with_base();
        l.pick(&fps(60), 1, Instant::now());
        l.reset();
        assert_eq!(l, SettingsLedger::default());
    }

    #[test]
    fn nil_direct_wireless_is_never_sent() {
        let old = r#"{"settings":{"maxFPS":120,"bitrate":15000000,"captureScale":2,"prioritizeSpeed":false,"virtualDisplay":false},"persistent":true,"virtualDisplayAvailable":true,"softwareEncoder":false}"#;
        let s: HostSettingsState = serde_json::from_str(old).unwrap();
        assert_eq!(s.settings.direct_wireless, None);
        let mut l = SettingsLedger::default();
        l.receive(s);
        let t0 = Instant::now();
        assert_eq!(l.pick(&dw(true), 1, t0), None);
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed().unwrap().direct_wireless, None);
        assert_eq!(l.pick(&dw(false), 2, t0), None);
        let mixed = HostSettingsChange {
            bitrate: Some(25_000_000),
            direct_wireless: Some(true),
            ..Default::default()
        };
        assert_eq!(l.pick(&mixed, 3, t0), Some(tok(3, bitrate(25_000_000))));
        assert!(!l.pending().contains_key(&SettingsField::DirectWireless));
    }

    #[test]
    fn direct_wireless_pick_answer_refusal_and_timeout() {
        let t0 = Instant::now();
        let four = Duration::from_secs(4);
        let on = || {
            let mut h = base();
            h.direct_wireless = Some(true);
            h
        };

        // on at once, cleared by its answer; the wire carries only the field
        let mut l = with_base();
        let out = l.pick(&dw(true), 1, t0).unwrap();
        assert_eq!(out, tok(1, dw(true)));
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            json!({"token": 1, "directWireless": true})
        );
        assert!(l.receive(st(on(), Some(1), true, None)).is_empty());
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));

        // on -> off -> on before any answer
        let mut l = with_base();
        l.pick(&dw(true), 1, t0);
        l.pick(&dw(false), 2, t0 + Duration::from_millis(100));
        l.pick(&dw(true), 3, t0 + Duration::from_millis(200));
        let mut h = on();
        l.receive(st(h.clone(), None, true, None));
        l.receive(st(h.clone(), Some(1), true, None));
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));
        h.direct_wireless = Some(false);
        l.receive(st(h.clone(), None, true, None));
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));
        let r2 = l.receive(st(h.clone(), Some(2), true, None));
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));
        assert!(r2.is_empty());
        h.direct_wireless = Some(true);
        l.receive(st(h.clone(), None, true, None));
        l.receive(st(h, Some(3), true, None));
        assert!(l.pending().is_empty());
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));

        // refused
        let mut l = with_base();
        l.pick(&dw(true), 1, t0);
        let refused = l.receive(st(base(), Some(1), true, None));
        assert_eq!(refused, BTreeSet::from([SettingsField::DirectWireless]));
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(false));

        // unanswered: back at the timeout; a late answer still applies
        let mut l = with_base();
        l.pick(&dw(true), 1, t0);
        assert!(!l.expire(t0 + four, four));
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));
        assert!(l.expire(t0 + Duration::from_millis(4010), four));
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(false));
        assert!(l.receive(st(on(), Some(1), true, None)).is_empty());
        assert_eq!(l.displayed().unwrap().direct_wireless, Some(true));
    }

    #[test]
    fn pick_timeout_floor_and_scaling() {
        let s = Duration::from_secs;
        assert_eq!(pick_timeout(None), s(4));
        assert_eq!(pick_timeout(Some(500.0)), s(4));
        assert_eq!(pick_timeout(Some(2000.0)), s(8));
        assert_eq!(pick_timeout(Some(f64::NAN)), s(4));
        assert_eq!(pick_timeout(Some(-1.0)), s(4));
        assert_eq!(pick_timeout(Some(f64::INFINITY)), s(4));
        assert_eq!(pick_timeout(Some(1e12)), s(86_400));
    }

    #[test]
    fn suggestion_needs_a_bitrate() {
        let report = |b, c| LinkReport {
            state: LINK_BEHIND.into(),
            withheld_per_second: 1,
            bitrate: 15_000_000,
            carried_kbps: None,
            suggested_bitrate: b,
            suggested_capture_scale: c,
        };
        let both = report(Some(8_000_000), Some(1.0)).suggestion().unwrap();
        assert_eq!(both.bitrate, Some(8_000_000));
        assert_eq!(both.capture_scale, Some(1.0));
        assert_eq!(both.token, None);
        let only_bitrate = report(Some(8_000_000), None).suggestion().unwrap();
        assert_eq!(only_bitrate, bitrate(8_000_000));
        assert_eq!(report(None, Some(1.0)).suggestion(), None);
    }

    #[test]
    fn fields_only_and_adding() {
        let c = HostSettingsChange {
            token: Some(9),
            max_fps: Some(60),
            virtual_display: Some(true),
            ..Default::default()
        };
        assert_eq!(
            c.fields(),
            vec![SettingsField::MaxFps, SettingsField::VirtualDisplay]
        );
        assert_eq!(c.only(SettingsField::MaxFps), fps(60));
        let m = fps(120).adding(&bitrate(8_000_000));
        assert_eq!(m.max_fps, Some(120));
        assert_eq!(m.bitrate, Some(8_000_000));
        assert_eq!(tok(5, fps(60)).adding(&fps(120)).token, Some(5));
        assert_eq!(tok(5, fps(60)).adding(&fps(120)).max_fps, Some(120));
    }

    #[test]
    fn wire_tolerance() {
        let c: HostSettingsChange = serde_json::from_str(r#"{"bitrate":8000000}"#).unwrap();
        assert_eq!(c.token, None);
        let c: HostSettingsChange =
            serde_json::from_str(r#"{"token":2,"bitrate":8000000,"sharpen":3}"#).unwrap();
        assert_eq!(c.bitrate, Some(8_000_000));
        assert!(serde_json::from_str::<HostSettingsChange>("{").is_err());
        assert!(
            serde_json::from_str::<HostSettingsState>(r#"{"settings":{"maxFPS":60}}"#).is_err()
        );
    }
}

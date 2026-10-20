//! The Mac's settings as a device sees and changes them: kind 16 (host to device) and kind 17
//! (device to host). Port of Sources/StreamProtocol/HostSettings.swift. The JSON keys are the
//! Swift property names; every optional field is omitted when None, as Swift's synthesized
//! `Codable` does (`encodeIfPresent`), and unknown keys are ignored.
//!
//! The unanswered-pick ledger (HostSettingsLedger.swift) is Phase 2 and not here yet.

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
}

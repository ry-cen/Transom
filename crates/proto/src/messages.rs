//! JSON payloads, Swift `Codable` style. Ports of Switcher.swift:4-83, Input.swift:6-38,
//! Viewport.swift:11-72, Compatibility.swift:30-117 and Remote.swift:16-236.
//!
//! Enums with associated values encode as `{"case":{"_0":positional,"label":value}}`; a nil
//! optional is omitted; unknown keys are ignored on decode.

use crate::identity::{fingerprint_of_point, mac_id};
use crate::safetext;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// `StreamSource` (Switcher.swift:4-20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreamSource {
    #[serde(rename = "none")]
    None {},
    #[serde(rename = "desktop")]
    Desktop {},
    #[serde(rename = "window")]
    Window {
        #[serde(rename = "_0")]
        id: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(rename = "bundleID")]
    pub bundle_id: String,
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    #[serde(rename = "bundleID")]
    pub bundle_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowList {
    #[serde(rename = "macName")]
    pub mac_name: String,
    pub windows: Vec<WindowInfo>,
    pub active: StreamSource,
    #[serde(rename = "launchID", default, skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<String>,
    #[serde(
        rename = "hostVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub host_version: Option<String>,
    #[serde(rename = "protocol", default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LaunchApp {
    #[serde(rename = "bundleID")]
    pub bundle_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowAction {
    Close,
    Minimize,
    FullScreen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowCommand {
    pub id: u32,
    pub action: WindowAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PointerAction {
    Move,
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollPhase {
    Began,
    Ended,
    MomentumBegan,
    MomentumEnded,
}

/// `InputEvent` (Input.swift:6-38). Coordinates are fractions of the video frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    #[serde(rename = "pointer")]
    Pointer {
        #[serde(rename = "_0")]
        action: PointerAction,
        x: f64,
        y: f64,
    },
    #[serde(rename = "scroll")]
    Scroll { x: f64, y: f64, dx: f64, dy: f64 },
    #[serde(rename = "text")]
    Text {
        #[serde(rename = "_0")]
        text: String,
    },
    #[serde(rename = "scrollGesture")]
    ScrollGesture {
        #[serde(rename = "_0")]
        phase: ScrollPhase,
        x: f64,
        y: f64,
    },
    #[serde(rename = "key")]
    Key {
        #[serde(rename = "hidUsage")]
        hid_usage: u16,
        down: bool,
        modifiers: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(
        rename = "localCursor",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub local_cursor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientStats {
    pub fps: i64,
    #[serde(rename = "frameAgeMs")]
    pub frame_age_ms: i64,
    #[serde(rename = "rttMs")]
    pub rtt_ms: i64,
    pub device: String,
    #[serde(
        rename = "frameAgeMaxMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub frame_age_max_ms: Option<i64>,
    #[serde(rename = "rttMaxMs", default, skip_serializing_if = "Option::is_none")]
    pub rtt_max_ms: Option<i64>,
}

/// Kind 23 (Compatibility.swift:98-117).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    #[serde(
        rename = "appVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(rename = "protocol", default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacAddress {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<i64>,
    pub kind: String,
    pub via: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacInfo {
    pub v: i64,
    #[serde(rename = "macID")]
    pub mac_id: String,
    pub name: String,
    #[serde(rename = "issuedAt")]
    pub issued_at: f64,
    #[serde(rename = "remoteAccess")]
    pub remote_access: bool,
    #[serde(rename = "remotePort")]
    pub remote_port: i64,
    pub internet: bool,
    pub addresses: Vec<MacAddress>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedMacInfo {
    pub info: String,
    pub key: String,
    pub sig: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairRequest {
    pub v: i64,
    pub method: String,
    pub proof: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cable: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PairResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    #[serde(rename = "macID", default, skip_serializing_if = "Option::is_none")]
    pub mac_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        rename = "recognitionKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub recognition_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "triesLeft", default, skip_serializing_if = "Option::is_none")]
    pub tries_left: Option<i64>,
    #[serde(
        rename = "retryAfter",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub retry_after: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Kind 22 (Remote.swift:211-236).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Goodbye {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(
        rename = "minimumVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub minimum_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect: Option<bool>,
}

/// `PairRequest.v`.
pub const PAIR_REQUEST_VERSION: i64 = 1;

/// `PairRequest.method` values (Remote.swift:112-130).
pub mod pair_method {
    pub const QR: &str = "qr";
    pub const CODE: &str = "code";
    pub const ASK: &str = "ask";
    pub const CANCEL: &str = "cancel";
}

/// `PairResult.reason` values (Remote.swift:188-190).
pub mod pair_reason {
    pub const CODE: &str = "code";
    pub const CLOSED: &str = "closed";
    pub const EXPIRED: &str = "expired";
    pub const STOPPED: &str = "stopped";
    pub const BUSY: &str = "busy";
    pub const SHOWN: &str = "shown";
    pub const OPEN_ON_MAC: &str = "openOnMac";
    pub const LOCKED: &str = "locked";
}

/// `Goodbye.reason` values (Remote.swift:230-236).
pub mod goodbye_reason {
    pub const REMOVED: &str = "removed";
    pub const REMOTE_OFF: &str = "remoteOff";
    pub const INTERNET_OFF: &str = "internetOff";
    pub const QUIT: &str = "quit";
    pub const BUSY: &str = "busy";
    pub const PAIRING_REQUIRED: &str = "pairingRequired";
    pub const UPDATE: &str = "update";
}

const KNOWN_PAIR_REASONS: [&str; 8] = [
    pair_reason::CODE,
    pair_reason::CLOSED,
    pair_reason::EXPIRED,
    pair_reason::STOPPED,
    pair_reason::BUSY,
    pair_reason::SHOWN,
    pair_reason::OPEN_ON_MAC,
    pair_reason::LOCKED,
];

/// A goodbye that does not decode reads as reason "" (StreamClient.swift:3768-3777).
pub fn decode_goodbye(payload: &[u8]) -> Goodbye {
    from_json(payload).unwrap_or(Goodbye {
        reason: String::new(),
        message: None,
        minimum_version: None,
        reconnect: None,
    })
}

/// JSON bytes; on error, which cannot happen for these types, `{}`.
pub fn to_json<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec())
}

pub fn from_json<T: DeserializeOwned>(b: &[u8]) -> Option<T> {
    serde_json::from_slice(b).ok()
}

impl PairResult {
    /// The Mac's own words for a refusal whose reason this build does not know. Port of
    /// Remote.swift:196-209.
    pub fn unknown_reason_message(&self) -> Option<String> {
        if self.ok {
            return None;
        }
        if let Some(r) = &self.reason {
            if KNOWN_PAIR_REASONS.contains(&r.as_str()) {
                return None;
            }
        }
        let text = safetext::label(self.message.as_deref().unwrap_or(""), 300);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

impl SignedMacInfo {
    /// The MacInfo and the signer's fingerprint, when the signature verifies over the exact
    /// `info` bytes. Port of Remote.swift:86-102.
    pub fn verified(&self) -> Option<(MacInfo, [u8; 32])> {
        let info_bytes = STANDARD.decode(&self.info).ok()?;
        let key = STANDARD.decode(&self.key).ok()?;
        let sig_bytes = STANDARD.decode(&self.sig).ok()?;
        if key.len() != 65 {
            return None;
        }
        let vk = VerifyingKey::from_sec1_bytes(&key).ok()?;
        let sig = Signature::from_der(&sig_bytes).ok()?;
        vk.verify(&info_bytes, &sig).ok()?;
        let decoded: MacInfo = from_json(&info_bytes)?;
        let fp = fingerprint_of_point(&key);
        if decoded.mac_id != mac_id(&fp) {
            return None;
        }
        Some((decoded, fp))
    }

    /// Signs `info` with `secret` (the host side of Remote.swift:70-79), for tests only.
    #[doc(hidden)]
    pub fn sign_for_tests(info: &MacInfo, secret: &[u8; 32]) -> Option<SignedMacInfo> {
        let sk = SigningKey::from_slice(secret).ok()?;
        let bytes = to_json(info);
        let sig: Signature = sk.sign(&bytes);
        let point = sk.verifying_key().to_sec1_point(false);
        Some(SignedMacInfo {
            info: STANDARD.encode(&bytes),
            key: STANDARD.encode(point.as_bytes()),
            sig: STANDARD.encode(sig.to_der().as_bytes()),
        })
    }
}

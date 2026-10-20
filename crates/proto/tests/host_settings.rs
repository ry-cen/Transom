//! Goldens for kinds 16 and 17 (HostSettings.swift): the JSON keys are the Swift property names,
//! optional fields are omitted when None, and unknown keys are ignored.

use serde_json::{json, Value};
use transom_proto::host_settings::{
    AwayQuality, HostSettingsChange, HostSettingsState, LinkReport, RunningStream, StreamSettings,
};

fn round_trip(v: Value) -> HostSettingsState {
    let s: HostSettingsState = serde_json::from_value(v.clone()).expect("decodes");
    assert_eq!(serde_json::to_value(&s).expect("encodes"), v);
    s
}

#[test]
fn full_state_round_trips() {
    let v = json!({
        "settings": {
            "maxFPS": 120, "bitrate": 25000000, "captureScale": 1.5,
            "prioritizeSpeed": true, "virtualDisplay": false, "directWireless": true
        },
        "persistent": false,
        "virtualDisplayAvailable": true,
        "virtualDisplayNote": "Needs macOS 14",
        "softwareEncoder": true,
        "stream": { "width": 2880, "height": 1800, "fps": 60, "mbps": 25, "onVirtualDisplay": true },
        "answering": 7,
        "away": {
            "homeBitrate": 40000000, "homeCaptureScale": 2.5,
            "awayBitrate": 4000000, "awayCaptureScale": 1.5,
            "thisConnectionAway": true, "awayRunning": false
        },
        "link": {
            "state": "behind", "withheldPerSecond": 12, "bitrate": 25000000,
            "carriedKbps": 9000, "suggestedBitrate": 8000000, "suggestedCaptureScale": 1.5
        }
    });
    let s = round_trip(v);
    assert_eq!(
        s.settings,
        StreamSettings {
            max_fps: 120,
            bitrate: 25_000_000,
            capture_scale: 1.5,
            prioritize_speed: true,
            virtual_display: false,
            direct_wireless: Some(true),
        }
    );
    assert_eq!(
        s.stream,
        Some(RunningStream {
            width: 2880,
            height: 1800,
            fps: 60,
            mbps: 25,
            on_virtual_display: true,
        })
    );
    assert_eq!(s.answering, Some(7));
    assert_eq!(
        s.away,
        Some(AwayQuality {
            home_bitrate: 40_000_000,
            home_capture_scale: 2.5,
            away_bitrate: 4_000_000,
            away_capture_scale: 1.5,
            this_connection_away: true,
            away_running: false,
        })
    );
    let link: LinkReport = s.link.expect("link");
    assert!(link.is_behind());
    assert_eq!(link.carried_kbps, Some(9000));
    assert_eq!(link.suggested_capture_scale, Some(1.5));
}

#[test]
fn minimal_state_has_every_option_none() {
    let v = json!({
        "settings": {
            "maxFPS": 60, "bitrate": 15000000, "captureScale": 2.5,
            "prioritizeSpeed": false, "virtualDisplay": false
        },
        "persistent": true,
        "virtualDisplayAvailable": false,
        "softwareEncoder": false
    });
    let s = round_trip(v);
    assert_eq!(s.settings.direct_wireless, None);
    assert_eq!(s.virtual_display_note, None);
    assert_eq!(s.stream, None);
    assert_eq!(s.answering, None);
    assert_eq!(s.away, None);
    assert_eq!(s.link, None);
}

#[test]
fn unknown_keys_are_ignored() {
    let v = json!({
        "settings": {
            "maxFPS": 60, "bitrate": 15000000, "captureScale": 1.5,
            "prioritizeSpeed": false, "virtualDisplay": false, "futureKnob": 3
        },
        "persistent": true,
        "virtualDisplayAvailable": false,
        "softwareEncoder": false,
        "somethingNew": { "a": 1 }
    });
    let s: HostSettingsState = serde_json::from_value(v).expect("decodes");
    assert_eq!(s.settings.max_fps, 60);
}

#[test]
fn integral_capture_scale_decodes_as_float() {
    let v = json!({
        "settings": {
            "maxFPS": 60, "bitrate": 15000000, "captureScale": 2,
            "prioritizeSpeed": false, "virtualDisplay": false
        },
        "persistent": true,
        "virtualDisplayAvailable": false,
        "softwareEncoder": false
    });
    let s: HostSettingsState = serde_json::from_value(v).expect("decodes");
    assert_eq!(s.settings.capture_scale, 2.0);
}

#[test]
fn change_encodes_only_set_fields() {
    let c = HostSettingsChange {
        token: Some(7),
        max_fps: Some(120),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_string(&c).expect("encodes"),
        r#"{"token":7,"maxFPS":120}"#
    );
    let all = json!({
        "token": 9, "maxFPS": 60, "bitrate": 8000000, "captureScale": 1.5,
        "prioritizeSpeed": true, "virtualDisplay": true, "directWireless": false
    });
    let c: HostSettingsChange = serde_json::from_value(all.clone()).expect("decodes");
    assert_eq!(serde_json::to_value(&c).expect("encodes"), all);
}

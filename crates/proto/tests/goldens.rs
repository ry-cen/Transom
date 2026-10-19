use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use transom_proto::messages::*;

/// Serializes `v` and compares with the golden as `Value`, then decodes the golden back.
fn check<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(v: T, golden: Value) {
    let ser: Value = serde_json::from_slice(&to_json(&v)).unwrap();
    assert_eq!(ser, golden);
    let back: T = serde_json::from_value(golden).unwrap();
    assert_eq!(back, v);
}

#[test]
fn pointer_move() {
    check(
        InputEvent::Pointer {
            action: PointerAction::Move,
            x: 0.25,
            y: 0.75,
        },
        json!({"pointer":{"_0":"move","x":0.25,"y":0.75}}),
    );
}

#[test]
fn pointer_left_down() {
    check(
        InputEvent::Pointer {
            action: PointerAction::LeftDown,
            x: 0.5,
            y: 0.5,
        },
        json!({"pointer":{"_0":"leftDown","x":0.5,"y":0.5}}),
    );
}

#[test]
fn pointer_right_up() {
    check(
        InputEvent::Pointer {
            action: PointerAction::RightUp,
            x: 0.125,
            y: 0.5,
        },
        json!({"pointer":{"_0":"rightUp","x":0.125,"y":0.5}}),
    );
}

#[test]
fn scroll() {
    check(
        InputEvent::Scroll {
            x: 0.5,
            y: 0.25,
            dx: 0.5,
            dy: -0.125,
        },
        json!({"scroll":{"x":0.5,"y":0.25,"dx":0.5,"dy":-0.125}}),
    );
}

#[test]
fn text() {
    check(
        InputEvent::Text { text: "hi".into() },
        json!({"text":{"_0":"hi"}}),
    );
}

#[test]
fn scroll_gesture_began() {
    check(
        InputEvent::ScrollGesture {
            phase: ScrollPhase::Began,
            x: 0.5,
            y: 0.25,
        },
        json!({"scrollGesture":{"_0":"began","x":0.5,"y":0.25}}),
    );
}

#[test]
fn scroll_gesture_momentum_ended() {
    check(
        InputEvent::ScrollGesture {
            phase: ScrollPhase::MomentumEnded,
            x: 0.5,
            y: 0.25,
        },
        json!({"scrollGesture":{"_0":"momentumEnded","x":0.5,"y":0.25}}),
    );
}

#[test]
fn key() {
    check(
        InputEvent::Key {
            hid_usage: 4,
            down: true,
            modifiers: 1 << 20,
        },
        json!({"key":{"hidUsage":4,"down":true,"modifiers":1048576}}),
    );
}

#[test]
fn source_none() {
    check(StreamSource::None {}, json!({"none":{}}));
}

#[test]
fn source_desktop() {
    check(StreamSource::Desktop {}, json!({"desktop":{}}));
}

#[test]
fn source_window() {
    check(
        StreamSource::Window { id: 123 },
        json!({"window":{"_0":123}}),
    );
}

#[test]
fn viewport_with_fps() {
    check(
        Viewport {
            width: 1117.5,
            height: 642.5,
            scale: None,
            local_cursor: None,
            fps: Some(60),
        },
        json!({"width":1117.5,"height":642.5,"fps":60}),
    );
}

#[test]
fn viewport_with_scale() {
    check(
        Viewport {
            width: 1117.5,
            height: 642.5,
            scale: Some(1.5),
            local_cursor: None,
            fps: None,
        },
        json!({"width":1117.5,"height":642.5,"scale":1.5}),
    );
}

#[test]
fn client_stats() {
    check(
        ClientStats {
            fps: 59,
            frame_age_ms: 12,
            rtt_ms: 3,
            device: "Transom (Linux)".into(),
            frame_age_max_ms: Some(40),
            rtt_max_ms: Some(9),
        },
        json!({"fps":59,"frameAgeMs":12,"rttMs":3,"device":"Transom (Linux)","frameAgeMaxMs":40,"rttMaxMs":9}),
    );
}

#[test]
fn hello() {
    check(
        Hello {
            app_version: Some("0.5.1".into()),
            build: Some("transom 0.1.0".into()),
            protocol: Some(1),
            device: Some("Transom (Linux)".into()),
        },
        json!({"appVersion":"0.5.1","build":"transom 0.1.0","protocol":1,"device":"Transom (Linux)"}),
    );
}

#[test]
fn hello_empty() {
    check(Hello::default(), json!({}));
}

#[test]
fn launch_app() {
    check(
        LaunchApp {
            bundle_id: "com.apple.Safari".into(),
        },
        json!({"bundleID":"com.apple.Safari"}),
    );
}

#[test]
fn window_command() {
    check(
        WindowCommand {
            id: 7,
            action: WindowAction::FullScreen,
        },
        json!({"id":7,"action":"fullScreen"}),
    );
}

#[test]
fn pair_request_code() {
    check(
        PairRequest {
            v: 1,
            method: "code".into(),
            proof: "abc".into(),
            name: "Transom (Linux)".into(),
            model: Some("transom-linux".into()),
            cable: None,
        },
        json!({"v":1,"method":"code","proof":"abc","name":"Transom (Linux)","model":"transom-linux"}),
    );
}

#[test]
fn pair_request_ask() {
    check(
        PairRequest {
            v: 1,
            method: "ask".into(),
            proof: "".into(),
            name: "x".into(),
            model: None,
            cable: None,
        },
        json!({"v":1,"method":"ask","proof":"","name":"x"}),
    );
}

#[test]
fn goodbye_quit() {
    check(
        Goodbye {
            reason: "quit".into(),
            message: None,
            minimum_version: None,
            reconnect: None,
        },
        json!({"reason":"quit"}),
    );
}

#[test]
fn decode_swift_integers() {
    let e: InputEvent = from_json(br#"{"scroll":{"x":1,"y":0,"dx":0,"dy":-1}}"#).unwrap();
    assert_eq!(
        e,
        InputEvent::Scroll {
            x: 1.0,
            y: 0.0,
            dx: 0.0,
            dy: -1.0
        }
    );
}

#[test]
fn windowlist_ignores_unknown() {
    let j = br#"{"macName":"M","windows":[{"id":5,"title":"T","appName":"A","bundleID":"b","width":800,"height":600}],"active":{"window":{"_0":5}},"protocol":1,"gestures":1,"clipboard":1,"futureKey":true}"#;
    let w: WindowList = from_json(j).unwrap();
    assert_eq!(w.protocol, Some(1));
    assert_eq!(w.active, StreamSource::Window { id: 5 });
    assert_eq!(w.windows.len(), 1);
    assert_eq!(w.launch_id, None);
}

#[test]
fn goodbye_garbage_is_empty_reason() {
    assert_eq!(decode_goodbye(b"not json").reason, "");
    assert_eq!(decode_goodbye(br#"{"reason":"removed"}"#).reason, "removed");
}

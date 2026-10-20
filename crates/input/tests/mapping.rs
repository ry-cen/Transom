use serde_json::json;
use transom_input::{hid_usage, KeyMapper, KeymapMode, PointerMapper};
use transom_proto::geometry::Rect;
use transom_proto::messages::{InputEvent, PointerAction, ScrollPhase};
use winit::dpi::PhysicalPosition;
use winit::event::{MouseButton, MouseScrollDelta, TouchPhase};
use winit::keyboard::KeyCode;

fn key(usage: u16, down: bool, modifiers: u64) -> InputEvent {
    InputEvent::Key {
        hid_usage: usage,
        down,
        modifiers,
    }
}

fn ptr(action: PointerAction, x: f64, y: f64) -> InputEvent {
    InputEvent::Pointer { action, x, y }
}

fn rect() -> Rect {
    Rect {
        x: 100.0,
        y: 0.0,
        width: 200.0,
        height: 100.0,
    }
}

fn mapper() -> PointerMapper {
    let mut p = PointerMapper::new();
    p.set_layout(Some(rect()), 1.0);
    p
}

#[test]
fn letters_and_digits() {
    assert_eq!(hid_usage(KeyCode::KeyA), Some(0x04));
    assert_eq!(hid_usage(KeyCode::Digit0), Some(0x27));
    assert_eq!(hid_usage(KeyCode::Numpad5), Some(0x22));
}

#[test]
fn pc_ctrl_c_is_command_c() {
    let mut k = KeyMapper::new(KeymapMode::Pc);
    let mut out = Vec::new();
    out.extend(k.key(KeyCode::ControlLeft, true, false));
    out.extend(k.key(KeyCode::KeyC, true, false));
    out.extend(k.key(KeyCode::KeyC, false, false));
    out.extend(k.key(KeyCode::ControlLeft, false, false));
    let m = 1u64 << 20;
    assert_eq!(
        out,
        vec![
            key(0xE3, true, m),
            key(0x06, true, m),
            key(0x06, false, m),
            key(0xE3, false, 0)
        ]
    );
    let mut k = KeyMapper::new(KeymapMode::Pc);
    k.key(KeyCode::ControlLeft, true, false);
    let ev = k.key(KeyCode::KeyA, true, false);
    assert_eq!(
        serde_json::to_value(&ev[0]).unwrap(),
        json!({"key":{"hidUsage":4,"down":true,"modifiers":1048576}})
    );
}

#[test]
fn mac_mode_keeps_control() {
    let mut k = KeyMapper::new(KeymapMode::Mac);
    assert_eq!(
        k.key(KeyCode::ControlLeft, true, false),
        vec![key(0xE0, true, 1 << 18)]
    );
}

#[test]
fn caps_toggles_bit_only() {
    let mut k = KeyMapper::new(KeymapMode::Pc);
    assert!(k.key(KeyCode::CapsLock, true, false).is_empty());
    assert!(k.key(KeyCode::CapsLock, false, false).is_empty());
    assert_eq!(
        k.key(KeyCode::KeyA, true, false),
        vec![key(4, true, 1 << 16)]
    );
    k.key(KeyCode::KeyA, false, false);
    assert!(k.key(KeyCode::CapsLock, true, false).is_empty());
    assert!(k.key(KeyCode::CapsLock, false, false).is_empty());
    assert_eq!(k.key(KeyCode::KeyA, true, false), vec![key(4, true, 0)]);
}

#[test]
fn repeat_sends_extra_down() {
    let mut k = KeyMapper::new(KeymapMode::Pc);
    k.key(KeyCode::KeyA, true, false);
    assert_eq!(k.key(KeyCode::KeyA, true, true), vec![key(4, true, 0)]);
    assert_eq!(k.key(KeyCode::KeyA, false, false), vec![key(4, false, 0)]);
}

#[test]
fn stray_release_ignored() {
    let mut k = KeyMapper::new(KeymapMode::Pc);
    assert!(k.key(KeyCode::KeyA, false, false).is_empty());
}

#[test]
fn release_all_modifiers_last() {
    let mut k = KeyMapper::new(KeymapMode::Mac);
    k.key(KeyCode::ShiftLeft, true, false);
    k.key(KeyCode::KeyB, true, false);
    k.key(KeyCode::KeyA, true, false);
    assert_eq!(
        k.release_all(),
        vec![
            key(0x04, false, 1 << 17),
            key(0x05, false, 1 << 17),
            key(0xE1, false, 0)
        ]
    );
    assert!(k.release_all().is_empty());
}

#[test]
fn unmapped_dropped() {
    let mut k = KeyMapper::new(KeymapMode::Pc);
    assert!(k.key(KeyCode::IntlBackslash, true, false).is_empty());
    assert!(k.key(KeyCode::F13, true, false).is_empty());
}

#[test]
fn pointer_inside_and_outside() {
    let mut p = mapper();
    assert_eq!(
        p.moved(200.0, 50.0),
        Some(ptr(PointerAction::Move, 0.5, 0.5))
    );
    assert!(p.cursor_inside());
    assert_eq!(p.moved(50.0, 50.0), None);
    assert!(!p.cursor_inside());
}

#[test]
fn drag_clamps() {
    let mut p = mapper();
    p.moved(200.0, 50.0);
    assert_eq!(
        p.button(MouseButton::Left, true),
        Some(ptr(PointerAction::LeftDown, 0.5, 0.5))
    );
    assert_eq!(
        p.moved(50.0, 50.0),
        Some(ptr(PointerAction::Move, 0.0, 0.5))
    );
    assert_eq!(
        p.button(MouseButton::Left, false),
        Some(ptr(PointerAction::LeftUp, 0.0, 0.5))
    );
    assert_eq!(p.moved(50.0, 50.0), None);
}

#[test]
fn down_outside_ignored() {
    let mut p = mapper();
    p.moved(50.0, 50.0);
    assert_eq!(p.button(MouseButton::Left, true), None);
    assert_eq!(p.button(MouseButton::Left, false), None);
    assert_eq!(p.button(MouseButton::Middle, true), None);
}

#[test]
fn wheel_line_sign() {
    let mut p = PointerMapper::new();
    p.set_layout(
        Some(Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 400.0,
        }),
        2.0,
    );
    p.moved(200.0, 200.0);
    let out = p.scroll(MouseScrollDelta::LineDelta(0.0, 1.0), TouchPhase::Moved);
    assert_eq!(
        out,
        vec![InputEvent::Scroll {
            x: 0.5,
            y: 0.5,
            dx: 0.0,
            dy: 0.2
        }]
    );
}

#[test]
fn pixel_gesture_phases() {
    let mut p = mapper();
    p.moved(200.0, 50.0);
    let d = |x, y| MouseScrollDelta::PixelDelta(PhysicalPosition::new(x, y));
    let mut out = p.scroll(d(20.0, 10.0), TouchPhase::Started);
    out.extend(p.scroll(d(0.0, -10.0), TouchPhase::Moved));
    out.extend(p.scroll(d(0.0, 0.0), TouchPhase::Ended));
    assert_eq!(
        out,
        vec![
            InputEvent::ScrollGesture {
                phase: ScrollPhase::Began,
                x: 0.5,
                y: 0.5
            },
            InputEvent::Scroll {
                x: 0.5,
                y: 0.5,
                dx: 0.1,
                dy: 0.1
            },
            InputEvent::Scroll {
                x: 0.5,
                y: 0.5,
                dx: 0.0,
                dy: -0.1
            },
            InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x: 0.5,
                y: 0.5
            },
        ]
    );
}

#[test]
fn buttons_held_tracks_down_up_and_release_all() {
    let mut p = mapper();
    p.moved(200.0, 50.0);
    assert!(!p.buttons_held());
    p.button(MouseButton::Right, true);
    assert!(p.buttons_held());
    p.button(MouseButton::Right, false);
    assert!(!p.buttons_held());
    p.button(MouseButton::Left, true);
    assert!(p.buttons_held());
    p.release_all();
    assert!(!p.buttons_held());
}

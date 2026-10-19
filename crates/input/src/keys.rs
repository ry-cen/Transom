//! Physical keys to Sill `key` events.

use std::collections::BTreeSet;

use transom_proto::messages::InputEvent;
use winit::keyboard::KeyCode;

/// Which modifier layout the local keyboard uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeymapMode {
    /// Ctrl acts as Command and the Windows/Super key as Control.
    Pc,
    /// Keys keep their names (Control is Control, Command/Super is Command).
    Mac,
}

impl KeymapMode {
    /// Mac on macOS, Pc elsewhere.
    pub fn default_for_os() -> Self {
        if cfg!(target_os = "macos") {
            KeymapMode::Mac
        } else {
            KeymapMode::Pc
        }
    }
}

/// The HID usage of a non-modifier physical key, or None (dropped). Modifiers and caps lock are
/// handled by `KeyMapper`. Only usages the host maps are returned (`KeyStrokes.swift:185-232`).
pub fn hid_usage(code: KeyCode) -> Option<u16> {
    use KeyCode::*;
    let u = match code {
        KeyA => 0x04,
        KeyB => 0x05,
        KeyC => 0x06,
        KeyD => 0x07,
        KeyE => 0x08,
        KeyF => 0x09,
        KeyG => 0x0A,
        KeyH => 0x0B,
        KeyI => 0x0C,
        KeyJ => 0x0D,
        KeyK => 0x0E,
        KeyL => 0x0F,
        KeyM => 0x10,
        KeyN => 0x11,
        KeyO => 0x12,
        KeyP => 0x13,
        KeyQ => 0x14,
        KeyR => 0x15,
        KeyS => 0x16,
        KeyT => 0x17,
        KeyU => 0x18,
        KeyV => 0x19,
        KeyW => 0x1A,
        KeyX => 0x1B,
        KeyY => 0x1C,
        KeyZ => 0x1D,
        Digit1 | Numpad1 => 0x1E,
        Digit2 | Numpad2 => 0x1F,
        Digit3 | Numpad3 => 0x20,
        Digit4 | Numpad4 => 0x21,
        Digit5 | Numpad5 => 0x22,
        Digit6 | Numpad6 => 0x23,
        Digit7 | Numpad7 => 0x24,
        Digit8 | Numpad8 => 0x25,
        Digit9 | Numpad9 => 0x26,
        Digit0 | Numpad0 => 0x27,
        Enter | NumpadEnter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus | NumpadSubtract => 0x2D,
        Equal | NumpadEqual => 0x2E,
        BracketLeft => 0x2F,
        BracketRight => 0x30,
        Backslash => 0x31,
        Semicolon => 0x33,
        Quote => 0x34,
        Backquote => 0x35,
        Comma => 0x36,
        Period | NumpadDecimal => 0x37,
        Slash | NumpadDivide => 0x38,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        _ => return None,
    };
    Some(u)
}

/// The HID usage of a modifier key in `mode`, or None for any other key.
fn modifier_usage(code: KeyCode, mode: KeymapMode) -> Option<u16> {
    use KeyCode::*;
    let pc = mode == KeymapMode::Pc;
    Some(match code {
        ControlLeft => {
            if pc {
                0xE3
            } else {
                0xE0
            }
        }
        ControlRight => {
            if pc {
                0xE7
            } else {
                0xE4
            }
        }
        SuperLeft => {
            if pc {
                0xE0
            } else {
                0xE3
            }
        }
        SuperRight => {
            if pc {
                0xE4
            } else {
                0xE7
            }
        }
        AltLeft => 0xE2,
        AltRight => 0xE6,
        ShiftLeft => 0xE1,
        ShiftRight => 0xE5,
        _ => return None,
    })
}

/// Tracks held keys and the modifier bits (`KeyStrokes.swift:157-170`: bit 16 caps lock, 17
/// shift, 18 control, 19 option, 20 command).
pub struct KeyMapper {
    mode: KeymapMode,
    held: BTreeSet<u16>,
    caps: bool,
}

impl KeyMapper {
    pub fn new(mode: KeymapMode) -> Self {
        KeyMapper {
            mode,
            held: BTreeSet::new(),
            caps: false,
        }
    }

    /// One physical key event to zero or more `InputEvent::Key`.
    pub fn key(&mut self, code: KeyCode, pressed: bool, repeat: bool) -> Vec<InputEvent> {
        if code == KeyCode::CapsLock {
            if pressed && !repeat {
                self.caps = !self.caps;
            }
            return Vec::new();
        }
        let usage = match modifier_usage(code, self.mode).or_else(|| hid_usage(code)) {
            Some(u) => u,
            None => return Vec::new(),
        };
        if pressed {
            self.held.insert(usage);
            vec![self.event(usage, true)]
        } else if self.held.remove(&usage) {
            vec![self.event(usage, false)]
        } else {
            Vec::new()
        }
    }

    /// Every held key up, modifiers last (focus lost, window closing, session switch).
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let usages: Vec<u16> = self.held.iter().copied().collect();
        let mut out = Vec::with_capacity(usages.len());
        for u in usages {
            self.held.remove(&u);
            out.push(self.event(u, false));
        }
        out
    }

    pub fn modifiers(&self) -> u64 {
        let has = |a: u16, b: u16| self.held.contains(&a) || self.held.contains(&b);
        let mut m = 0u64;
        if self.caps {
            m |= 1 << 16;
        }
        if has(0xE1, 0xE5) {
            m |= 1 << 17;
        }
        if has(0xE0, 0xE4) {
            m |= 1 << 18;
        }
        if has(0xE2, 0xE6) {
            m |= 1 << 19;
        }
        if has(0xE3, 0xE7) {
            m |= 1 << 20;
        }
        m
    }

    fn event(&self, usage: u16, down: bool) -> InputEvent {
        InputEvent::Key {
            hid_usage: usage,
            down,
            modifiers: self.modifiers(),
        }
    }
}

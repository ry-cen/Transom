//! Who has the keyboard (docs/ui-spec.md section 6.1). Pure: the shell recomputes the mode
//! before every key event and releases everything held on the Mac at each Mac-to-Transom change.

/// The conditions of section 6.1, as the shell sees them before a key event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeInputs {
    /// The session's phase is Connected.
    pub connected: bool,
    /// Keyboard forwarding is on (off when the Mac is this computer, unless allowed).
    pub forward_keys: bool,
    /// The window has the focus.
    pub focused: bool,
    /// A popover, panel, menu or dialog is open (`Model::overlay_open`).
    pub overlay_open: bool,
    /// A Transom text field wants the keyboard (`egui::Context::egui_wants_keyboard_input`).
    pub egui_wants_keyboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardMode {
    /// Keys and input-method events go to the Mac, never to the UI; chords are handled.
    Mac,
    /// Keys go to the UI; nothing goes to the Mac except chords.
    Transom,
}

/// Mac when every condition holds, else Transom.
pub fn keyboard_mode(i: &ModeInputs) -> KeyboardMode {
    if i.connected && i.forward_keys && i.focused && !i.overlay_open && !i.egui_wants_keyboard {
        KeyboardMode::Mac
    } else {
        KeyboardMode::Transom
    }
}

/// True when going from `prev` to `next` must release every held key and button on the Mac.
pub fn transition(prev: KeyboardMode, next: KeyboardMode) -> bool {
    prev == KeyboardMode::Mac && next == KeyboardMode::Transom
}

/// True when the Windows keyboard hook must take the OS-reserved keys (the Win keys, Alt+Tab,
/// Alt+F4 and the rest; see `capture.rs`) for the Mac: the window is full screen and the Mac has
/// the keyboard. Never on other systems.
pub fn capture_wanted(mode: KeyboardMode, fullscreen: bool, on_windows: bool) -> bool {
    on_windows && fullscreen && mode == KeyboardMode::Mac
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_on() -> ModeInputs {
        ModeInputs {
            connected: true,
            forward_keys: true,
            focused: true,
            overlay_open: false,
            egui_wants_keyboard: false,
        }
    }

    #[test]
    fn mac_only_when_every_condition_holds() {
        assert_eq!(keyboard_mode(&all_on()), KeyboardMode::Mac);
        let cases: [fn(&mut ModeInputs); 5] = [
            |i| i.connected = false,
            |i| i.forward_keys = false,
            |i| i.focused = false,
            |i| i.overlay_open = true,
            |i| i.egui_wants_keyboard = true,
        ];
        for (n, change) in cases.iter().enumerate() {
            let mut i = all_on();
            change(&mut i);
            assert_eq!(keyboard_mode(&i), KeyboardMode::Transom, "case {n}");
        }
        assert_eq!(keyboard_mode(&ModeInputs::default()), KeyboardMode::Transom);
    }

    #[test]
    fn mac_to_transom_releases_everything() {
        assert!(transition(KeyboardMode::Mac, KeyboardMode::Transom));
        assert!(!transition(KeyboardMode::Transom, KeyboardMode::Mac));
        assert!(!transition(KeyboardMode::Mac, KeyboardMode::Mac));
        assert!(!transition(KeyboardMode::Transom, KeyboardMode::Transom));
    }

    #[test]
    fn capture_only_in_full_screen_mac_mode_on_windows() {
        assert!(capture_wanted(KeyboardMode::Mac, true, true));
        assert!(!capture_wanted(KeyboardMode::Mac, false, true));
        assert!(!capture_wanted(KeyboardMode::Transom, true, true));
        assert!(!capture_wanted(KeyboardMode::Mac, true, false));
        assert!(!capture_wanted(KeyboardMode::Transom, false, false));
    }
}

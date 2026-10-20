//! Which keys the Windows full-screen keyboard hook takes from Windows (docs/ui-spec.md section
//! 6.1, "Keys the OS keeps"). Pure: no Win32 calls, so it is tested on every platform;
//! `winhook.rs` (Windows only) feeds it the hook's events.
//!
//! Only the OS-reserved keys are swallowed: both Win keys, PrintScreen, Tab, Esc, F4 and Space
//! while Alt is down, and Esc while Ctrl is down. Ctrl, Alt and Shift are never swallowed, so the
//! modifiers, the chord and the AltGr rule stay on winit's path. A key's repeats and its release
//! always take the path its press took, so neither Windows nor the Mac is left with a key down.

use winit::keyboard::KeyCode;

/// The Windows virtual-key codes the hook can swallow (WinUser.h).
pub const VK_TAB: u32 = 0x09;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_SNAPSHOT: u32 = 0x2C;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;
pub const VK_F4: u32 = 0x73;

/// A key the hook took from Windows, for `StreamCore::key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysKey {
    pub code: KeyCode,
    pub pressed: bool,
    pub repeat: bool,
}

/// One low-level keyboard event, as the hook sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawKey {
    /// `KBDLLHOOKSTRUCT::vkCode`.
    pub vk: u32,
    /// A release (`LLKHF_UP`).
    pub up: bool,
    /// Alt is down (`LLKHF_ALTDOWN`).
    pub alt: bool,
    /// Ctrl is down (`GetAsyncKeyState(VK_CONTROL)`; only read for Esc).
    pub ctrl: bool,
    /// `KBDLLHOOKSTRUCT::time`, in milliseconds (wraps).
    pub time: u32,
}

/// What the hook does with one event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Windows gets it (and winit, when the window has the focus).
    Pass,
    /// Windows never sees it; the shell gets it.
    Swallow(SysKey),
}

/// The physical key of a virtual key the hook can swallow; None for every other key.
pub fn key_code(vk: u32) -> Option<KeyCode> {
    Some(match vk {
        VK_TAB => KeyCode::Tab,
        VK_ESCAPE => KeyCode::Escape,
        VK_SPACE => KeyCode::Space,
        VK_SNAPSHOT => KeyCode::PrintScreen,
        VK_LWIN => KeyCode::SuperLeft,
        VK_RWIN => KeyCode::SuperRight,
        VK_F4 => KeyCode::F4,
        _ => return None,
    })
}

/// True when a fresh press of `vk` would start something Windows keeps for itself: Start and
/// every Win shortcut, the Snipping Tool, Alt+Tab, Alt+Shift+Tab, Ctrl+Alt+Tab, Alt+Esc,
/// Ctrl+Esc, Ctrl+Shift+Esc, Alt+F4 and Alt+Space.
pub fn reserved(vk: u32, alt: bool, ctrl: bool) -> bool {
    match vk {
        VK_LWIN | VK_RWIN | VK_SNAPSHOT => true,
        VK_TAB | VK_F4 | VK_SPACE => alt,
        VK_ESCAPE => alt || ctrl,
        _ => false,
    }
}

/// Where a key's press went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    Up,
    Passed,
    Swallowed,
}

/// A swallowed key whose next press comes later than this after its last event cannot be a
/// typematic repeat (Windows' longest repeat delay is 1 s): its release was lost, for example to
/// the secure desktop, so the press is a fresh one.
const STALE_MS: u32 = 1500;

/// The hook's memory: where each key's press went, and whether capture is on.
pub struct Capture {
    paths: [Path; 256],
    /// The time of the last event of each key.
    times: [u32; 256],
    active: bool,
}

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture {
    pub const fn new() -> Self {
        Capture {
            paths: [Path::Up; 256],
            times: [0; 256],
            active: false,
        }
    }

    /// Capture starts. `held(vk)` says which keys Windows already has down: their repeats and
    /// releases keep going to Windows.
    pub fn activate(&mut self, held: impl Fn(u32) -> bool) {
        for (vk, p) in (0u32..).zip(self.paths.iter_mut()) {
            if *p == Path::Up && held(vk) {
                *p = Path::Passed;
            }
        }
        self.active = true;
    }

    /// Capture stops. A swallowed key stays swallowed until it is released: Windows never saw
    /// its press, and the shell has already released everything on the Mac.
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Inactive with no key swallowed: the hook can be removed.
    pub fn idle(&self) -> bool {
        !self.active && !self.paths.contains(&Path::Swallowed)
    }

    /// Forgets every key and stops (the hook was removed).
    pub fn reset(&mut self) {
        self.paths = [Path::Up; 256];
        self.times = [0; 256];
        self.active = false;
    }

    /// One event. `foreground`: Transom's window is the foreground window.
    pub fn on_key(&mut self, k: RawKey, foreground: bool) -> Verdict {
        let Some(path) = self.paths.get_mut(k.vk as usize) else {
            return Verdict::Pass;
        };
        let last = std::mem::replace(&mut self.times[k.vk as usize], k.time);
        let mut was = *path;
        if !k.up && was == Path::Swallowed && k.time.wrapping_sub(last) > STALE_MS {
            was = Path::Up;
        }
        if k.up {
            *path = Path::Up;
            return match (was, key_code(k.vk)) {
                (Path::Swallowed, Some(code)) => Verdict::Swallow(SysKey {
                    code,
                    pressed: false,
                    repeat: false,
                }),
                _ => Verdict::Pass,
            };
        }
        match was {
            Path::Swallowed => match key_code(k.vk) {
                Some(code) => Verdict::Swallow(SysKey {
                    code,
                    pressed: true,
                    repeat: true,
                }),
                None => Verdict::Pass,
            },
            Path::Passed => Verdict::Pass,
            Path::Up => {
                if self.active && foreground && reserved(k.vk, k.alt, k.ctrl) {
                    if let Some(code) = key_code(k.vk) {
                        *path = Path::Swallowed;
                        return Verdict::Swallow(SysKey {
                            code,
                            pressed: true,
                            repeat: false,
                        });
                    }
                }
                *path = Path::Passed;
                Verdict::Pass
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u32 = 0x41;
    const VK_L: u32 = 0x4C;

    fn down(vk: u32) -> RawKey {
        RawKey {
            vk,
            ..RawKey::default()
        }
    }

    fn alt_down(vk: u32) -> RawKey {
        RawKey {
            vk,
            alt: true,
            ..RawKey::default()
        }
    }

    fn up(vk: u32) -> RawKey {
        RawKey {
            vk,
            up: true,
            ..RawKey::default()
        }
    }

    fn press(code: KeyCode) -> Verdict {
        Verdict::Swallow(SysKey {
            code,
            pressed: true,
            repeat: false,
        })
    }

    fn repeat(code: KeyCode) -> Verdict {
        Verdict::Swallow(SysKey {
            code,
            pressed: true,
            repeat: true,
        })
    }

    fn release(code: KeyCode) -> Verdict {
        Verdict::Swallow(SysKey {
            code,
            pressed: false,
            repeat: false,
        })
    }

    fn active() -> Capture {
        let mut c = Capture::new();
        c.activate(|_| false);
        c
    }

    #[test]
    fn maps_every_swallowable_vk_and_nothing_else() {
        let cases = [
            (VK_TAB, KeyCode::Tab),
            (VK_ESCAPE, KeyCode::Escape),
            (VK_SPACE, KeyCode::Space),
            (VK_SNAPSHOT, KeyCode::PrintScreen),
            (VK_LWIN, KeyCode::SuperLeft),
            (VK_RWIN, KeyCode::SuperRight),
            (VK_F4, KeyCode::F4),
        ];
        for (vk, code) in cases {
            assert_eq!(key_code(vk), Some(code), "vk {vk:#x}");
        }
        for vk in 0..=255u32 {
            if reserved(vk, true, true) {
                assert!(
                    key_code(vk).is_some(),
                    "reserved vk {vk:#x} has no key code"
                );
            }
        }
        assert_eq!(key_code(VK_A), None);
        assert_eq!(key_code(0xA2), None, "VK_LCONTROL");
    }

    #[test]
    fn reserved_combinations() {
        for (alt, ctrl) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(reserved(VK_LWIN, alt, ctrl));
            assert!(reserved(VK_RWIN, alt, ctrl));
            assert!(reserved(VK_SNAPSHOT, alt, ctrl));
            assert_eq!(reserved(VK_TAB, alt, ctrl), alt);
            assert_eq!(reserved(VK_F4, alt, ctrl), alt);
            assert_eq!(reserved(VK_SPACE, alt, ctrl), alt);
            assert_eq!(reserved(VK_ESCAPE, alt, ctrl), alt || ctrl);
        }
    }

    #[test]
    fn chord_keys_and_modifiers_are_never_captured() {
        // B D W A S F Q, Shift/Ctrl/Alt (generic, left, right), CapsLock.
        let keys = [
            0x42, 0x44, 0x57, 0x41, 0x53, 0x46, 0x51, 0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3,
            0xA4, 0xA5, 0x14,
        ];
        for vk in keys {
            for (alt, ctrl) in [(false, false), (true, false), (false, true), (true, true)] {
                assert!(!reserved(vk, alt, ctrl), "vk {vk:#x}");
                let mut c = active();
                let d = RawKey {
                    vk,
                    up: false,
                    alt,
                    ctrl,
                    time: 0,
                };
                assert_eq!(c.on_key(d, true), Verdict::Pass, "vk {vk:#x} down");
                assert_eq!(c.on_key(RawKey { up: true, ..d }, true), Verdict::Pass);
            }
        }
    }

    #[test]
    fn win_key_goes_to_the_mac_with_its_repeats_and_release() {
        let mut c = active();
        assert_eq!(c.on_key(down(VK_LWIN), true), press(KeyCode::SuperLeft));
        assert_eq!(c.on_key(down(VK_LWIN), true), repeat(KeyCode::SuperLeft));
        // Win+L style: the other key is not reserved and stays with Windows (and winit).
        assert_eq!(c.on_key(down(VK_L), true), Verdict::Pass);
        assert_eq!(c.on_key(up(VK_L), true), Verdict::Pass);
        assert_eq!(c.on_key(up(VK_LWIN), true), release(KeyCode::SuperLeft));
        assert_eq!(
            c.on_key(up(VK_LWIN), true),
            Verdict::Pass,
            "a second release passes"
        );
        assert_eq!(c.on_key(down(VK_RWIN), true), press(KeyCode::SuperRight));
    }

    #[test]
    fn nothing_is_captured_when_inactive_or_in_the_background() {
        let mut c = Capture::new();
        assert_eq!(c.on_key(down(VK_LWIN), true), Verdict::Pass);
        assert_eq!(c.on_key(up(VK_LWIN), true), Verdict::Pass);
        let mut c = active();
        assert_eq!(c.on_key(down(VK_LWIN), false), Verdict::Pass);
        // Its release follows its press, even once Transom is in front again.
        assert_eq!(c.on_key(up(VK_LWIN), true), Verdict::Pass);
    }

    #[test]
    fn release_follows_press_when_alt_is_let_go_first() {
        let mut c = active();
        assert_eq!(c.on_key(alt_down(VK_TAB), true), press(KeyCode::Tab));
        // Alt released, then Tab: the release has no Alt flag but was swallowed at the press.
        assert_eq!(c.on_key(up(VK_TAB), true), release(KeyCode::Tab));
    }

    #[test]
    fn a_key_pressed_before_alt_stays_with_windows() {
        let mut c = active();
        assert_eq!(c.on_key(down(VK_TAB), true), Verdict::Pass);
        assert_eq!(c.on_key(alt_down(VK_TAB), true), Verdict::Pass, "repeat");
        let alt_up = RawKey {
            alt: true,
            ..up(VK_TAB)
        };
        assert_eq!(c.on_key(alt_up, true), Verdict::Pass);
        // A fresh Alt+Tab is captured.
        assert_eq!(c.on_key(alt_down(VK_TAB), true), press(KeyCode::Tab));
    }

    #[test]
    fn ctrl_esc_and_alt_keys() {
        let mut c = active();
        let ctrl_esc = RawKey {
            vk: VK_ESCAPE,
            ctrl: true,
            ..RawKey::default()
        };
        assert_eq!(c.on_key(ctrl_esc, true), press(KeyCode::Escape));
        assert_eq!(c.on_key(up(VK_ESCAPE), true), release(KeyCode::Escape));
        assert_eq!(c.on_key(down(VK_ESCAPE), true), Verdict::Pass, "plain Esc");
        assert_eq!(c.on_key(up(VK_ESCAPE), true), Verdict::Pass);
        assert_eq!(c.on_key(alt_down(VK_F4), true), press(KeyCode::F4));
        assert_eq!(c.on_key(alt_down(VK_SPACE), true), press(KeyCode::Space));
        assert_eq!(
            c.on_key(down(VK_SNAPSHOT), true),
            press(KeyCode::PrintScreen)
        );
    }

    #[test]
    fn deactivating_drains_swallowed_keys() {
        let mut c = active();
        assert_eq!(c.on_key(down(VK_LWIN), true), press(KeyCode::SuperLeft));
        c.deactivate();
        assert!(!c.active());
        assert!(!c.idle(), "Win is still down");
        assert_eq!(c.on_key(down(VK_LWIN), true), repeat(KeyCode::SuperLeft));
        assert_eq!(
            c.on_key(alt_down(VK_TAB), true),
            Verdict::Pass,
            "no new capture"
        );
        assert_eq!(c.on_key(up(VK_TAB), true), Verdict::Pass);
        assert_eq!(c.on_key(up(VK_LWIN), true), release(KeyCode::SuperLeft));
        assert!(c.idle());
    }

    #[test]
    fn keys_held_at_activation_stay_with_windows() {
        let mut c = Capture::new();
        c.activate(|vk| vk == VK_LWIN);
        assert_eq!(c.on_key(down(VK_LWIN), true), Verdict::Pass, "repeat");
        assert_eq!(c.on_key(up(VK_LWIN), true), Verdict::Pass);
        assert_eq!(c.on_key(down(VK_LWIN), true), press(KeyCode::SuperLeft));
    }

    #[test]
    fn reset_forgets_everything() {
        let mut c = active();
        assert_eq!(c.on_key(down(VK_LWIN), true), press(KeyCode::SuperLeft));
        c.reset();
        assert!(c.idle());
        assert!(!c.active());
        assert_eq!(c.on_key(up(VK_LWIN), true), Verdict::Pass);
    }

    fn down_at(vk: u32, time: u32) -> RawKey {
        RawKey {
            vk,
            time,
            ..RawKey::default()
        }
    }

    fn up_at(vk: u32, time: u32) -> RawKey {
        RawKey {
            vk,
            time,
            up: true,
            ..RawKey::default()
        }
    }

    #[test]
    fn a_stale_swallowed_key_is_a_fresh_press_when_inactive() {
        let mut c = active();
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 1000), true),
            press(KeyCode::SuperLeft)
        );
        c.deactivate();
        // The release was lost (Win+L); the next press, 5 s later, passes.
        assert_eq!(c.on_key(down_at(VK_LWIN, 6000), true), Verdict::Pass);
        assert_eq!(c.on_key(up_at(VK_LWIN, 6100), true), Verdict::Pass);
        assert!(c.idle());
    }

    #[test]
    fn a_stale_swallowed_key_is_a_fresh_press_when_active() {
        let mut c = active();
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 1000), true),
            press(KeyCode::SuperLeft)
        );
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 6000), true),
            press(KeyCode::SuperLeft)
        );
        assert_eq!(
            c.on_key(up_at(VK_LWIN, 6100), true),
            release(KeyCode::SuperLeft)
        );
    }

    #[test]
    fn a_quick_second_down_is_still_a_repeat() {
        let mut c = active();
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 1000), true),
            press(KeyCode::SuperLeft)
        );
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 1500), true),
            repeat(KeyCode::SuperLeft)
        );
        // Repeats refresh the time, so a long hold stays a hold.
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 2900), true),
            repeat(KeyCode::SuperLeft)
        );
        // The time wraps.
        assert_eq!(
            c.on_key(up_at(VK_LWIN, 2950), true),
            release(KeyCode::SuperLeft)
        );
        assert_eq!(
            c.on_key(down_at(VK_LWIN, u32::MAX - 10), true),
            press(KeyCode::SuperLeft)
        );
        assert_eq!(
            c.on_key(down_at(VK_LWIN, 20), true),
            repeat(KeyCode::SuperLeft)
        );
    }

    #[test]
    fn out_of_range_vk_passes() {
        let mut c = active();
        assert_eq!(c.on_key(down(300), true), Verdict::Pass);
        assert_eq!(c.on_key(up(300), true), Verdict::Pass);
    }
}

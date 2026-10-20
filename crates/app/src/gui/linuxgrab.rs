//! Linux full-screen key capture (docs/ui-spec.md section 6.1). Picks the X11 keyboard grab or
//! the Wayland shortcut inhibitor from winit's raw handles. The keys the desktop kept for itself
//! then arrive as normal `KeyboardInput` events.

use crate::gui::wlinhibit::WlInhibit;
use crate::gui::x11grab::X11Grab;
use std::sync::Arc;
use std::time::Instant;
use winit::event_loop::OwnedDisplayHandle;
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::Window;

/// The capture of whichever window system the app runs on.
pub struct OsGrab(Kind);

enum Kind {
    X11(Box<X11Grab>),
    Wayland(Box<WlInhibit>),
}

impl OsGrab {
    /// None when the window system is neither, or the desktop offers no way (logged).
    pub fn new(display: OwnedDisplayHandle, window: Arc<Window>) -> Option<Self> {
        let raw_display = match display.display_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                log::debug!("full-screen key capture: no display handle: {e}");
                return None;
            }
        };
        let raw_window = match window.window_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                log::debug!("full-screen key capture: no window handle: {e}");
                return None;
            }
        };
        match (raw_display, raw_window) {
            (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(w)) => {
                let Some(dpy) = d.display else {
                    log::debug!("full-screen key capture: no X display");
                    return None;
                };
                X11Grab::new(display, window, dpy, w.window).map(|g| Self(Kind::X11(Box::new(g))))
            }
            (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) => {
                WlInhibit::new(display, window, d.display, w.surface)
                    .map(|g| Self(Kind::Wayland(Box::new(g))))
            }
            _ => {
                log::debug!("full-screen key capture: unsupported window system");
                None
            }
        }
    }

    /// After every keyboard-mode recompute: `capture` from `capture_wanted`, `focused` the window
    /// focus.
    pub fn set(&mut self, capture: bool, focused: bool, now: Instant) {
        match &mut self.0 {
            Kind::X11(g) => g.set(capture, focused, now),
            Kind::Wayland(w) => w.set(capture, focused),
        }
    }

    /// From `about_to_wait`: retries a failed X11 grab when due, runs Wayland events. Returns when
    /// to call again.
    pub fn poll(&mut self, now: Instant) -> Option<Instant> {
        match &mut self.0 {
            Kind::X11(g) => g.poll(now),
            Kind::Wayland(w) => {
                w.poll();
                None
            }
        }
    }
}

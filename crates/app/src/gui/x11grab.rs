//! The X11 half of the full-screen key capture (docs/ui-spec.md section 6.1): an
//! `XGrabKeyboard` on winit's window, so the window manager stops handling its own shortcuts and
//! the keys arrive as normal `KeyboardInput` events.
//!
//! Grabbing the window that already has the focus sends no XI2 focus events (the server skips
//! them when the old and the new focus window are the same), and winit reads only XI2 focus
//! events, so winit's focus and key delivery are unaffected. Under XWayland the window also
//! announces `_XWAYLAND_MAY_GRAB_KEYBOARD`, which tells GNOME's mutter that the grab may hold back
//! its own shortcuts.

use crate::gui::grabstate::{x_grab_status, GrabState, GrabStep};
use std::ffi::c_void;
use std::os::raw::c_ulong;
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Instant;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;
use x11_dl::xlib::{self, Xlib};

/// The X11 keyboard grab of one window.
pub struct X11Grab {
    xlib: Xlib,
    display: *mut xlib::Display,
    xid: xlib::Window,
    state: GrabState,
    announced: bool,
    _window: Arc<Window>,
    _owner: OwnedDisplayHandle, // last: keeps winit's X connection open
}

impl X11Grab {
    pub fn new(
        owner: OwnedDisplayHandle,
        window: Arc<Window>,
        display: NonNull<c_void>,
        xid: c_ulong,
    ) -> Option<Self> {
        let xlib = match Xlib::open() {
            Ok(x) => x,
            Err(e) => {
                log::warn!("X11: could not load libX11 for the full-screen keyboard grab: {e}");
                return None;
            }
        };
        Some(Self {
            xlib,
            display: display.as_ptr().cast(),
            xid,
            state: GrabState::new(),
            announced: false,
            _window: window,
            _owner: owner,
        })
    }

    /// Follows `capture` exactly: a focus loss means `capture` is false here, so an unfocused
    /// window never holds the grab.
    pub fn set(&mut self, capture: bool, focused: bool, now: Instant) {
        if focused {
            self.announce();
        }
        let step = self.state.want(capture, now);
        self.run(step, now);
    }

    /// Retries a failed grab when due. Returns when to call again.
    pub fn poll(&mut self, now: Instant) -> Option<Instant> {
        let step = self.state.again(now);
        self.run(step, now);
        self.state.retry_at()
    }

    fn run(&mut self, step: GrabStep, now: Instant) {
        match step {
            GrabStep::Grab => {
                // SAFETY: `display` is winit's Xlib `Display*`, kept open by `_owner`; the call is
                // made on the UI thread (winit called `XInitThreads`); a stale `xid` only raises
                // an X error, which winit's handler logs.
                let code = unsafe {
                    (self.xlib.XGrabKeyboard)(
                        self.display,
                        self.xid,
                        xlib::True,
                        xlib::GrabModeAsync,
                        xlib::GrabModeAsync,
                        xlib::CurrentTime,
                    )
                };
                let before = self.state.failures();
                let ok = code == xlib::GrabSuccess;
                self.state.grabbed(ok, now);
                match (ok, before) {
                    (true, 0) => log::debug!("X11: took the keyboard for full screen"),
                    (true, n) => log::info!("X11: took the keyboard after {n} failed tries"),
                    (false, 0) => log::info!(
                        "X11: could not take the keyboard yet: {}; retrying every 250 ms",
                        x_grab_status(code)
                    ),
                    (false, _) => {}
                }
            }
            GrabStep::Ungrab => {
                self.ungrab();
                log::debug!("X11: gave the keyboard back");
            }
            GrabStep::Wait => {}
        }
    }

    fn ungrab(&mut self) {
        // SAFETY: `display` is winit's Xlib `Display*`, kept open by `_owner`; the calls are made
        // on the UI thread (winit called `XInitThreads`).
        unsafe {
            (self.xlib.XUngrabKeyboard)(self.display, xlib::CurrentTime);
            (self.xlib.XFlush)(self.display);
        }
    }

    /// Tells the window manager, once, that this window may grab the keyboard (XWayland).
    fn announce(&mut self) {
        if self.announced {
            return;
        }
        self.announced = true;
        // SAFETY: `display` is winit's Xlib `Display*`, kept open by `_owner`; the calls are made
        // on the UI thread (winit called `XInitThreads`); a stale `xid` only raises an X error,
        // which winit's handler logs.
        unsafe {
            let atom = (self.xlib.XInternAtom)(
                self.display,
                c"_XWAYLAND_MAY_GRAB_KEYBOARD".as_ptr(),
                xlib::False,
            );
            let mut ev = xlib::XClientMessageEvent {
                type_: xlib::ClientMessage,
                serial: 0,
                send_event: xlib::True,
                display: self.display,
                window: self.xid,
                message_type: atom,
                format: 32,
                data: xlib::ClientMessageData::new(),
            };
            ev.data.set_long(0, 1);
            let mut xev = xlib::XEvent::from(ev);
            (self.xlib.XSendEvent)(
                self.display,
                (self.xlib.XDefaultRootWindow)(self.display),
                xlib::False,
                xlib::SubstructureNotifyMask | xlib::SubstructureRedirectMask,
                &mut xev,
            );
            (self.xlib.XFlush)(self.display);
        }
        log::debug!("X11: told the window manager this window may grab the keyboard (XWayland)");
    }
}

impl Drop for X11Grab {
    fn drop(&mut self) {
        if self.state.held() {
            self.ungrab();
        }
    }
}

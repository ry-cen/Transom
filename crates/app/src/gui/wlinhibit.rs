//! The Wayland half of the full-screen key capture (docs/ui-spec.md section 6.1): a
//! `zwp_keyboard_shortcuts_inhibitor_v1` on the window's surface, so the compositor passes its
//! shortcuts (Super, Alt+Tab) to the window as normal `KeyboardInput` events.
//!
//! winit exposes the `wl_display` and the window's `wl_surface` but no seat, so this opens a guest
//! connection on winit's display with its own event queue. winit's loop reads the shared socket
//! and libwayland files our events into our queue; `poll` (from `about_to_wait`) runs them. Our
//! only events are the inhibitor's `active` and `inactive`, which are only logged.
//!
//! Lifetime: the struct keeps an `OwnedDisplayHandle` (declared last, so it drops last) and the
//! `Arc<Window>`, so the display stays connected and the surface alive while it lives. A protocol
//! error would kill the whole app (the display is shared), so the inhibitor is created in exactly
//! one place, `set`, and only when none exists.
//!
//! A focus loss keeps an existing inhibitor (`keep_inhibitor`): GNOME's prompt can take the
//! keyboard focus, and destroying the inhibitor then would cancel the prompt and loop.

use crate::gui::grabstate::keep_inhibitor;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{
    wl_registry::{self, WlRegistry},
    wl_seat::{self, WlSeat},
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::{self, ZwpKeyboardShortcutsInhibitManagerV1},
    zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1},
};
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

/// The dispatch state of our guest queue; it holds nothing.
struct WlState;

impl Dispatch<WlRegistry, GlobalListContents> for WlState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitManagerV1,
        _: zwp_keyboard_shortcuts_inhibit_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => log::info!(
                "Wayland: the compositor passes its shortcuts (Super, Alt+Tab) to Transom while it has the focus"
            ),
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => log::info!(
                "Wayland: the compositor took its shortcuts back (on GNOME, Super+Esc); they come back the next time full screen starts capture"
            ),
            _ => {}
        }
    }
}

/// The shortcut inhibitor of one window.
pub struct WlInhibit {
    inhibitor: Option<ZwpKeyboardShortcutsInhibitorV1>,
    manager: ZwpKeyboardShortcutsInhibitManagerV1,
    seat: WlSeat,
    surface: WlSurface,
    _globals: GlobalList,
    state: WlState,
    queue: EventQueue<WlState>,
    conn: Connection,
    _window: Arc<Window>,
    _owner: OwnedDisplayHandle, // last: keeps winit's wl_display connected
}

impl WlInhibit {
    pub fn new(
        owner: OwnedDisplayHandle,
        window: Arc<Window>,
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
    ) -> Option<Self> {
        // SAFETY: display is winit's live wl_display; owner (stored last) keeps it connected for
        // as long as this struct and its backend live.
        let conn = Connection::from_backend(unsafe {
            Backend::from_foreign_display(display.as_ptr().cast())
        });
        let (globals, queue) = match registry_queue_init::<WlState>(&conn) {
            Ok(x) => x,
            Err(e) => {
                log::warn!("Wayland: could not list the compositor's globals: {e}");
                return None;
            }
        };
        let qh = queue.handle();
        let manager = match globals.bind::<ZwpKeyboardShortcutsInhibitManagerV1, _, _>(
            &qh,
            1..=1,
            (),
        ) {
            Ok(m) => m,
            Err(_) => {
                log::info!(
                    "Wayland: the compositor cannot pass its shortcuts to apps (no keyboard-shortcuts-inhibit protocol); in full screen, Super, Alt+Tab and the like stay with the desktop"
                );
                return None;
            }
        };
        let seat = match globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()) {
            Ok(s) => s,
            Err(_) => {
                manager.destroy();
                log::warn!("Wayland: no seat; full-screen key capture is off");
                return None;
            }
        };
        // SAFETY: surface is the live wl_surface of winit's window (kept alive by _window); winit
        // manages it with the same wayland-backend, so the id shares its liveness flag.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) };
        let surface = match id.and_then(|id| WlSurface::from_id(&conn, id)) {
            Ok(s) => s,
            Err(_) => {
                manager.destroy();
                log::warn!(
                    "Wayland: could not use the window's surface; full-screen key capture is off"
                );
                return None;
            }
        };
        log::debug!("Wayland: keyboard-shortcuts-inhibit is available");
        Some(Self {
            inhibitor: None,
            manager,
            seat,
            surface,
            _globals: globals,
            state: WlState,
            queue,
            conn,
            _window: window,
            _owner: owner,
        })
    }

    /// The one place that creates an inhibitor, and only when none exists (a second one on the
    /// same surface and seat is a protocol error that would close the whole app).
    pub fn set(&mut self, capture: bool, focused: bool) {
        let want = keep_inhibitor(capture, self.inhibitor.is_some(), focused);
        if want && self.inhibitor.is_none() {
            self.inhibitor = Some(self.manager.inhibit_shortcuts(
                &self.surface,
                &self.seat,
                &self.queue.handle(),
                (),
            ));
            let _ = self.conn.flush();
            log::debug!(
                "Wayland: asked the compositor to pass its shortcuts (GNOME asks the user the first time)"
            );
        } else if !want {
            if let Some(i) = self.inhibitor.take() {
                i.destroy();
                let _ = self.conn.flush();
                log::debug!("Wayland: gave the shortcuts back");
            }
        }
    }

    /// Runs the events already queued for us (no socket read; winit's loop does that).
    pub fn poll(&mut self) {
        if let Err(e) = self.queue.dispatch_pending(&mut self.state) {
            log::debug!("Wayland: inhibitor events: {e}");
        }
    }
}

impl Drop for WlInhibit {
    fn drop(&mut self) {
        if let Some(i) = self.inhibitor.take() {
            i.destroy();
        }
        self.manager.destroy();
        let _ = self.conn.flush();
    }
}

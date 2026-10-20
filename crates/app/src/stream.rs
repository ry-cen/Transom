//! `StreamCore`: everything per connection that is not window chrome, shared by the CLI viewer
//! and the graphical app (docs/ui-spec.md section 8.4). WP-B2.
//!
//! It owns the supervisor handle, the event channel, the decoder, the newest decoded frame, the
//! key and pointer mappers and the Mac cursor. Its behaviour is the CLI viewer's, moved here
//! unchanged (viewer.rs of wave A); the viewer and the GUI shell are thin callers.
#![cfg(feature = "video")]

use crate::cli::ConnectOpts;
use crossbeam_channel::Receiver;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use transom_input::{KeyMapper, KeymapMode, PointerMapper};
use transom_net::identity::DeviceIdentity;
use transom_net::session::{Outputs, SessionCommand, SessionEvent, SessionOptions, SourceChoice};
use transom_net::store::Store;
use transom_net::supervisor::{spawn_supervisor, Route, SessionHandle, SupervisorConfig};
use transom_proto::blobs::CursorShape;
use transom_proto::compat::client_hello;
use transom_proto::geometry::letterbox;
use transom_proto::messages::{InputEvent, StreamSource, Viewport};
use transom_render::cursor::cursor_image;
use transom_render::{Chroma, Renderer, YuvFrame};
use transom_video::{spawn_decoder, DecodedFrame, Planes};
use winit::event::{MouseButton, MouseScrollDelta, TouchPhase};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, ModifiersState};
use winit::window::{CursorIcon, CustomCursor, Window};

/// The delay between a size change and the Viewport message it causes.
const VIEWPORT_DELAY: Duration = Duration::from_millis(300);

/// A wake-up the stream calls from other threads (the event loop proxy's `send_event`).
pub type Wake = Arc<dyn Fn() + Send + Sync>;

type Slot = Arc<Mutex<Option<DecodedFrame>>>;

/// What a stream needs to know about the user's choices.
#[derive(Clone, Debug)]
pub struct StreamOpts {
    pub source: SourceChoice,
    pub keymap: KeymapMode,
    pub fps: i64,
    pub allow_local_pointer: bool,
    pub allow_local_keyboard: bool,
    pub hello_version: String,
    pub name: String,
}

impl From<&ConnectOpts> for StreamOpts {
    fn from(o: &ConnectOpts) -> Self {
        StreamOpts {
            source: o.source,
            keymap: o.keymap_mode(),
            fps: o.fps,
            allow_local_pointer: o.allow_local_pointer,
            allow_local_keyboard: o.allow_local_keyboard,
            hello_version: o.hello_version.clone(),
            name: o.name.clone(),
        }
    }
}

/// A keyboard chord: Ctrl+Alt+Shift plus one physical key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    B,
    D,
    W,
    A,
    S,
    F,
    Q,
}

impl Chord {
    fn of_key(code: KeyCode) -> Option<Chord> {
        Some(match code {
            KeyCode::KeyB => Chord::B,
            KeyCode::KeyD => Chord::D,
            KeyCode::KeyW => Chord::W,
            KeyCode::KeyA => Chord::A,
            KeyCode::KeyS => Chord::S,
            KeyCode::KeyF => Chord::F,
            KeyCode::KeyQ => Chord::Q,
            _ => return None,
        })
    }
}

/// Which chords an app handles, and whether Windows' AltGr rule applies (spec 7.2).
#[derive(Clone, Copy, Debug)]
pub struct ChordRules {
    pub handled: &'static [Chord],
    pub windows_left_ctrl: bool,
}

impl ChordRules {
    /// The CLI viewer: D, W, F and Q, with no AltGr rule.
    pub const CLI: ChordRules = ChordRules {
        handled: &[Chord::D, Chord::W, Chord::F, Chord::Q],
        windows_left_ctrl: false,
    };
    /// The graphical app.
    pub const GUI: ChordRules = ChordRules {
        handled: &[Chord::B, Chord::D, Chord::W, Chord::S, Chord::F, Chord::Q],
        windows_left_ctrl: true,
    };
}

/// Ctrl+Alt+Shift plus one physical key in `rules.handled`; with `windows_left_ctrl` and
/// `on_windows`, the left Ctrl must be down too (AltGr is reported as Ctrl+Alt; spec 7.2).
pub fn chord_of(
    mods: ModifiersState,
    left_ctrl_down: bool,
    code: KeyCode,
    rules: &ChordRules,
    on_windows: bool,
) -> Option<Chord> {
    if !(mods.control_key() && mods.alt_key() && mods.shift_key()) {
        return None;
    }
    if rules.windows_left_ctrl && on_windows && !left_ctrl_down {
        return None;
    }
    Chord::of_key(code).filter(|c| rules.handled.contains(c))
}

/// What `StreamCore::key` did with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// A chord press: the caller acts on it (nothing was forwarded).
    Chord(Chord),
    /// A chord's repeat or release: eaten.
    Swallowed,
    /// Sent to the Mac.
    Forwarded,
    /// Not a chord and not sent.
    NotForwarded,
}

/// The chord bookkeeping of the keyboard: modifiers, the left Ctrl, and the chord keys whose
/// release must be eaten.
struct KeyRouter {
    mods: ModifiersState,
    left_ctrl_down: bool,
    swallowed: HashSet<KeyCode>,
    rules: ChordRules,
}

impl KeyRouter {
    fn new(rules: ChordRules) -> Self {
        KeyRouter {
            mods: ModifiersState::empty(),
            left_ctrl_down: false,
            swallowed: HashSet::new(),
            rules,
        }
    }

    /// Tracks the left Ctrl; true when this press is a chord (the key is then swallowed until its
    /// release). None = not a chord.
    fn route(
        &mut self,
        code: KeyCode,
        pressed: bool,
        _repeat: bool,
        on_windows: bool,
    ) -> Option<Chord> {
        if code == KeyCode::ControlLeft {
            self.left_ctrl_down = pressed;
        }
        if !pressed {
            return None;
        }
        let c = chord_of(
            self.mods,
            self.left_ctrl_down,
            code,
            &self.rules,
            on_windows,
        )?;
        self.swallowed.insert(code);
        Some(c)
    }

    /// True when this release belongs to a swallowed chord key.
    fn swallowed_release(&mut self, code: KeyCode) -> bool {
        self.swallowed.remove(&code)
    }

    /// The part of `StreamCore::key` before forwarding.
    fn pre(
        &mut self,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
        on_windows: bool,
    ) -> Option<KeyOutcome> {
        if let Some(c) = self.route(code, pressed, repeat, on_windows) {
            return Some(if repeat {
                KeyOutcome::Swallowed
            } else {
                KeyOutcome::Chord(c)
            });
        }
        if !pressed && self.swallowed_release(code) {
            return Some(KeyOutcome::Swallowed);
        }
        None
    }
}

/// One connection's non-chrome state.
pub struct StreamCore {
    handle: SessionHandle,
    ev_rx: Receiver<SessionEvent>,
    slot: Slot,
    _decoder: std::thread::JoinHandle<()>,
    keys: KeyMapper,
    pointer: PointerMapper,
    router: KeyRouter,
    opts: StreamOpts,
    forward_pointer: bool,
    forward_keys: bool,
    connected: bool,
    local_peer: bool,
    mac_cursor: Option<CustomCursor>,
    last_shape: Option<CursorShape>,
    cursor_dirty: bool,
    showing_mac_cursor: bool,
    show_mac: bool,
    viewport_due: Option<Instant>,
}

impl StreamCore {
    /// Spawns the decoder and the supervisor. `wake_net` is called after every session event,
    /// `wake_frame` after every decoded frame.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        route: Route,
        opts: StreamOpts,
        identity: DeviceIdentity,
        store: Option<Store>,
        rt: &tokio::runtime::Handle,
        chords: ChordRules,
        wake_net: Wake,
        wake_frame: Wake,
    ) -> StreamCore {
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let (video_tx, video_rx) = crossbeam_channel::bounded(16);
        let slot: Slot = Arc::new(Mutex::new(None));
        let slot_for_decoder = slot.clone();
        let decoder = spawn_decoder(video_rx, move |f| {
            *slot_for_decoder.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
            wake_frame();
        });
        let handle = spawn_supervisor(
            rt,
            SupervisorConfig {
                route,
                options: SessionOptions {
                    hello: client_hello(&opts.hello_version, &opts.name),
                    viewport: None,
                    source: opts.source,
                    stats_device: opts.name.clone(),
                },
                identity,
                store,
            },
            Outputs {
                events: ev_tx,
                video: video_tx,
                wake: wake_net,
            },
        );
        StreamCore {
            handle,
            ev_rx,
            slot,
            _decoder: decoder,
            keys: KeyMapper::new(opts.keymap),
            pointer: PointerMapper::new(),
            router: KeyRouter::new(chords),
            opts,
            forward_pointer: false,
            forward_keys: false,
            connected: false,
            local_peer: false,
            mac_cursor: None,
            last_shape: None,
            cursor_dirty: false,
            showing_mac_cursor: false,
            show_mac: true,
            viewport_due: None,
        }
    }

    /// Drains the session's events, updating the forwarding flags, and returns them unchanged
    /// and in order.
    pub fn take_events(&mut self) -> Vec<SessionEvent> {
        let events: Vec<SessionEvent> = self.ev_rx.try_iter().collect();
        for ev in &events {
            match ev {
                SessionEvent::Connected { local_peer, .. } => {
                    self.connected = true;
                    self.local_peer = *local_peer;
                    self.recompute_forwarding();
                    self.viewport_due = Some(Instant::now());
                }
                SessionEvent::CursorShape(s) => {
                    self.last_shape = Some(s.clone());
                    self.cursor_dirty = true;
                }
                SessionEvent::Ended { .. } | SessionEvent::Finished { .. } => {
                    // Release while the flags are still on, so the button-ups are sent.
                    self.release_all();
                    self.forward_keys = false;
                    self.forward_pointer = false;
                    self.connected = false;
                }
                _ => {}
            }
        }
        events
    }

    fn recompute_forwarding(&mut self) {
        self.forward_keys = !self.local_peer || self.opts.allow_local_keyboard;
        self.forward_pointer = !self.local_peer || self.opts.allow_local_pointer;
    }

    /// The window-side effects of `events` (as returned by `take_events`): rebuild the Mac
    /// cursor, blank the picture when the session ended, and ask for a redraw.
    pub fn apply_window_effects(
        &mut self,
        el: &ActiveEventLoop,
        window: &Window,
        renderer: Option<&mut Renderer>,
        events: &[SessionEvent],
    ) {
        if self.cursor_dirty {
            self.cursor_dirty = false;
            self.rebuild_cursor(el, window);
        }
        if events.iter().any(|e| {
            matches!(
                e,
                SessionEvent::Ended { .. } | SessionEvent::Finished { .. }
            )
        }) {
            if let Some(r) = renderer {
                r.clear();
            }
        }
        window.request_redraw();
    }

    fn rebuild_cursor(&mut self, el: &ActiveEventLoop, window: &Window) {
        let Some(shape) = &self.last_shape else {
            return;
        };
        let image = cursor_image(
            &shape.png,
            (shape.hot_x, shape.hot_y),
            (shape.width, shape.height),
            window.scale_factor(),
        );
        self.mac_cursor = image
            .and_then(|i| i.to_source())
            .map(|s| el.create_custom_cursor(s));
        self.set_window_cursor(window);
    }

    /// Sets the window's cursor from the current wish, whether or not it changed.
    fn set_window_cursor(&mut self, window: &Window) {
        let want = self.show_mac && self.pointer.cursor_inside() && self.mac_cursor.is_some();
        match (&self.mac_cursor, want) {
            (Some(c), true) => window.set_cursor(c.clone()),
            _ => window.set_cursor(CursorIcon::Default),
        }
        self.showing_mac_cursor = want;
    }

    fn update_layout(&mut self, renderer: &Renderer, window: &Window) {
        let rect = renderer
            .video_size()
            .and_then(|v| letterbox(renderer.surface_size(), v));
        self.pointer.set_layout(rect, window.scale_factor());
    }

    /// Uploads the newest decoded frame, if any. True when one was uploaded (a redraw is then
    /// requested).
    pub fn take_frame(&mut self, renderer: &mut Renderer, window: &Window) -> bool {
        let frame = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(f) = frame else {
            return false;
        };
        let before = renderer.video_size();
        let (y, chroma) = match f.planes() {
            Planes::I420 { y, u, v } => (
                y,
                Chroma::I420 {
                    u: u.data,
                    u_stride: u.stride,
                    v: v.data,
                    v_stride: v.stride,
                },
            ),
            Planes::Nv12 { y, uv } => (
                y,
                Chroma::Nv12 {
                    uv: uv.data,
                    uv_stride: uv.stride,
                },
            ),
        };
        renderer.upload(&YuvFrame {
            width: f.width,
            height: f.height,
            y: y.data,
            y_stride: y.stride,
            chroma,
            full_range: f.full_range,
            bt601: f.bt601,
        });
        if renderer.video_size() != before {
            self.update_layout(renderer, window);
        }
        window.request_redraw();
        true
    }

    /// The window's modifier state (from `ModifiersChanged`).
    pub fn set_mods(&mut self, mods: ModifiersState) {
        self.router.mods = mods;
    }

    /// Routes one physical key: a chord press is reported and swallowed (with its repeats and
    /// release); anything else goes to the Mac when `forward` and the key flag allow.
    pub fn key(&mut self, code: KeyCode, pressed: bool, repeat: bool, forward: bool) -> KeyOutcome {
        if let Some(o) = self.router.pre(code, pressed, repeat, cfg!(windows)) {
            return o;
        }
        if forward && self.forward_keys {
            let events = self.keys.key(code, pressed, repeat);
            self.send_all(events);
            KeyOutcome::Forwarded
        } else {
            KeyOutcome::NotForwarded
        }
    }

    pub fn pointer_moved(&mut self, x: f64, y: f64, forward: bool) {
        if let Some(ev) = self.pointer.moved(x, y) {
            if self.forward_pointer && (forward || self.pointer.buttons_held()) {
                self.send_input(ev);
            }
        }
    }

    pub fn pointer_button(&mut self, button: MouseButton, pressed: bool, forward: bool) {
        if pressed && !forward {
            return;
        }
        if let Some(ev) = self.pointer.button(button, pressed) {
            if self.forward_pointer {
                self.send_input(ev);
            }
        }
    }

    pub fn pointer_wheel(&mut self, delta: MouseScrollDelta, phase: TouchPhase, forward: bool) {
        let events = self.pointer.scroll(delta, phase);
        if forward && self.forward_pointer {
            self.send_all(events);
        }
    }

    pub fn pointer_left(&mut self) {
        self.pointer.left_window();
    }

    /// Some Mac button is held down by the local pointer.
    pub fn mac_buttons_held(&self) -> bool {
        self.pointer.buttons_held()
    }

    fn send_input(&self, ev: InputEvent) {
        self.send(SessionCommand::Input(ev));
    }

    fn send_all(&self, events: Vec<InputEvent>) {
        for ev in events {
            self.send_input(ev);
        }
    }

    /// Lets go of every held key and button on the Mac.
    pub fn release_all(&mut self) {
        let keys = self.keys.release_all();
        let pointer = self.pointer.release_all();
        if self.forward_keys {
            self.send_all(keys);
        }
        if self.forward_pointer {
            self.send_all(pointer);
        }
        self.router.swallowed.clear();
        // Focus loss and mode changes also drop the left-Ctrl memory (its release may be missed).
        self.router.left_ctrl_down = false;
    }

    pub fn send(&self, cmd: SessionCommand) {
        self.handle.send(cmd);
    }

    pub fn select(&self, src: StreamSource) {
        self.send(SessionCommand::Select(src));
    }

    /// The window was resized: resize the surface, redo the layout, and send a Viewport soon.
    pub fn on_resized(&mut self, window: &Window, renderer: &mut Renderer) {
        let s = window.inner_size();
        renderer.resize(s.width, s.height);
        self.update_layout(renderer, window);
        self.viewport_due = Some(Instant::now() + VIEWPORT_DELAY);
        window.request_redraw();
    }

    pub fn on_scale_factor(&mut self, el: &ActiveEventLoop, window: &Window, renderer: &Renderer) {
        self.update_layout(renderer, window);
        self.rebuild_cursor(el, window);
    }

    /// Send a Viewport message at `at`.
    pub fn request_viewport(&mut self, at: Instant) {
        self.viewport_due = Some(at);
    }

    /// When a Viewport is due, sends it and returns None; otherwise the time it is due.
    pub fn viewport_tick(&mut self, window: &Window, now: Instant) -> Option<Instant> {
        let due = self.viewport_due?;
        if now < due {
            return Some(due);
        }
        let size = window.inner_size();
        let sf = window.scale_factor();
        self.viewport_due = None;
        self.send(SessionCommand::Viewport(Viewport {
            width: f64::from(size.width) / sf,
            height: f64::from(size.height) / sf,
            scale: None,
            local_cursor: None,
            fps: Some(self.opts.fps),
        }));
        None
    }

    /// When the next Viewport is due, if one is pending.
    pub fn viewport_due(&self) -> Option<Instant> {
        self.viewport_due
    }

    /// The pointer is over the video area.
    pub fn cursor_inside(&self) -> bool {
        self.pointer.cursor_inside()
    }

    /// Shows the Mac's cursor (when it has one and the pointer is inside the video) or the
    /// default arrow, changing the window's cursor only when the wish changed.
    pub fn apply_cursor(&mut self, window: &Window, show_mac: bool) {
        self.show_mac = show_mac;
        let want = show_mac && self.pointer.cursor_inside() && self.mac_cursor.is_some();
        if want == self.showing_mac_cursor {
            return;
        }
        match (&self.mac_cursor, want) {
            (Some(c), true) => window.set_cursor(c.clone()),
            _ => window.set_cursor(CursorIcon::Default),
        }
        self.showing_mac_cursor = want;
    }

    pub fn forward_keys(&self) -> bool {
        self.forward_keys
    }

    pub fn forward_pointer(&self) -> bool {
        self.forward_pointer
    }

    /// The Mac is this computer.
    pub fn local_peer(&self) -> bool {
        self.local_peer
    }

    /// Connected, and neither pointer nor keys are forwarded.
    pub fn view_only(&self) -> bool {
        self.connected && !self.forward_pointer && !self.forward_keys
    }

    /// Changes the keymap and the local-input switches. Everything held is released first.
    pub fn set_input_options(
        &mut self,
        keymap: KeymapMode,
        allow_local_pointer: bool,
        allow_local_keyboard: bool,
    ) {
        self.release_all();
        self.keys = KeyMapper::new(keymap);
        self.opts.keymap = keymap;
        self.opts.allow_local_pointer = allow_local_pointer;
        self.opts.allow_local_keyboard = allow_local_keyboard;
        if self.connected {
            self.recompute_forwarding();
        }
    }

    /// Releases everything, closes the session and, when `wait` is not zero and this is not a
    /// runtime thread, waits up to `wait` for the supervisor to flush the queued input.
    pub fn close(mut self, rt: &tokio::runtime::Handle, wait: Duration) {
        self.release_all();
        self.handle.close();
        if !wait.is_zero() && tokio::runtime::Handle::try_current().is_err() {
            if let Some(join) = self.handle.take_join() {
                let _ = rt.block_on(tokio::time::timeout(wait, join));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cas() -> ModifiersState {
        ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT
    }

    #[test]
    fn chord_detection() {
        let cli = ChordRules::CLI;
        let gui = ChordRules::GUI;
        assert_eq!(
            chord_of(cas(), false, KeyCode::KeyD, &cli, false),
            Some(Chord::D)
        );
        assert_eq!(chord_of(cas(), false, KeyCode::KeyB, &cli, false), None);
        let no_shift = ModifiersState::CONTROL | ModifiersState::ALT;
        assert_eq!(chord_of(no_shift, false, KeyCode::KeyD, &cli, false), None);
        assert_eq!(
            chord_of(cas(), false, KeyCode::KeyB, &gui, false),
            Some(Chord::B)
        );
        assert_eq!(chord_of(cas(), false, KeyCode::KeyB, &gui, true), None);
        assert_eq!(
            chord_of(cas(), true, KeyCode::KeyB, &gui, true),
            Some(Chord::B)
        );
        assert_eq!(
            chord_of(cas(), false, KeyCode::KeyD, &cli, true),
            Some(Chord::D)
        );
    }

    #[test]
    fn key_outcomes() {
        let mut r = KeyRouter::new(ChordRules::CLI);
        r.mods = cas();
        assert_eq!(
            r.pre(KeyCode::KeyD, true, false, false),
            Some(KeyOutcome::Chord(Chord::D))
        );
        assert_eq!(
            r.pre(KeyCode::KeyD, true, true, false),
            Some(KeyOutcome::Swallowed)
        );
        assert_eq!(
            r.pre(KeyCode::KeyD, false, false, false),
            Some(KeyOutcome::Swallowed)
        );
        // The release was consumed: a second one passes through.
        assert_eq!(r.pre(KeyCode::KeyD, false, false, false), None);
        // An ordinary key passes through, with or without the chord modifiers.
        assert_eq!(r.pre(KeyCode::KeyX, true, false, false), None);
        r.mods = ModifiersState::empty();
        assert_eq!(r.pre(KeyCode::KeyD, true, false, false), None);
    }

    #[test]
    fn left_ctrl_is_tracked_even_for_repeats() {
        let mut r = KeyRouter::new(ChordRules::GUI);
        r.mods = cas();
        assert_eq!(r.pre(KeyCode::KeyB, true, false, true), None);
        r.pre(KeyCode::ControlLeft, true, true, true);
        assert_eq!(
            r.pre(KeyCode::KeyB, true, false, true),
            Some(KeyOutcome::Chord(Chord::B))
        );
        r.pre(KeyCode::ControlLeft, false, false, true);
        assert!(!r.left_ctrl_down);
    }
}

//! The viewer window behind `transom connect`.
#![cfg(feature = "video")]

use crate::cli::ConnectOpts;
use anyhow::{anyhow, Result};
use crossbeam_channel::Receiver;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use transom_input::{KeyMapper, PointerMapper};
use transom_net::session::{Outputs, SessionCommand, SessionEvent, SessionOptions};
use transom_net::store::Store;
use transom_net::supervisor::{spawn_supervisor, Route, SessionHandle, SupervisorConfig};
use transom_proto::blobs::CursorShape;
use transom_proto::compat::client_hello;
use transom_proto::geometry::letterbox;
use transom_proto::messages::{InputEvent, StreamSource, Viewport, WindowInfo};
use transom_render::cursor::cursor_image;
use transom_render::{wgpu, Renderer, YuvFrame};
use transom_video::{spawn_decoder, DecodedFrame};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, CustomCursor, Fullscreen, Window, WindowId};

const VIEWPORT_DELAY: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy)]
enum UserEvent {
    Net,
    Frame,
}

type Slot = Arc<Mutex<Option<DecodedFrame>>>;

struct Viewer {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    instance: wgpu::Instance,
    ev_rx: Receiver<SessionEvent>,
    slot: Slot,
    handle: SessionHandle,
    keys: KeyMapper,
    pointer: PointerMapper,
    mods: ModifiersState,
    swallowed: HashSet<KeyCode>,
    allow_pointer: bool,
    allow_keyboard: bool,
    forward_pointer: bool,
    forward_keys: bool,
    connected: bool,
    local_notice_shown: bool,
    mac_name: String,
    windows: Vec<WindowInfo>,
    active: StreamSource,
    status: String,
    rtt: Option<f64>,
    fps: u32,
    mac_cursor: Option<CustomCursor>,
    last_shape: Option<CursorShape>,
    showing_mac_cursor: bool,
    viewport_due: Option<Instant>,
    fps_wanted: i64,
    fullscreen: bool,
    exit_code: i32,
}

pub fn run(route: Route, opts: ConnectOpts, store: Store, rt: &Runtime) -> Result<i32> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
    let (video_tx, video_rx) = crossbeam_channel::bounded(16);

    let slot: Slot = Arc::new(Mutex::new(None));
    let slot_for_decoder = slot.clone();
    let frame_proxy = proxy.clone();
    let _decoder = spawn_decoder(video_rx, move |f| {
        *slot_for_decoder.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
        let _ = frame_proxy.send_event(UserEvent::Frame);
    });

    let identity = store.identity().map_err(|e| anyhow!("{e}"))?;
    let net_proxy = proxy.clone();
    let handle = spawn_supervisor(
        rt.handle(),
        SupervisorConfig {
            route,
            options: SessionOptions {
                hello: client_hello(&opts.hello_version, &opts.name),
                viewport: None,
                source: opts.source,
                stats_device: opts.name.clone(),
            },
            identity,
            store: Some(store),
        },
        Outputs {
            events: ev_tx,
            video: video_tx,
            wake: Arc::new(move || {
                let _ = net_proxy.send_event(UserEvent::Net);
            }),
        },
    );

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
        Box::new(event_loop.owned_display_handle()),
    ));
    let mut viewer = Viewer {
        window: None,
        renderer: None,
        instance,
        ev_rx,
        slot,
        handle: handle.clone(),
        keys: KeyMapper::new(opts.keymap_mode()),
        pointer: PointerMapper::new(),
        mods: ModifiersState::empty(),
        swallowed: HashSet::new(),
        allow_pointer: opts.allow_local_pointer,
        allow_keyboard: opts.allow_local_keyboard,
        forward_pointer: false,
        forward_keys: false,
        connected: false,
        local_notice_shown: false,
        mac_name: String::new(),
        windows: Vec::new(),
        active: StreamSource::None {},
        status: "connecting…".to_string(),
        rtt: None,
        fps: 0,
        mac_cursor: None,
        last_shape: None,
        showing_mac_cursor: false,
        viewport_due: None,
        fps_wanted: opts.fps,
        fullscreen: false,
        exit_code: 0,
    };
    event_loop.run_app(&mut viewer)?;
    handle.close();
    if let Some(join) = handle.take_join() {
        // Give the supervisor a moment to flush the queued button-ups before the runtime goes.
        let _ = rt.block_on(tokio::time::timeout(Duration::from_millis(300), join));
    }
    Ok(viewer.exit_code)
}

impl Viewer {
    fn send(&self, cmd: SessionCommand) {
        self.handle.send(cmd);
    }

    fn send_input(&self, ev: InputEvent) {
        self.send(SessionCommand::Input(ev));
    }

    fn send_all(&self, events: Vec<InputEvent>) {
        for ev in events {
            self.send_input(ev);
        }
    }

    /// Let go of every held key and button on the Mac.
    fn release_all(&mut self) {
        let keys = self.keys.release_all();
        let pointer = self.pointer.release_all();
        if self.forward_keys {
            self.send_all(keys);
        }
        if self.forward_pointer {
            self.send_all(pointer);
        }
        self.swallowed.clear();
    }

    fn update_layout(&mut self) {
        let (Some(r), Some(w)) = (&self.renderer, &self.window) else {
            return;
        };
        let rect = r.video_size().and_then(|v| letterbox(r.surface_size(), v));
        self.pointer.set_layout(rect, w.scale_factor());
    }

    fn refresh_cursor(&mut self, force: bool) {
        let Some(window) = &self.window else {
            return;
        };
        let want = self.pointer.cursor_inside() && self.mac_cursor.is_some();
        if !force && want == self.showing_mac_cursor {
            return;
        }
        match (&self.mac_cursor, want) {
            (Some(c), true) => window.set_cursor(c.clone()),
            _ => window.set_cursor(CursorIcon::Default),
        }
        self.showing_mac_cursor = want;
    }

    fn rebuild_cursor(&mut self, el: &ActiveEventLoop) {
        let (Some(shape), Some(window)) = (&self.last_shape, &self.window) else {
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
        self.refresh_cursor(true);
    }

    fn source_text(&self) -> String {
        match &self.active {
            StreamSource::Desktop {} => "Desktop".to_string(),
            StreamSource::Window { id } => self.windows.iter().find(|w| w.id == *id).map_or_else(
                || "Window".to_string(),
                |w| format!("{}: {}", w.app_name, w.title),
            ),
            StreamSource::None {} => "nothing".to_string(),
        }
    }

    fn update_title(&self) {
        let Some(window) = &self.window else {
            return;
        };
        let title = if self.connected {
            let rtt = self
                .rtt
                .map_or_else(|| "–".to_string(), |r| format!("{r:.0}"));
            let view_only = if !self.forward_pointer && !self.forward_keys {
                " (view only)"
            } else {
                ""
            };
            format!(
                "{} — {} — {} ms · {} fps — Transom{}",
                self.mac_name,
                self.source_text(),
                rtt,
                self.fps,
                view_only
            )
        } else {
            format!("{} — Transom", self.status)
        };
        window.set_title(&title);
    }

    fn handle_net(&mut self, el: &ActiveEventLoop) {
        let events: Vec<SessionEvent> = self.ev_rx.try_iter().collect();
        for ev in events {
            match ev {
                SessionEvent::Status(s) => self.status = s,
                SessionEvent::Connected {
                    mac_name,
                    local_peer,
                    ..
                } => {
                    self.mac_name = mac_name;
                    self.connected = true;
                    self.forward_keys = !local_peer || self.allow_keyboard;
                    self.forward_pointer = !local_peer || self.allow_pointer;
                    if local_peer
                        && (!self.forward_keys || !self.forward_pointer)
                        && !self.local_notice_shown
                    {
                        self.local_notice_shown = true;
                        let mut off = Vec::new();
                        if !self.forward_keys {
                            off.push("keyboard input is off (it would loop; --allow-local-keyboard turns it on)");
                        }
                        if !self.forward_pointer {
                            off.push("pointer input is off (--allow-local-pointer turns it on)");
                        }
                        eprintln!("This Mac is the one you are viewing: {}.", off.join("; "));
                    }
                    self.viewport_due = Some(Instant::now());
                }
                SessionEvent::WindowList(l) => {
                    self.windows = l.windows;
                    self.active = l.active;
                }
                SessionEvent::CursorShape(s) => {
                    self.last_shape = Some(s);
                    self.rebuild_cursor(el);
                }
                SessionEvent::Stats { rtt_ms, fps } => {
                    self.rtt = rtt_ms;
                    self.fps = fps;
                }
                SessionEvent::Ended { text, retry_in, .. } => {
                    self.connected = false;
                    self.status = match retry_in {
                        Some(d) => {
                            let secs = d.as_secs_f64().ceil() as u64;
                            format!("{text} Retrying in {secs} s.")
                        }
                        None => text,
                    };
                    if let Some(r) = &mut self.renderer {
                        r.clear();
                    }
                    self.release_all();
                    self.forward_keys = false;
                    self.forward_pointer = false;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                SessionEvent::Finished { text, .. } => {
                    eprintln!("{}", transom_proto::safetext::label(&text, 300));
                    self.exit_code = 1;
                    el.exit();
                }
                SessionEvent::MacInfo { .. }
                | SessionEvent::AppList(_)
                | SessionEvent::Thumbnail { .. }
                | SessionEvent::AppIcon { .. }
                | SessionEvent::HostSettings(_) => {}
            }
        }
        self.update_title();
    }

    fn handle_frame(&mut self) {
        let frame = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        let (Some(f), Some(r)) = (frame, self.renderer.as_mut()) else {
            return;
        };
        let before = r.video_size();
        r.upload(&YuvFrame {
            width: f.width,
            height: f.height,
            y: &f.y,
            y_stride: f.y_stride,
            u: &f.u,
            u_stride: f.u_stride,
            v: &f.v,
            v_stride: f.v_stride,
            full_range: f.full_range,
            bt601: f.bt601,
        });
        let changed = r.video_size() != before;
        if changed {
            self.update_layout();
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn close(&mut self, el: &ActiveEventLoop) {
        self.release_all();
        self.handle.close();
        self.exit_code = 0;
        el.exit();
    }

    fn shortcut(&mut self, code: KeyCode, el: &ActiveEventLoop) {
        match code {
            KeyCode::KeyD => self.send(SessionCommand::Select(StreamSource::Desktop {})),
            KeyCode::KeyW => {
                if self.windows.is_empty() {
                    self.status = "No windows on the Mac".to_string();
                    eprintln!("transom: {}", self.status);
                    self.update_title();
                    return;
                }
                let next = match &self.active {
                    StreamSource::Window { id } => self
                        .windows
                        .iter()
                        .position(|w| w.id == *id)
                        .map_or(0, |i| (i + 1) % self.windows.len()),
                    _ => 0,
                };
                let id = self.windows[next].id;
                self.send(SessionCommand::Select(StreamSource::Window { id }));
            }
            KeyCode::KeyF => {
                self.fullscreen = !self.fullscreen;
                if let Some(w) = &self.window {
                    w.set_fullscreen(self.fullscreen.then_some(Fullscreen::Borderless(None)));
                }
            }
            KeyCode::KeyQ => self.close(el),
            _ => {}
        }
    }

    fn keyboard(&mut self, el: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool) {
        let chord = self.mods.control_key() && self.mods.alt_key() && self.mods.shift_key();
        let is_shortcut = matches!(
            code,
            KeyCode::KeyD | KeyCode::KeyW | KeyCode::KeyF | KeyCode::KeyQ
        );
        if pressed && chord && is_shortcut {
            self.swallowed.insert(code);
            if !repeat {
                self.shortcut(code, el);
            }
            return;
        }
        if !pressed && self.swallowed.remove(&code) {
            return;
        }
        if self.forward_keys {
            let events = self.keys.key(code, pressed, repeat);
            self.send_all(events);
        }
    }
}

impl ApplicationHandler<UserEvent> for Viewer {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Transom — connecting…")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("transom: could not create the window: {e}");
                self.exit_code = 1;
                el.exit();
                return;
            }
        };
        match Renderer::new(&self.instance, window.clone()) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                eprintln!("transom: could not start the renderer: {e:#}");
                self.exit_code = 1;
                el.exit();
                return;
            }
        }
        self.window = Some(window);
        self.viewport_due = Some(Instant::now() + VIEWPORT_DELAY);
        self.update_layout();
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Net => self.handle_net(el),
            UserEvent::Frame => self.handle_frame(),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(s) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(s.width, s.height);
                }
                self.update_layout();
                self.viewport_due = Some(Instant::now() + VIEWPORT_DELAY);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.update_layout();
                self.rebuild_cursor(el);
            }
            WindowEvent::RedrawRequested => {
                let ok = self.renderer.as_mut().is_none_or(|r| r.render());
                if !ok {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.keyboard(el, code, event.state == ElementState::Pressed, event.repeat);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(ev) = self.pointer.moved(position.x, position.y) {
                    if self.forward_pointer {
                        self.send_input(ev);
                    }
                }
                self.refresh_cursor(false);
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer.left_window();
                self.refresh_cursor(false);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(ev) = self.pointer.button(button, state == ElementState::Pressed) {
                    if self.forward_pointer {
                        self.send_input(ev);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let events = self.pointer.scroll(delta, phase);
                if self.forward_pointer {
                    self.send_all(events);
                }
            }
            WindowEvent::Focused(false) => self.release_all(),
            WindowEvent::CloseRequested => self.close(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let (Some(due), Some(window)) = (self.viewport_due, &self.window) {
            if Instant::now() >= due {
                let size = window.inner_size();
                let sf = window.scale_factor();
                self.viewport_due = None;
                self.send(SessionCommand::Viewport(Viewport {
                    width: f64::from(size.width) / sf,
                    height: f64::from(size.height) / sf,
                    scale: None,
                    local_cursor: None,
                    fps: Some(self.fps_wanted),
                }));
            }
        }
        el.set_control_flow(match self.viewport_due {
            Some(due) => ControlFlow::WaitUntil(due),
            None => ControlFlow::Wait,
        });
    }
}

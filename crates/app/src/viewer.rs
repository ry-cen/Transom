//! The viewer window behind `transom connect`. The per-connection work (supervisor, decoder,
//! key and pointer mapping, the Mac cursor) lives in `StreamCore` (stream.rs), shared with the
//! graphical app; this file is the window around it.
#![cfg(feature = "video")]

use crate::cli::ConnectOpts;
use crate::session_state::SessionState;
use crate::stream::{Chord, ChordRules, KeyOutcome, StreamCore, StreamOpts};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use transom_net::session::SessionEvent;
use transom_net::store::Store;
use transom_net::supervisor::Route;
use transom_proto::messages::{StreamSource, WindowInfo};
use transom_render::{wgpu, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Fullscreen, Window, WindowId};

const VIEWPORT_DELAY: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy)]
enum UserEvent {
    Net,
    Frame,
}

struct Viewer {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    instance: wgpu::Instance,
    /// Some until `run` returns.
    core: Option<StreamCore>,
    state: SessionState,
    /// Fed only by `WindowList` events and not cleared on `Ended`, so chord+W behaves the same
    /// during a reconnect.
    windows: Vec<WindowInfo>,
    active: StreamSource,
    allow_pointer: bool,
    allow_keyboard: bool,
    local_notice_shown: bool,
    fullscreen: bool,
    exit_code: i32,
}

pub fn run(route: Route, opts: ConnectOpts, store: Store, rt: &Runtime) -> Result<i32> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let identity = store.identity().map_err(|e| anyhow!("{e}"))?;
    let frame_proxy = proxy.clone();
    let net_proxy = proxy.clone();
    let core = StreamCore::start(
        route,
        StreamOpts::from(&opts),
        identity,
        Some(store),
        rt.handle(),
        ChordRules::CLI,
        Arc::new(move || {
            let _ = net_proxy.send_event(UserEvent::Net);
        }),
        Arc::new(move || {
            let _ = frame_proxy.send_event(UserEvent::Frame);
        }),
    );

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
        Box::new(event_loop.owned_display_handle()),
    ));
    let mut state = SessionState::new("");
    state.status = "connecting\u{2026}".to_string();
    let mut viewer = Viewer {
        window: None,
        renderer: None,
        instance,
        core: Some(core),
        state,
        windows: Vec::new(),
        active: StreamSource::None {},
        allow_pointer: opts.allow_local_pointer,
        allow_keyboard: opts.allow_local_keyboard,
        local_notice_shown: false,
        fullscreen: false,
        exit_code: 0,
    };
    event_loop.run_app(&mut viewer)?;
    // Give the supervisor a moment to flush the queued button-ups before the runtime goes.
    if let Some(core) = viewer.core.take() {
        core.close(rt.handle(), Duration::from_millis(300));
    }
    Ok(viewer.exit_code)
}

impl Viewer {
    fn update_title(&self) {
        let (Some(window), Some(core)) = (&self.window, &self.core) else {
            return;
        };
        window.set_title(&self.state.title_text(core.view_only()));
    }

    fn handle_net(&mut self, el: &ActiveEventLoop) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        let events = core.take_events();
        let now = Instant::now();
        for ev in &events {
            match ev {
                SessionEvent::Connected {
                    local_peer: true, ..
                } if (!self.allow_keyboard || !self.allow_pointer) && !self.local_notice_shown => {
                    self.local_notice_shown = true;
                    let mut off = Vec::new();
                    if !self.allow_keyboard {
                        off.push("keyboard input is off (it would loop; --allow-local-keyboard turns it on)");
                    }
                    if !self.allow_pointer {
                        off.push("pointer input is off (--allow-local-pointer turns it on)");
                    }
                    eprintln!("This Mac is the one you are viewing: {}.", off.join("; "));
                }
                SessionEvent::WindowList(l) => {
                    self.windows = l.windows.clone();
                    self.active = l.active;
                }
                SessionEvent::Finished { text, .. } => {
                    eprintln!("{}", transom_proto::safetext::label(text, 300));
                    self.exit_code = 1;
                    el.exit();
                }
                _ => {}
            }
            self.state.apply(ev.clone(), now);
        }
        if let Some(window) = &self.window {
            core.apply_window_effects(el, window, self.renderer.as_mut(), &events);
            window.set_title(&self.state.title_text(core.view_only()));
        }
    }

    fn handle_frame(&mut self) {
        if let (Some(core), Some(r), Some(w)) =
            (self.core.as_mut(), self.renderer.as_mut(), &self.window)
        {
            core.take_frame(r, w);
        }
    }

    fn close(&mut self, el: &ActiveEventLoop) {
        if let Some(core) = &mut self.core {
            core.release_all();
            core.send(transom_net::session::SessionCommand::Close);
        }
        self.exit_code = 0;
        el.exit();
    }

    fn shortcut(&mut self, chord: Chord, el: &ActiveEventLoop) {
        match chord {
            Chord::D => {
                if let Some(core) = &self.core {
                    core.select(StreamSource::Desktop {});
                }
            }
            Chord::W => {
                if self.windows.is_empty() {
                    self.state.status = "No windows on the Mac".to_string();
                    eprintln!("transom: {}", self.state.status);
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
                if let Some(core) = &self.core {
                    core.select(StreamSource::Window { id });
                }
            }
            Chord::F => {
                self.fullscreen = !self.fullscreen;
                if let Some(w) = &self.window {
                    w.set_fullscreen(self.fullscreen.then_some(Fullscreen::Borderless(None)));
                }
            }
            Chord::Q => self.close(el),
            _ => {}
        }
    }
}

impl ApplicationHandler<UserEvent> for Viewer {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Transom \u{2014} connecting\u{2026}")
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
        if let (Some(core), Some(r)) = (self.core.as_mut(), self.renderer.as_ref()) {
            core.request_viewport(Instant::now() + VIEWPORT_DELAY);
            core.on_scale_factor(el, &window, r);
        }
        self.window = Some(window);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Net => self.handle_net(el),
            UserEvent::Frame => self.handle_frame(),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(_) => {
                if let (Some(core), Some(r), Some(w)) =
                    (self.core.as_mut(), self.renderer.as_mut(), &self.window)
                {
                    core.on_resized(w, r);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let (Some(core), Some(r), Some(w)) =
                    (self.core.as_mut(), self.renderer.as_ref(), &self.window)
                {
                    core.on_scale_factor(el, w, r);
                }
            }
            WindowEvent::RedrawRequested => {
                let ok = self.renderer.as_mut().is_none_or(|r| r.render());
                if !ok {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                if let Some(core) = &mut self.core {
                    core.set_mods(m.state());
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                if let (PhysicalKey::Code(code), Some(core)) = (event.physical_key, &mut self.core)
                {
                    let outcome = core.key(
                        code,
                        event.state == ElementState::Pressed,
                        event.repeat,
                        true,
                    );
                    if let KeyOutcome::Chord(c) = outcome {
                        self.shortcut(c, el);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(core) = &mut self.core {
                    core.pointer_moved(position.x, position.y, true);
                    if let Some(w) = &self.window {
                        core.apply_cursor(w, true);
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(core) = &mut self.core {
                    core.pointer_left();
                    if let Some(w) = &self.window {
                        core.apply_cursor(w, true);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(core) = &mut self.core {
                    core.pointer_button(button, state == ElementState::Pressed, true);
                }
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                if let Some(core) = &mut self.core {
                    core.pointer_wheel(delta, phase, true);
                }
            }
            WindowEvent::Focused(false) => {
                if let Some(core) = &mut self.core {
                    core.release_all();
                }
            }
            WindowEvent::CloseRequested => self.close(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let due = match (&mut self.core, &self.window) {
            (Some(core), Some(window)) => core.viewport_tick(window, Instant::now()),
            (Some(core), None) => core.viewport_due(),
            _ => None,
        };
        el.set_control_flow(match due {
            Some(due) => ControlFlow::WaitUntil(due),
            None => ControlFlow::Wait,
        });
    }
}

//! The winit `ApplicationHandler` of the graphical app: window, renderer, egui, routing of
//! keyboard and pointer, redraw (docs/ui-spec.md sections 6 and 8.3). Every message from any
//! source goes through `App::apply`, so the keyboard mode is recomputed (and held keys are
//! released on a Mac-to-Transom change) after each model update. On Windows, in full screen with
//! the Mac in charge of the keyboard, the low-level keyboard hook (winhook.rs) takes the keys
//! Windows keeps for itself and hands them back here as `UserEvent::SysKey`.

use crate::gui::effects::{Effects, Platform};
use crate::gui::images::ImageCache;
use crate::gui::keyboard::{capture_wanted, keyboard_mode, transition, KeyboardMode, ModeInputs};
use crate::gui::model::{Effect, IdentityInfo, Model, Msg, Prefs, StreamPanel};
use crate::gui::overlay::EguiOverlay;
use crate::gui::texts;
use crate::gui::view::{self, theme, Drafts};
use crate::session_state::Phase;
use crate::stream::Chord;
use crate::stream::KeyOutcome;
use anyhow::{anyhow, Result};
use crossbeam_channel::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transom_net::discovery::{self, BrowseEvent, Watch};
use transom_net::identity::DeviceIdentity;
use transom_net::store::Store;
use transom_render::{wgpu, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Icon, Window, WindowId};

/// The window icon (a placeholder; docs/ui-spec.md section 7.1).
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/icons/transom-64.png");
/// No model tick more often than this, so a deadline that has passed but is not yet consumed
/// cannot spin the loop.
const MIN_TICK_GAP: Duration = Duration::from_millis(50);

/// Wake-ups from other threads.
pub enum UserEvent {
    /// The session produced events.
    Net,
    /// A frame was decoded.
    Frame,
    /// A message or a Bonjour event is queued.
    Msg,
    /// egui wants a frame now.
    Repaint,
    AccessKit(egui_winit::accesskit_winit::Event),
    /// A key the Windows keyboard hook took from Windows (full screen, Mac mode).
    #[cfg(windows)]
    SysKey(crate::gui::capture::SysKey),
}

impl From<egui_winit::accesskit_winit::Event> for UserEvent {
    fn from(e: egui_winit::accesskit_winit::Event) -> Self {
        UserEvent::AccessKit(e)
    }
}

/// Everything that exists only while the window does.
struct Gfx {
    renderer: Renderer,
    overlay: EguiOverlay,
    state: egui_winit::State,
}

struct WinPlatform<'a> {
    window: &'a Window,
    ctx: &'a egui::Context,
}

impl Platform for WinPlatform<'_> {
    fn set_fullscreen(&mut self, on: bool) {
        self.window
            .set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
    }
    fn copy_text(&mut self, s: &str) {
        self.ctx.copy_text(s.to_string());
    }
    fn open_url(&mut self, url: &str) {
        self.ctx.open_url(egui::OpenUrl::new_tab(url));
    }
}

/// Before the window exists nothing can be asked of the window system.
struct NoPlatform;

impl Platform for NoPlatform {
    fn set_fullscreen(&mut self, _on: bool) {}
    fn copy_text(&mut self, _s: &str) {}
    fn open_url(&mut self, _url: &str) {}
}

struct App {
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    instance: wgpu::Instance,
    model: Model,
    drafts: Drafts,
    effects: Effects,
    images: ImageCache,
    msg_rx: Receiver<Msg>,
    browse_rx: Receiver<BrowseEvent>,
    watch: Option<Watch>,
    ctx: egui::Context,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    mods: ModifiersState,
    mode: KeyboardMode,
    /// The Windows keyboard hook is taking the OS-reserved keys (`capture_wanted`).
    capturing: bool,
    #[cfg(windows)]
    hook: crate::gui::winhook::Hook,
    focused: bool,
    last_over_ui: bool,
    last_wants_kb: bool,
    /// The video must be blanked (a stream started or stopped).
    clear_video: bool,
    /// When egui asked to be drawn again (from the last pass's `repaint_delay`).
    repaint_at: Option<Instant>,
    last_tick: Instant,
    title: String,
    exit_code: i32,
}

/// Builds the event loop and runs the app until its window closes. The runtime is owned by the
/// caller and must outlive this call; do not hold `rt.enter()` on this thread (the stream
/// waits for its supervisor only when no runtime is entered).
pub fn run(
    rt: &tokio::runtime::Runtime,
    store: Store,
    identity: DeviceIdentity,
    prefs: Prefs,
    info: IdentityInfo,
) -> Result<i32> {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_default_menu(false);
    }
    let el = builder.build()?;
    // Nothing uses raw input (device events), and raw keyboard input may keep the keyboard hook
    // from being called.
    #[cfg(windows)]
    el.listen_device_events(winit::event_loop::DeviceEvents::Never);
    let proxy = el.create_proxy();
    let wake = |make: fn() -> UserEvent| -> crate::stream::Wake {
        let p = proxy.clone();
        Arc::new(move || {
            let _ = p.send_event(make());
        })
    };
    let wake_msg = wake(|| UserEvent::Msg);
    let wake_net = wake(|| UserEvent::Net);
    let wake_frame = wake(|| UserEvent::Frame);

    let saved = store.macs().unwrap_or_default();
    let (msg_tx, msg_rx) = crossbeam_channel::unbounded();
    let (browse_tx, browse_rx) = crossbeam_channel::unbounded();
    let watch = match discovery::watch(browse_tx, wake_msg.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            log::warn!("Bonjour discovery is not running: {e}");
            None
        }
    };
    let effects = Effects::new(
        rt.handle().clone(),
        store,
        identity,
        msg_tx,
        wake_msg,
        wake_net,
        wake_frame,
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
        Box::new(el.owned_display_handle()),
    ));
    #[cfg(windows)]
    let hook = {
        let p = proxy.clone();
        crate::gui::winhook::Hook::new(Box::new(move |k: crate::gui::capture::SysKey| {
            let _ = p.send_event(UserEvent::SysKey(k));
        }))
    };
    let mut app = App {
        proxy,
        instance,
        model: Model::new(info, prefs, saved, Instant::now()),
        drafts: Drafts::default(),
        effects,
        images: ImageCache::new(),
        msg_rx,
        browse_rx,
        watch,
        ctx: egui::Context::default(),
        window: None,
        gfx: None,
        mods: ModifiersState::empty(),
        mode: KeyboardMode::Transom,
        capturing: false,
        #[cfg(windows)]
        hook,
        focused: false,
        last_over_ui: false,
        last_wants_kb: false,
        clear_video: false,
        repaint_at: None,
        last_tick: Instant::now(),
        title: texts::WINDOW_TITLE.to_string(),
        exit_code: 0,
    };
    el.run_app(&mut app)
        .map_err(|e| anyhow!("the event loop failed: {e}"))?;
    app.shutdown();
    Ok(app.exit_code)
}

fn load_icon() -> Option<Icon> {
    let img = match image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            log::warn!("could not decode the window icon: {e}");
            return None;
        }
    };
    let (w, h) = (img.width(), img.height());
    match Icon::from_rgba(img.into_raw(), w, h) {
        Ok(i) => Some(i),
        Err(e) => {
            log::warn!("could not use the window icon: {e}");
            None
        }
    }
}

impl App {
    /// Closes the stream (waiting briefly for queued input to flush) and stops discovery.
    fn shutdown(&mut self) {
        // The hook goes first: this thread blocks in `close` below.
        #[cfg(windows)]
        self.hook.stop();
        if let Some(core) = self.effects.stream_mut() {
            core.release_all();
        }
        self.effects.close(Duration::from_millis(300));
        if let Some(w) = self.watch.take() {
            w.stop();
        }
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// The one path every message takes: model update, effects, keyboard mode.
    fn apply(&mut self, msg: Msg) {
        let fx = self.model.update(msg, Instant::now());
        self.run_effects(fx, true);
    }

    fn run_effects(&mut self, fx: Vec<Effect>, redraw: bool) {
        for e in fx {
            let resets = matches!(e, Effect::StartStream { .. } | Effect::StopStream);
            let starts = matches!(e, Effect::StartStream { .. });
            let prefs = self.model.prefs.clone();
            if resets {
                // The Mac's cursor must not outlive the stream: put the arrow back first.
                if let (Some(core), Some(w)) = (self.effects.stream_mut(), self.window.as_ref()) {
                    core.apply_cursor(w, false);
                }
            }
            match &self.window {
                Some(w) => {
                    let mut p = WinPlatform {
                        window: w,
                        ctx: &self.ctx,
                    };
                    self.effects.run(e, &prefs, &mut p);
                }
                None => self.effects.run(e, &prefs, &mut NoPlatform),
            }
            if resets {
                self.images.clear();
                self.clear_video = true;
            }
            if starts {
                let mods = self.mods;
                if let Some(core) = self.effects.stream_mut() {
                    core.set_mods(mods);
                    core.request_viewport(Instant::now());
                }
            }
        }
        self.recompute_mode();
        if redraw {
            self.request_redraw();
        }
    }

    /// Spec 6.1: recomputed before every key event, after every model update and after every
    /// frame. Everything held on the Mac is released at each Mac-to-Transom change and whenever
    /// the Windows keyboard hook stops capturing.
    fn recompute_mode(&mut self) {
        let stream = self.model.stream();
        let core = self.effects.stream();
        let inputs = ModeInputs {
            connected: stream.is_some_and(|s| matches!(s.session.phase, Phase::Connected)),
            forward_keys: core.is_some_and(|c| c.forward_keys()),
            focused: self.focused,
            overlay_open: self.model.overlay_open(),
            egui_wants_keyboard: self.last_wants_kb,
        };
        let next = keyboard_mode(&inputs);
        let capture = capture_wanted(next, self.model.fullscreen, cfg!(windows));
        let stop_capture = self.capturing && !capture;
        if capture != self.capturing {
            self.capturing = capture;
            #[cfg(windows)]
            self.set_hook(capture);
        }
        // The hook keeps swallowing a captured key until it is released, and drops presses that
        // arrive after capture stopped, so let go of everything on the Mac now.
        if transition(self.mode, next) || stop_capture {
            if let Some(core) = self.effects.stream_mut() {
                core.release_all();
            }
        }
        self.mode = next;
    }

    /// Starts or stops the keyboard hook's capture.
    #[cfg(windows)]
    fn set_hook(&mut self, on: bool) {
        let hwnd = self
            .window
            .as_deref()
            .and_then(crate::gui::winhook::hwnd_of);
        match hwnd {
            Some(h) if on => self.hook.activate(h),
            _ => self.hook.deactivate(),
        }
    }

    /// A key the hook took from Windows. A press counts only while capturing; a release always
    /// goes through, so nothing stays down on the Mac. Never given to egui.
    #[cfg(windows)]
    fn sys_key(&mut self, k: crate::gui::capture::SysKey) {
        self.recompute_mode();
        if k.pressed && !self.capturing {
            return;
        }
        let outcome = match self.effects.stream_mut() {
            Some(core) => core.key(k.code, k.pressed, k.repeat, true),
            None => return,
        };
        if let KeyOutcome::Chord(c) = outcome {
            self.chord(c);
        }
    }

    /// Pointer events reach the Mac only while connected, away from Transom's own UI (as of the
    /// last frame) and with no dialog open.
    fn forward_pointer(&self) -> bool {
        self.model
            .stream()
            .is_some_and(|s| matches!(s.session.phase, Phase::Connected))
            && !self.last_over_ui
            && self.model.dialog.is_none()
    }

    fn feed_egui(&mut self, event: &WindowEvent) {
        if let (Some(g), Some(w)) = (self.gfx.as_mut(), self.window.as_ref()) {
            if g.state.on_window_event(w, event).repaint {
                w.request_redraw();
            }
        }
    }

    fn chord(&mut self, c: Chord) {
        let msg = match c {
            Chord::B => Msg::ToggleBar,
            Chord::D => Msg::SelectDesktop,
            Chord::W => Msg::NextWindow,
            Chord::S => Msg::OpenPanel(StreamPanel::MacSettings),
            Chord::F => Msg::ToggleFullscreen,
            Chord::Q => Msg::Disconnect,
            Chord::A => return,
        };
        self.apply(msg);
    }

    fn check_fullscreen(&mut self) {
        if let Some(w) = &self.window {
            let now = w.fullscreen().is_some();
            if now != self.model.fullscreen {
                self.apply(Msg::FullscreenChanged(now));
            }
        }
    }

    fn handle_net(&mut self, el: &ActiveEventLoop) {
        let events = self.effects.session_events();
        if events.is_empty() {
            return;
        }
        if let (Some(core), Some(g), Some(w)) = (
            self.effects.stream_mut(),
            self.gfx.as_mut(),
            self.window.as_ref(),
        ) {
            core.apply_window_effects(el, w, Some(&mut g.renderer), &events);
        }
        for ev in events {
            self.apply(Msg::Session(ev));
        }
        self.request_redraw();
    }

    fn handle_msgs(&mut self) {
        let browse: Vec<BrowseEvent> = self.browse_rx.try_iter().collect();
        for b in browse {
            self.apply(Msg::Browse(b));
        }
        let msgs: Vec<Msg> = self.msg_rx.try_iter().collect();
        for m in msgs {
            self.apply(m);
        }
    }

    fn close(&mut self, el: &ActiveEventLoop) {
        self.shutdown();
        self.exit_code = 0;
        el.exit();
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        // 1. The newest decoded frame.
        let mut shown = false;
        if let Some(g) = self.gfx.as_mut() {
            if std::mem::take(&mut self.clear_video) {
                g.renderer.clear();
            }
            if let Some(core) = self.effects.stream_mut() {
                shown = core.take_frame(&mut g.renderer, &window);
            }
        }
        if shown {
            // The pass below already draws the update: no second redraw for it.
            let fx = self.model.update(Msg::FrameShown, Instant::now());
            self.run_effects(fx, false);
        }
        let now = Instant::now();
        // 2. Pictures, 3-5. The egui pass.
        let mut more_images = false;
        if let Some(s) = self.model.stream() {
            more_images = self.images.prepare(&self.ctx, &s.session);
        }
        let Some(g) = self.gfx.as_mut() else {
            return;
        };
        let raw = g.state.take_egui_input(&window);
        self.ctx.set_zoom_factor(self.model.prefs.ui_zoom);
        let mut out: Vec<Msg> = Vec::new();
        let mut scratch: Vec<Msg> = Vec::new();
        let mut full = {
            let (model, drafts, images) = (&self.model, &mut self.drafts, &self.images);
            self.ctx.run_ui(raw, |ui| {
                // egui may run the pass twice and gives the input events to pass 0 only, so
                // pass 0's messages are the ones that count; later passes draw into a scratch.
                if ui.ctx().current_pass_index() == 0 {
                    view::draw(ui, model, drafts, images, now, &mut out);
                } else {
                    scratch.clear();
                    view::draw(ui, model, drafts, images, now, &mut scratch);
                }
            })
        };
        // 6.
        self.last_over_ui = self.ctx.is_pointer_over_egui() || self.ctx.egui_is_using_pointer();
        self.last_wants_kb = self.ctx.egui_wants_keyboard_input();
        // 7. The cursor: the arrow before egui's output, the Mac's cursor after it.
        let connected = self
            .model
            .stream()
            .is_some_and(|s| matches!(s.session.phase, Phase::Connected));
        let show_mac = connected && !self.last_over_ui;
        if !show_mac {
            if let Some(core) = self.effects.stream_mut() {
                core.apply_cursor(&window, false);
            }
        }
        g.state
            .handle_platform_output_with_event_loop(&window, el, full.platform_output);
        if show_mac {
            if let Some(core) = self.effects.stream_mut() {
                core.apply_cursor(&window, true);
            }
        }
        // 8. Textures, always drained.
        for (id, deltas) in full.textures_delta.set.drain() {
            for d in deltas {
                g.overlay
                    .renderer
                    .update_texture(g.renderer.device(), g.renderer.queue(), id, &d);
            }
        }
        // 9.
        g.overlay.jobs = self.ctx.tessellate(full.shapes, full.pixels_per_point);
        let (w, h) = g.renderer.surface_size();
        g.overlay.screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: full.pixels_per_point,
        };
        // 10.
        if !g.renderer.render_with(Some(&mut g.overlay)) {
            window.request_redraw();
        }
        // 11.
        for id in full.textures_delta.free.drain() {
            g.overlay.renderer.free_texture(&id);
        }
        // 14.
        let delay = full
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay);
        self.repaint_at = if delay == Duration::MAX {
            None
        } else {
            Instant::now().checked_add(delay)
        };
        if more_images {
            window.request_redraw();
        }
        // 12.
        for m in out {
            self.apply(m);
        }
        // Step 6 may have given the keyboard to a text field, or taken it back, with no message.
        self.recompute_mode();
        // 13.
        let title = match self.model.stream() {
            Some(s) => texts::window_title_streaming(&s.session.mac_name),
            None => texts::WINDOW_TITLE.to_string(),
        };
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(texts::WINDOW_TITLE)
            .with_inner_size(LogicalSize::new(1100.0, 720.0))
            .with_visible(false)
            .with_window_icon(load_icon());
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        ))]
        let attrs =
            winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "transom", "transom");
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("transom: could not create the window: {e}");
                self.exit_code = 1;
                el.exit();
                return;
            }
        };
        let renderer = match Renderer::new(&self.instance, window.clone()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("transom: could not start the renderer: {e:#}");
                self.exit_code = 1;
                el.exit();
                return;
            }
        };
        let overlay = EguiOverlay::new(renderer.device(), renderer.format());
        theme::apply(&self.ctx);
        let mut state = egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            el,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(renderer.device().limits().max_texture_dimension_2d as usize),
        );
        state.init_accesskit(el, &window, self.proxy.clone());
        window.set_visible(true);
        let proxy = self.proxy.clone();
        self.ctx.set_request_repaint_callback(move |info| {
            // Delayed requests come back as the pass's `repaint_delay`.
            if info.delay.is_zero() {
                let _ = proxy.send_event(UserEvent::Repaint);
            }
        });
        self.gfx = Some(Gfx {
            renderer,
            overlay,
            state,
        });
        self.window = Some(window.clone());
        window.request_redraw();
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Net => self.handle_net(el),
            UserEvent::Frame | UserEvent::Repaint => self.request_redraw(),
            UserEvent::Msg => self.handle_msgs(),
            #[cfg(windows)]
            UserEvent::SysKey(k) => self.sys_key(k),
            UserEvent::AccessKit(e) => {
                use egui_winit::accesskit_winit::WindowEvent as Ak;
                match e.window_event {
                    Ak::InitialTreeRequested => self.ctx.enable_accesskit(),
                    Ak::ActionRequested(req) => {
                        if let Some(g) = self.gfx.as_mut() {
                            g.state.on_accesskit_action_request(req);
                        }
                    }
                    Ak::AccessibilityDeactivated => {}
                }
                self.request_redraw();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CloseRequested => self.close(el),
            WindowEvent::RedrawRequested => self.redraw(el),
            WindowEvent::KeyboardInput {
                event: ke,
                is_synthetic: false,
                ..
            } => {
                self.recompute_mode();
                if let (PhysicalKey::Code(code), true) =
                    (ke.physical_key, self.effects.stream().is_some())
                {
                    let mac = self.mode == KeyboardMode::Mac;
                    let outcome = match self.effects.stream_mut() {
                        Some(core) => {
                            core.key(code, ke.state == ElementState::Pressed, ke.repeat, mac)
                        }
                        None => KeyOutcome::NotForwarded,
                    };
                    match outcome {
                        KeyOutcome::Chord(c) => {
                            self.chord(c);
                            return;
                        }
                        KeyOutcome::Swallowed => {
                            // Egui never saw the press; a stray release is harmless.
                            if ke.state == ElementState::Released {
                                self.feed_egui(&event);
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                // A release always reaches egui: a key pressed in Transom mode and released in
                // Mac mode would otherwise stay down in egui's keys_down.
                if self.mode == KeyboardMode::Transom
                    || self.effects.stream().is_none()
                    || ke.state == ElementState::Released
                {
                    self.feed_egui(&event);
                }
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                if self.mode == KeyboardMode::Transom
                    || self.effects.stream().is_none()
                    || ke.state == ElementState::Released
                {
                    self.feed_egui(&event);
                }
            }
            WindowEvent::Ime(_) => {
                if self.mode == KeyboardMode::Transom || self.effects.stream().is_none() {
                    self.feed_egui(&event);
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                if let Some(core) = self.effects.stream_mut() {
                    core.set_mods(self.mods);
                }
                self.feed_egui(&event);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.feed_egui(&event);
                let forward = self.forward_pointer();
                if let Some(core) = self.effects.stream_mut() {
                    core.pointer_moved(position.x, position.y, forward);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.feed_egui(&event);
                let forward = self.forward_pointer();
                if let Some(core) = self.effects.stream_mut() {
                    core.pointer_button(*button, *state == ElementState::Pressed, forward);
                }
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                self.feed_egui(&event);
                let forward = self.forward_pointer();
                if let Some(core) = self.effects.stream_mut() {
                    core.pointer_wheel(*delta, *phase, forward);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.feed_egui(&event);
                if let Some(core) = self.effects.stream_mut() {
                    core.pointer_left();
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = *f;
                if !*f {
                    if let Some(core) = self.effects.stream_mut() {
                        core.release_all();
                    }
                }
                self.feed_egui(&event);
                self.recompute_mode();
            }
            WindowEvent::Resized(size) => {
                if let (Some(g), Some(w)) = (self.gfx.as_mut(), self.window.as_ref()) {
                    match self.effects.stream_mut() {
                        Some(core) => core.on_resized(w, &mut g.renderer),
                        None => g.renderer.resize(size.width, size.height),
                    }
                }
                self.feed_egui(&event);
                self.check_fullscreen();
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let (Some(g), Some(w), Some(core)) = (
                    self.gfx.as_ref(),
                    self.window.as_ref(),
                    self.effects.stream_mut(),
                ) {
                    core.on_scale_factor(el, w, &g.renderer);
                }
                self.feed_egui(&event);
            }
            _ => self.feed_egui(&event),
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        let mut take = |t: Instant| next = Some(next.map_or(t, |n| n.min(t)));
        if let Some(d) = self.model.next_deadline(now) {
            let earliest = self.last_tick + MIN_TICK_GAP;
            let due = d.max(earliest);
            if due <= now {
                self.last_tick = now;
                self.apply(Msg::Tick);
                if let Some(d) = self.model.next_deadline(now) {
                    take(d.max(now + MIN_TICK_GAP));
                }
            } else {
                take(due);
            }
        }
        if let (Some(core), Some(w)) = (self.effects.stream_mut(), self.window.as_ref()) {
            if let Some(t) = core.viewport_tick(w, now) {
                take(t);
            }
        }
        if let Some(t) = self.repaint_at {
            if t <= now {
                self.repaint_at = None;
                self.request_redraw();
            } else {
                take(t);
            }
        }
        el.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}

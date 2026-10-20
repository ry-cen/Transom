//! Manual GPU check: shows a synthetic gradient for 5 seconds, then exits.
//! It is uploaded as I420, then as NV12 after 2.5 s: the picture must not change at the switch.
use std::sync::Arc;
use std::time::{Duration, Instant};

use transom_render::wgpu;
use transom_render::{Chroma, Renderer, YuvFrame};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const W: usize = 320;
const H: usize = 180;

struct App {
    instance: wgpu::Instance,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    deadline: Instant,
    switch_at: Instant,
    switched: bool,
}

impl App {
    fn upload(&mut self, nv12: bool) {
        let y: Vec<u8> = (0..W * H)
            .map(|i| ((i % W) * 255 / (W - 1)) as u8)
            .collect();
        let (cw, ch) = (W / 2, H / 2);
        let u: Vec<u8> = (0..cw * ch)
            .map(|i| ((i / cw) * 255 / (ch - 1)) as u8)
            .collect();
        let v = vec![128u8; cw * ch];
        let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(&u, &v)| [u, v]).collect();
        let chroma = if nv12 {
            Chroma::Nv12 {
                uv: &uv,
                uv_stride: 2 * cw as u32,
            }
        } else {
            Chroma::I420 {
                u: &u,
                u_stride: cw as u32,
                v: &v,
                v_stride: cw as u32,
            }
        };
        if let Some(r) = self.renderer.as_mut() {
            r.upload(&YuvFrame {
                width: W as u32,
                height: H as u32,
                y: &y,
                y_stride: W as u32,
                chroma,
                full_range: true,
                bt601: false,
            });
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("transom test pattern")
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("window: {e}");
                event_loop.exit();
                return;
            }
        };
        match Renderer::new(&self.instance, window.clone()) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                eprintln!("renderer: {e:#}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        let now = Instant::now();
        self.deadline = now + Duration::from_secs(5);
        self.switch_at = now + Duration::from_millis(2500);
        self.upload(false);
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.switch_at));
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(s) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(s.width, s.height);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(r) = self.renderer.as_mut() {
                    r.render();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }
        let now = Instant::now();
        if now >= self.deadline {
            event_loop.exit();
            return;
        }
        if !self.switched && now >= self.switch_at {
            self.upload(true);
            self.switched = true;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
        Box::new(event_loop.owned_display_handle()),
    ));
    let mut app = App {
        instance,
        window: None,
        renderer: None,
        deadline: Instant::now() + Duration::from_secs(5),
        switch_at: Instant::now() + Duration::from_millis(2500),
        switched: false,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

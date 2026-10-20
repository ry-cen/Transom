//! egui drawn inside the video's render pass (`transom_render::Overlay`), docs/ui-spec.md
//! section 8.3.

use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use transom_render::wgpu;

/// egui's wgpu renderer plus the frame's paint jobs. The shell fills `jobs` and `screen` each
/// frame; `Renderer::render_with` calls `prepare` and `paint`.
pub struct EguiOverlay {
    pub renderer: Renderer,
    pub jobs: Vec<egui::ClippedPrimitive>,
    pub screen: ScreenDescriptor,
}

impl EguiOverlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> EguiOverlay {
        EguiOverlay {
            renderer: Renderer::new(device, format, RendererOptions::default()),
            jobs: Vec::new(),
            screen: ScreenDescriptor {
                size_in_pixels: [1, 1],
                pixels_per_point: 1.0,
            },
        }
    }
}

impl transom_render::Overlay for EguiOverlay {
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_px: (u32, u32),
    ) -> Vec<wgpu::CommandBuffer> {
        // The surface may have been resized since the jobs were made; egui scales by this.
        self.screen.size_in_pixels = [surface_px.0, surface_px.1];
        self.renderer
            .update_buffers(device, queue, encoder, &self.jobs, &self.screen)
    }

    fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        self.renderer.render(pass, &self.jobs, &self.screen);
    }
}

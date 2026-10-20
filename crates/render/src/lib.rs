//! Draws decoded 8-bit 4:2:0 frames (I420 or NV12), letterboxed on black, into a winit window with
//! wgpu; the YUV to RGB conversion runs in the fragment shader. NV12's interleaved UV plane is
//! uploaded as an Rg8 texture and sampled from it. Wave 0 ships the renderer complete (it was
//! compiled and run against wgpu 30.0.1 and winit 0.30.13 on macOS). WP3 adds `pub mod cursor;`
//! and the tests, and changes nothing else here without a reason it reports. The app reaches
//! wgpu through the `wgpu` re-export below (it has no wgpu dependency of its own).
use std::sync::Arc;

use transom_proto::geometry::letterbox;
/// The wgpu this renderer uses: the app builds its `wgpu::Instance` through it.
pub use wgpu;
use winit::window::Window;

pub mod cursor;
mod frame;

use frame::{plane_copies, PlaneCopy};
pub use frame::{Chroma, FrameError, YuvFrame};

const SHADER: &str = r#"
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // Two triangles covering the viewport; the viewport itself is the letterbox rectangle.
    var p = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
                                vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    var o: VsOut;
    o.pos = vec4(p[i], 0.0, 1.0);
    o.uv = vec2((p[i].x + 1.0) * 0.5, 1.0 - (p[i].y + 1.0) * 0.5);
    return o;
}
struct Params { full_range: u32, bt601: u32, nv12: u32, pad1: u32 };
@group(0) @binding(0) var tex_y: texture_2d<f32>;
@group(0) @binding(1) var tex_u: texture_2d<f32>;
@group(0) @binding(2) var tex_v: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<uniform> params: Params;
@fragment
fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    var y = textureSample(tex_y, samp, v.uv).r;
    let c0 = textureSample(tex_u, samp, v.uv);
    let c1 = textureSample(tex_v, samp, v.uv);
    var cb = c0.r - 0.5;
    var cr = select(c1.r, c0.g, params.nv12 == 1u) - 0.5;
    if (params.full_range == 0u) {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        cb = cb * (255.0 / 224.0);
        cr = cr * (255.0 / 224.0);
    }
    var rgb: vec3<f32>;
    if (params.bt601 == 1u) {
        rgb = vec3(y + 1.402 * cr, y - 0.344136 * cb - 0.714136 * cr, y + 1.772 * cb);
    } else {
        rgb = vec3(y + 1.5748 * cr, y - 0.1873 * cb - 0.4681 * cr, y + 1.8556 * cb);
    }
    return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), 1.0);
}
"#;

/// Something drawn over the video in the same render pass (the GUI's egui output).
pub trait Overlay {
    /// Before the pass: upload buffers. The returned buffers are submitted before the frame's
    /// encoder.
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_px: (u32, u32),
    ) -> Vec<wgpu::CommandBuffer>;
    /// Inside the pass, after the video.
    fn paint(&self, pass: &mut wgpu::RenderPass<'static>);
}

/// The window's GPU surface and the video pipeline. Main thread only.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    dummy: wgpu::TextureView,
    bad_frame_logged: bool,
    video: Option<Planes>,
}

/// The plane textures of the current frame size and layout, and the bind group over them.
struct Planes {
    y: wgpu::Texture,
    c0: wgpu::Texture,
    c1: Option<wgpu::Texture>,
    bind: wgpu::BindGroup,
    width: u32,
    height: u32,
    nv12: bool,
}

impl Renderer {
    /// Makes the surface for `window` and the pipeline. `instance` is made once per process with
    /// `wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
    /// event_loop.owned_display_handle())))`.
    pub fn new(instance: &wgpu::Instance, window: Arc<Window>) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let surface = instance.create_surface(window)?;
        // The fast GPU: on a desktop with an enabled integrated GPU (every Ryzen 7000, many Intel
        // chips) the low-power one cannot keep up with 4K. `WGPU_POWER_PREF=low` overrides it.
        let power_preference =
            wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let info = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, {:?}, driver {} {})",
            info.name,
            info.device_type,
            info.backend,
            info.driver,
            info.driver_info
        );
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("transom"),
                required_features: wgpu::Features::empty(),
                required_limits:
                    wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow::anyhow!("the window's surface is not supported by this GPU"))?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 1;
        // Draw the frame's sRGB bytes as they are: a non-sRGB view keeps the picture's colours.
        config.format = config.format.remove_srgb_suffix();
        surface.configure(&device, &config);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("video dummy"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            layout,
            sampler,
            params,
            dummy,
            bad_frame_logged: false,
            video: None,
        })
    }

    /// The window's new inner size in physical pixels (WindowEvent::Resized).
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Uploads one frame; the next `render` draws it. A frame whose strides or plane lengths do
    /// not fit its size is logged once and skipped.
    pub fn upload(&mut self, f: &YuvFrame<'_>) {
        let copies = match plane_copies(f) {
            Ok(c) => c,
            Err(e) => {
                if !self.bad_frame_logged {
                    log::warn!("video frame skipped: {e}");
                    self.bad_frame_logged = true;
                }
                return;
            }
        };
        self.bad_frame_logged = false;
        let nv12 = matches!(f.chroma, Chroma::Nv12 { .. });
        let same = matches!(
            &self.video,
            Some(p) if p.width == f.width && p.height == f.height && p.nv12 == nv12
        );
        if !same {
            let plane = |c: &PlaneCopy<'_>| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("video plane"),
                    size: wgpu::Extent3d {
                        width: c.width,
                        height: c.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: c.format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let y = plane(&copies[0]);
            let c0 = plane(&copies[1]);
            let c1 = copies.get(2).map(plane);
            let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
            let (y_view, c0_view) = (view(&y), view(&c0));
            let c1_view = c1.as_ref().map(view);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&y_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&c0_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            c1_view.as_ref().unwrap_or(&self.dummy),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            });
            self.video = Some(Planes {
                y,
                c0,
                c1,
                bind,
                width: f.width,
                height: f.height,
                nv12,
            });
        }
        let planes = self.video.as_ref().expect("planes were just made");
        let write = |texture: &wgpu::Texture, copy: &PlaneCopy<'_>| {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                copy.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(copy.stride),
                    rows_per_image: Some(copy.height),
                },
                wgpu::Extent3d {
                    width: copy.width,
                    height: copy.height,
                    depth_or_array_layers: 1,
                },
            );
        };
        write(&planes.y, &copies[0]);
        write(&planes.c0, &copies[1]);
        if let (Some(t), Some(c)) = (&planes.c1, copies.get(2)) {
            write(t, c);
        }
        let words: [u32; 4] = [f.full_range as u32, f.bt601 as u32, nv12 as u32, 0];
        let mut bytes = [0u8; 16];
        for (i, w) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.queue.write_buffer(&self.params, 0, &bytes);
    }

    /// The surface size in physical pixels, as last configured.
    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The size of the last uploaded frame, if any.
    pub fn video_size(&self) -> Option<(u32, u32)> {
        self.video.as_ref().map(|p| (p.width, p.height))
    }

    /// Forgets the last frame: the next `render` draws black until a new `upload`.
    pub fn clear(&mut self) {
        self.video = None;
    }

    /// The GPU device (for an overlay's own resources).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The GPU queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The surface's texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Clears to black and draws the last uploaded frame letterboxed. Returns false when the frame
    /// was skipped (surface busy or reconfigured): call again on the next redraw.
    pub fn render(&mut self) -> bool {
        self.render_with(None)
    }

    /// Like `render`, then draws `overlay` over the video in the same pass. The overlay's
    /// `prepare` is not called when the frame is skipped. Texture uploads and frees are the
    /// caller's job, outside this call.
    pub fn render_with(&mut self, mut overlay: Option<&mut dyn Overlay>) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("video"),
            });
        let extra = match overlay.as_deref_mut() {
            Some(o) => o.prepare(
                &self.device,
                &self.queue,
                &mut encoder,
                (self.config.width, self.config.height),
            ),
            None => Vec::new(),
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("video"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(p) = &self.video {
                if let Some(r) =
                    letterbox((self.config.width, self.config.height), (p.width, p.height))
                {
                    pass.set_viewport(
                        r.x as f32,
                        r.y as f32,
                        r.width as f32,
                        r.height as f32,
                        0.0,
                        1.0,
                    );
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &p.bind, &[]);
                    pass.draw(0..6, 0..1);
                }
            }
            // egui sets its own viewport and scissor, so the video's needs no reset.
            if let Some(o) = overlay {
                let mut pass = pass.forget_lifetime();
                o.paint(&mut pass);
            }
        }
        self.queue
            .submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        self.queue.present(frame);
        true
    }
}

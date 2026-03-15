// SPDX-License-Identifier: GPL-3.0

//! Custom wgpu shader widget for the projectM visualizer.
//!
//! Uses iced's `shader::Program` / `shader::Primitive` API to maintain a
//! **single persistent GPU texture** that is updated in-place every frame
//! via `queue.write_texture()`.  This completely bypasses iced's raster
//! image cache (and its per-frame `Id::unique()` churn), eliminating the
//! white-flash artefact that plagued the `image::Handle::from_rgba` path.

use cosmic::iced::widget::shader;
use cosmic::iced::{Rectangle, mouse};
use cosmic::iced_wgpu::wgpu;
use std::sync::{Arc, Mutex};

/// Shared frame buffer: the render subscription writes RGBA pixels here,
/// the shader widget reads them in `prepare()`.
pub struct VizFrameBuffer {
    /// Raw RGBA pixels (width * height * 4 bytes).
    pub pixels: Vec<u8>,
    /// Width of the frame in pixels.
    pub width: u32,
    /// Height of the frame in pixels.
    pub height: u32,
    /// Monotonically increasing generation counter — bumped on each new
    /// frame so `prepare()` knows when to re-upload.
    pub generation: u64,
}

impl VizFrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixels: vec![0u8; (width * height * 4) as usize],
            width,
            height,
            generation: 0,
        }
    }

    /// Replace the pixel data with a new frame.
    pub fn update(&mut self, pixels: Vec<u8>) {
        self.pixels = pixels;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// The `shader::Program` implementation that drives the visualizer widget.
///
/// It holds a shared reference to the latest frame data written by the
/// visualizer render subscription.
#[derive(Clone)]
pub struct VizProgram {
    pub frame_buf: Arc<Mutex<VizFrameBuffer>>,
}

impl VizProgram {
    pub fn new(frame_buf: Arc<Mutex<VizFrameBuffer>>) -> Self {
        Self { frame_buf }
    }
}

impl<Message> shader::Program<Message> for VizProgram {
    type State = ();
    type Primitive = VizPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        // Snapshot the current frame data under the lock
        let guard = self.frame_buf.lock().unwrap();
        VizPrimitive {
            pixels: guard.pixels.clone(),
            width: guard.width,
            height: guard.height,
            generation: guard.generation,
        }
    }
}

/// The per-frame primitive sent to the GPU pipeline.
#[derive(Debug)]
pub struct VizPrimitive {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    generation: u64,
}

/// Persistent GPU resources stored across frames in iced's `Storage`.
struct VizPipeline {
    pipeline: wgpu::RenderPipeline,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    /// Last generation that was uploaded, to skip redundant uploads.
    last_generation: u64,
}

impl VizPipeline {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        // --- Shader module (inline WGSL) ---
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viz_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        // --- Texture for the visualizer frame ---
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viz_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viz_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // --- Bind group layout + bind group ---
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viz_bgl"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viz_bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        // --- Pipeline layout + render pipeline ---
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viz_pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viz_rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            texture,
            bind_group,
            last_generation: u64::MAX, // force first upload
        }
    }

    /// Upload new pixel data to the GPU texture.
    fn upload(&mut self, queue: &wgpu::Queue, pixels: &[u8], width: u32, height: u32) {
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}

impl shader::Primitive for VizPrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut shader::Storage,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        if !storage.has::<VizPipeline>() {
            storage.store(VizPipeline::new(device, format, self.width, self.height));
        }

        let pipeline = storage.get_mut::<VizPipeline>().unwrap();

        // Only re-upload if the generation changed
        if pipeline.last_generation != self.generation && !self.pixels.is_empty() {
            pipeline.upload(queue, &self.pixels, self.width, self.height);
            pipeline.last_generation = self.generation;
        }
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let pipeline = storage.get::<VizPipeline>().unwrap();

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viz_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Don't clear — we're drawing into the iced compositor's target
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.bind_group, &[]);
        // Fullscreen triangle (3 vertices, no vertex buffer — generated in shader)
        pass.draw(0..3, 0..1);
    }
}

/// Inline WGSL shader: fullscreen triangle + texture sampling.
///
/// Uses a single oversized triangle (vertex shader trick) to cover the
/// entire viewport. The fragment shader samples the visualizer texture.
const SHADER_WGSL: &str = r#"
// Vertex output
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle via vertex index trick (no vertex buffer needed).
// Vertex 0: (-1, -1), Vertex 1: (3, -1), Vertex 2: (-1, 3)
// This single triangle covers the entire clip space [-1,1]^2.
@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    // Map clip-space to UV: x: [-1,1] -> [0,1], y: [-1,1] -> [1,0] (flip Y)
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var viz_texture: texture_2d<f32>;
@group(0) @binding(1) var viz_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(viz_texture, viz_sampler, in.uv);
}
"#;

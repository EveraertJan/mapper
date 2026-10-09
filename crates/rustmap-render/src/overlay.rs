use std::collections::HashMap;

use rustmap_geom::Vec2;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    color: [f32; 4],
}

/// Collects overlay shapes in window pixels.
#[derive(Default)]
pub struct OverlayBuilder {
    vertices: Vec<Vertex>,
}

/// Straight-alpha RGBA, 0..1.
pub type Rgba = [f32; 4];

impl OverlayBuilder {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    fn tri(&mut self, a: Vec2, b: Vec2, c: Vec2, color: Rgba) {
        for p in [a, b, c] {
            self.vertices.push(Vertex {
                pos: p.to_array(),
                color,
            });
        }
    }

    /// A line segment `width` pixels wide.
    pub fn line(&mut self, a: Vec2, b: Vec2, width: f32, color: Rgba) {
        let dir = (b - a).normalize_or_zero();
        if dir == Vec2::ZERO {
            return;
        }
        let n = dir.perp() * (width * 0.5);
        self.tri(a - n, b - n, b + n, color);
        self.tri(a - n, b + n, a + n, color);
    }

    /// Closed outline through `points`.
    pub fn polygon(&mut self, points: &[Vec2], width: f32, color: Rgba) {
        for i in 0..points.len() {
            self.line(points[i], points[(i + 1) % points.len()], width, color);
        }
    }

    /// Axis-aligned filled square centered on `c`.
    pub fn square(&mut self, c: Vec2, half: f32, color: Rgba) {
        let (a, b) = (c - Vec2::splat(half), c + Vec2::splat(half));
        self.tri(a, Vec2::new(b.x, a.y), b, color);
        self.tri(a, b, Vec2::new(a.x, b.y), color);
    }

    /// Axis-aligned square outline centered on `c`.
    pub fn square_outline(&mut self, c: Vec2, half: f32, width: f32, color: Rgba) {
        let h = Vec2::splat(half);
        let pts = [
            c - h,
            c + Vec2::new(half, -half),
            c + h,
            c + Vec2::new(-half, half),
        ];
        self.polygon(&pts, width, color);
    }
}

/// Draws an [`OverlayBuilder`]'s shapes onto a window frame.
pub struct OverlayRenderer {
    shader: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl OverlayRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/overlay.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            shader,
            layout,
            pipeline_layout,
            pipelines: HashMap::new(),
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("overlay"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Draws on top of `view` (which must already hold the frame).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        shapes: &OverlayBuilder,
    ) {
        if shapes.is_empty() {
            return;
        }
        puffin::profile_function!();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay vertices"),
            contents: bytemuck::cast_slice(&shapes.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let screen = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay screen"),
            contents: bytemuck::cast_slice(&[size[0] as f32, size[1] as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });
        let pipeline = self.pipeline(device, format);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("overlay"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..shapes.vertices.len() as u32, 0..1);
    }
}

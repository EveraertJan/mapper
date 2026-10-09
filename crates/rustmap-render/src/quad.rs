use rustmap_core::{BlendMode, ColorAdjust, MASK_MAX_POINTS, Mask, Shape};
use rustmap_geom::{MeshGrid, Quad, corner_weights, square_to_quad};
use wgpu::util::DeviceExt;

use crate::image::GpuImage;
use crate::target::{OUTPUT_FORMAT, OutputTarget};

/// Subdivisions per mesh cell when tessellating a warp.
const MESH_SUBDIVISIONS: usize = 12;

/// Per-surface uniform block; layout must match `Surface` in `quad.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    opacity: f32,
    fill: u32,
    shape: u32,
    mask_count: u32,
    mask_feather: f32,
    mask_invert: u32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    hue: f32,
    _pad: [f32; 2],
    gain: [f32; 4],
    input: [[f32; 4]; 3],
    /// Two mask points per vec4.
    mask: [[f32; 4]; MASK_MAX_POINTS / 2],
}

const _: () = assert!(std::mem::size_of::<Uniforms>() == 112 + 16 * MASK_MAX_POINTS / 2);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    /// Clip-space position (w ≠ 1 for perspective quads).
    pos: [f32; 4],
    /// Surface texture coordinate.
    uv: [f32; 2],
}

/// An image bound for drawing on quads (texture + sampler bind group).
pub struct ImageBinding {
    bind_group: wgpu::BindGroup,
}

/// What a quad shows.
#[derive(Clone, Copy)]
pub enum QuadFill<'a> {
    Image(&'a ImageBinding),
    /// Tinted checker with border and center cross: the placeholder for
    /// surfaces without media, and a good warp check.
    Checker,
    /// Texture coordinates as red/green. For tests.
    UvDebug,
}

/// Per-surface GPU state (own uniforms so many surfaces fit in one submit).
pub struct QuadState {
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// One surface to draw.
pub struct QuadDraw<'a> {
    pub state: &'a QuadState,
    /// Corners in normalized output space (TL, TR, BR, BL). Used unless `mesh` is set.
    pub corners: Quad,
    /// Mesh warp; replaces the perspective quad when present.
    pub mesh: Option<&'a MeshGrid>,
    pub opacity: f32,
    pub fill: QuadFill<'a>,
    pub blend: BlendMode,
    pub shape: Shape,
    /// Region of the media to show (media uv space, same corner order).
    pub input: Quad,
    pub mask: Option<&'a Mask>,
    pub color: ColorAdjust,
}

impl<'a> QuadDraw<'a> {
    /// A plain perspective quad with defaults for everything else.
    pub fn simple(state: &'a QuadState, corners: Quad, opacity: f32, fill: QuadFill<'a>) -> Self {
        Self {
            state,
            corners,
            mesh: None,
            opacity,
            fill,
            blend: BlendMode::Normal,
            shape: Shape::Quad,
            input: [
                rustmap_geom::Vec2::new(0.0, 0.0),
                rustmap_geom::Vec2::new(1.0, 0.0),
                rustmap_geom::Vec2::new(1.0, 1.0),
                rustmap_geom::Vec2::new(0.0, 1.0),
            ],
            mask: None,
            color: ColorAdjust::default(),
        }
    }
}

/// Draws perspective-correct or mesh-warped textured surfaces.
pub struct QuadRenderer {
    pipelines: [wgpu::RenderPipeline; 4],
    uniform_layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound for non-image fills so the bind group slot is always valid.
    blank: ImageBinding,
}

impl QuadRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/quad.wgsl"));
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad image"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&image_layout)],
            immediate_size: 0,
        });
        let make = |blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("quad"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2],
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: OUTPUT_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = BlendMode::ALL.map(|mode| make(blend_state(mode)));
        // Steep keystone angles squash the image along one axis; anisotropic
        // filtering keeps it sharp along the other.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("quad"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let blank_image = GpuImage::upload(
            device,
            queue,
            &rustmap_media::ImageData {
                width: 1,
                height: 1,
                mips: vec![vec![0, 0, 0, 0]],
                original_size: [1, 1],
            },
            "blank",
        );
        let blank = ImageBinding {
            bind_group: Self::image_bind_group(device, &image_layout, &sampler, &blank_image),
        };

        Self {
            pipelines,
            uniform_layout,
            image_layout,
            sampler,
            blank,
        }
    }

    fn image_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        image: &GpuImage,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad image"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&image.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    pub fn bind_image(&self, device: &wgpu::Device, image: &GpuImage) -> ImageBinding {
        ImageBinding {
            bind_group: Self::image_bind_group(device, &self.image_layout, &self.sampler, image),
        }
    }

    pub fn create_state(&self, device: &wgpu::Device) -> QuadState {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad uniforms"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        QuadState {
            uniforms,
            bind_group,
        }
    }

    /// Draws `quads` in order on top of what is already in `target`.
    /// Non-convex perspective quads are skipped (they have no valid mapping).
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &OutputTarget,
        quads: &[QuadDraw<'_>],
    ) {
        puffin::profile_function!();
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        // (draw, first index, index count)
        let mut draws = Vec::new();

        for quad in quads.iter().filter(|q| q.opacity > 0.0) {
            let base = vertices.len() as u32;
            let first = indices.len() as u32;
            if let Some(mesh) = quad.mesh {
                let t = mesh.tessellate(MESH_SUBDIVISIONS);
                vertices.extend(t.positions.iter().zip(&t.uvs).map(|(p, uv)| Vertex {
                    pos: [p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0],
                    uv: uv.to_array(),
                }));
                indices.extend(t.indices.iter().map(|i| base + i));
            } else {
                let Some(w) = corner_weights(&quad.corners) else {
                    continue;
                };
                // Perspective-correct: clip position = (ndc * W, 0, W).
                const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                for i in 0..4 {
                    let c = quad.corners[i];
                    vertices.push(Vertex {
                        pos: [
                            (c.x * 2.0 - 1.0) * w[i],
                            (1.0 - c.y * 2.0) * w[i],
                            0.0,
                            w[i],
                        ],
                        uv: UVS[i],
                    });
                }
                indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
            }
            draws.push((quad, first, indices.len() as u32 - first));
            queue.write_buffer(&quad.state.uniforms, 0, bytemuck::bytes_of(&uniforms(quad)));
        }
        if draws.is_empty() {
            return;
        }

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("surface vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("surface indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("surfaces"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for (quad, first, count) in draws {
            pass.set_pipeline(&self.pipelines[blend_index(quad.blend)]);
            let image = match quad.fill {
                QuadFill::Image(binding) => binding,
                _ => &self.blank,
            };
            pass.set_bind_group(0, &quad.state.bind_group, &[]);
            pass.set_bind_group(1, &image.bind_group, &[]);
            pass.draw_indexed(first..first + count, 0, 0..1);
        }
    }
}

fn uniforms(quad: &QuadDraw<'_>) -> Uniforms {
    let fill = match quad.fill {
        QuadFill::Image(_) => 0,
        QuadFill::Checker => 1,
        QuadFill::UvDebug => 2,
    };
    // Surface uv -> media uv. A degenerate input region falls back to the full frame.
    let m = if rustmap_geom::is_convex(&quad.input) {
        square_to_quad(&quad.input)
    } else {
        rustmap_geom::Mat3::IDENTITY
    };
    let mut mask = [[0.0; 4]; MASK_MAX_POINTS / 2];
    let (mask_count, mask_feather, mask_invert) = match quad.mask {
        Some(mk) if mk.points.len() >= 3 => {
            let n = mk.points.len().min(MASK_MAX_POINTS);
            for (i, p) in mk.points.iter().take(n).enumerate() {
                mask[i / 2][(i % 2) * 2] = p[0];
                mask[i / 2][(i % 2) * 2 + 1] = p[1];
            }
            (n as u32, mk.feather.max(0.0), u32::from(mk.invert))
        }
        _ => (0, 0.0, 0),
    };
    let c = quad.color;
    Uniforms {
        opacity: quad.opacity.clamp(0.0, 1.0),
        fill,
        shape: match quad.shape {
            Shape::Quad => 0,
            Shape::Triangle => 1,
            Shape::Circle => 2,
        },
        mask_count,
        mask_feather,
        mask_invert,
        brightness: c.brightness,
        contrast: c.contrast,
        saturation: c.saturation,
        hue: c.hue.to_radians(),
        _pad: [0.0; 2],
        gain: [c.gain[0], c.gain[1], c.gain[2], 1.0],
        input: [
            m.x_axis.extend(0.0).to_array(),
            m.y_axis.extend(0.0).to_array(),
            m.z_axis.extend(0.0).to_array(),
        ],
        mask,
    }
}

fn blend_index(mode: BlendMode) -> usize {
    BlendMode::ALL.iter().position(|m| *m == mode).unwrap_or(0)
}

/// Blend equations for premultiplied colors. Alpha always accumulates like
/// "normal" so the output stays opaque where something was drawn.
fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor as F, BlendOperation as Op};
    let normal_alpha = BlendComponent {
        src_factor: F::One,
        dst_factor: F::OneMinusSrcAlpha,
        operation: Op::Add,
    };
    let color = |src_factor, dst_factor| BlendComponent {
        src_factor,
        dst_factor,
        operation: Op::Add,
    };
    wgpu::BlendState {
        color: match mode {
            BlendMode::Normal => color(F::One, F::OneMinusSrcAlpha),
            BlendMode::Add => color(F::One, F::One),
            BlendMode::Screen => color(F::One, F::OneMinusSrc),
            BlendMode::Multiply => color(F::Dst, F::OneMinusSrcAlpha),
        },
        alpha: normal_alpha,
    }
}

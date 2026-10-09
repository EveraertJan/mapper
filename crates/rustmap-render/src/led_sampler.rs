//! Reads the output image at LED positions: a tiny render pass writes one
//! texel per LED, which is read back asynchronously like NDI frames.

use crate::readback::FrameReadback;
use crate::target::{OUTPUT_FORMAT, OutputTarget};

/// Texels per row of the sample texture.
const ROW: u32 = 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    width: u32,
    count: u32,
    pad: [u32; 2],
}

/// Shared pipeline.
pub struct LedSampler {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Per-output sampling state for a fixed number of LEDs.
pub struct LedSamples {
    count: usize,
    target: OutputTarget,
    readback: FrameReadback,
    points: wgpu::Buffer,
    params: wgpu::Buffer,
}

impl LedSamples {
    pub fn count(&self) -> usize {
        self.count
    }

    /// Starts mapping this frame's copy. Call after `queue.submit`.
    pub fn after_submit(&mut self) {
        self.readback.after_submit();
    }

    /// The newest finished samples (RGB per LED, in the order given to
    /// [`LedSampler::sample`]). Needs `device.poll` to have run.
    pub fn read_latest(&mut self) -> Option<Vec<[u8; 3]>> {
        let count = self.count;
        self.readback.read_latest(|frame| {
            let mut out = Vec::with_capacity(count);
            for row in frame.data.chunks(frame.stride as usize) {
                for px in row[..(frame.size[0] * 4) as usize].chunks_exact(4) {
                    if out.len() == count {
                        return out;
                    }
                    out.push([px[0], px[1], px[2]]);
                }
            }
            out
        })
    }
}

impl LedSampler {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/led_sample.wgsl"));
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("led sample"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    2,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("led sample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("led sample"),
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
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("led sample"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    fn create_samples(&self, device: &wgpu::Device, count: usize) -> LedSamples {
        let n = count.max(1) as u32;
        let size = [n.min(ROW), n.div_ceil(ROW)];
        LedSamples {
            count,
            target: OutputTarget::new(device, size, "led samples"),
            readback: FrameReadback::new(device, size),
            points: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("led points"),
                size: (count.max(1) * 16) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("led params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// Samples `source` at `points` (`[x, y, radius_x, radius_y]`, normalized)
    /// and records a copy for readback. `state` is (re)created when the
    /// number of points changes, which drops samples still in flight.
    pub fn sample(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        state: &mut Option<LedSamples>,
        source: &OutputTarget,
        points: &[[f32; 4]],
    ) {
        if points.is_empty() {
            *state = None;
            return;
        }
        if state.as_ref().is_none_or(|s| s.count != points.len()) {
            *state = Some(self.create_samples(device, points.len()));
        }
        let Some(s) = state else { return };
        queue.write_buffer(&s.points, 0, bytemuck::cast_slice(points));
        let params = Params {
            width: s.target.size()[0],
            count: points.len() as u32,
            pad: [0; 2],
        };
        queue.write_buffer(&s.params, 0, bytemuck::bytes_of(&params));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("led sample"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: s.points.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: s.params.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("led sample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s.target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        s.readback.capture(encoder, &s.target);
    }
}

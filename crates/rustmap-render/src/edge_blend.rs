use rustmap_core::EdgeBlend;

use crate::target::{OUTPUT_FORMAT, OutputTarget};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    gamma: f32,
    curve: f32,
    widths: [f32; 4],
}

/// Per-output edge-blend state (own uniforms so several outputs fit in one submit).
pub struct EdgeBlendState {
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Multiplies an output by its edge-blend ramps, in place.
pub struct EdgeBlendRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl EdgeBlendRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/edge_blend.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("edge blend"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("edge blend"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // result = dst * src: the shader outputs the brightness factor.
        let multiply = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::Src,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("edge blend"),
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
                    blend: Some(wgpu::BlendState {
                        color: multiply,
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    pub fn create_state(&self, device: &wgpu::Device) -> EdgeBlendState {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("edge blend uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("edge blend"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        EdgeBlendState {
            uniforms,
            bind_group,
        }
    }

    /// Applies `blend` to `target`. Does nothing when no side has a ramp.
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        state: &EdgeBlendState,
        target: &OutputTarget,
        blend: &EdgeBlend,
    ) {
        if !blend.is_active() {
            return;
        }
        let [w, h] = target.size();
        let clamp = |v: f32| v.clamp(0.0, 0.5);
        let uniforms = Uniforms {
            resolution: [w as f32, h as f32],
            gamma: blend.gamma,
            curve: blend.curve,
            widths: [
                clamp(blend.left),
                clamp(blend.right),
                clamp(blend.top),
                clamp(blend.bottom),
            ],
        };
        queue.write_buffer(&state.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("edge blend"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &state.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

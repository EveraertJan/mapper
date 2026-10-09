use rustmap_core::TestPattern;
use wgpu::util::DeviceExt;

use crate::target::{OUTPUT_FORMAT, OutputTarget};

/// Uniform block; layout must match `Params` in `test_pattern.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    pattern: u32,
    color: [f32; 4],
    cell: f32,
    index: f32,
    _pad: [f32; 2],
}

const _: () = assert!(std::mem::size_of::<Uniforms>() == 48);

/// What to draw this frame.
#[derive(Debug, Clone, Copy)]
pub struct PatternParams {
    pub pattern: TestPattern,
    pub time_seconds: f32,
    pub grid_cell_px: u32,
    /// Output number shown as dots on the grid.
    pub output_index: u32,
}

/// Per-output GPU state (each output needs its own uniforms within one submit).
pub struct PatternState {
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Renders projector test patterns into output targets.
pub struct TestPatternRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl TestPatternRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/test_pattern.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test pattern"),
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
            label: Some("test pattern"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test pattern"),
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
                targets: &[Some(OUTPUT_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    pub fn create_state(&self, device: &wgpu::Device) -> PatternState {
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test pattern uniforms"),
            contents: bytemuck::bytes_of(&<Uniforms as bytemuck::Zeroable>::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test pattern"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        PatternState {
            uniforms,
            bind_group,
        }
    }

    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        state: &PatternState,
        target: &OutputTarget,
        params: &PatternParams,
    ) {
        puffin::profile_function!();
        let [w, h] = target.size();
        let color = match params.pattern {
            TestPattern::Solid([r, g, b]) => [r, g, b, 1.0],
            _ => [0.0, 0.0, 0.0, 1.0],
        };
        let uniforms = Uniforms {
            resolution: [w as f32, h as f32],
            time: params.time_seconds,
            pattern: params.pattern.shader_index(),
            color,
            cell: params.grid_cell_px as f32,
            index: params.output_index as f32,
            _pad: [0.0; 2],
        };
        queue.write_buffer(&state.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test pattern"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
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
        pass.set_bind_group(0, &state.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

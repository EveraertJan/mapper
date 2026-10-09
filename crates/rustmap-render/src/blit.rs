use std::collections::HashMap;

/// Draws a texture into a window, aspect-preserving (black bars if the window
/// shape differs from the output resolution — never a silent stretch).
pub struct Blitter {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    /// Linear for scaled previews; nearest when the window matches pixel-for-pixel.
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Blitter {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/blit.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
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
            label: Some("blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = |filter, label| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            linear: sampler(wgpu::FilterMode::Linear, "blit linear"),
            nearest: sampler(wgpu::FilterMode::Nearest, "blit nearest"),
            layout,
            shader,
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
                label: Some("blit"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Clears `dst` to black and draws `src` centered and aspect-fit.
    #[allow(clippy::too_many_arguments)]
    pub fn blit(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        src_size: [u32; 2],
        dst: &wgpu::TextureView,
        dst_format: wgpu::TextureFormat,
        dst_size: [u32; 2],
    ) {
        puffin::profile_function!();
        let exact = src_size == dst_size;
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if exact {
                        &self.nearest
                    } else {
                        &self.linear
                    }),
                },
            ],
        });
        let pipeline = self.pipeline(device, dst_format);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let [x, y, w, h] = fit_rect(src_size, dst_size);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Largest rect with `src`'s aspect ratio centered in `dst`: `[x, y, w, h]`.
/// Snapped to whole pixels so a 1:1 fit stays pixel-exact.
pub fn fit_rect(src: [u32; 2], dst: [u32; 2]) -> [f32; 4] {
    let (sw, sh) = (src[0].max(1) as f32, src[1].max(1) as f32);
    let (dw, dh) = (dst[0] as f32, dst[1] as f32);
    let scale = (dw / sw).min(dh / sh);
    let (w, h) = ((sw * scale).round(), (sh * scale).round());
    let (x, y) = (((dw - w) * 0.5).floor(), ((dh - h) * 0.5).floor());
    [x, y, w, h]
}

#[cfg(test)]
mod tests {
    use super::fit_rect;

    #[test]
    fn exact_fit_is_identity() {
        assert_eq!(
            fit_rect([1920, 1080], [1920, 1080]),
            [0.0, 0.0, 1920.0, 1080.0]
        );
    }

    #[test]
    fn letterbox_and_pillarbox() {
        // 16:9 into 4:3 → bars top and bottom.
        assert_eq!(
            fit_rect([1920, 1080], [1024, 768]),
            [0.0, 96.0, 1024.0, 576.0]
        );
        // 4:3 into 16:9 → bars left and right.
        assert_eq!(
            fit_rect([1024, 768], [1920, 1080]),
            [240.0, 0.0, 1440.0, 1080.0]
        );
    }
}

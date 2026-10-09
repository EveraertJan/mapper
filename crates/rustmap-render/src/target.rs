/// Format of every output target.
///
/// Gamma-encoded 8-bit RGBA: it is what projectors receive anyway, and it is
/// the format egui requires for showing native textures as previews.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Offscreen texture holding one output's final image at projector resolution.
pub struct OutputTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    size: [u32; 2],
}

impl OutputTarget {
    pub fn new(device: &wgpu::Device, size: [u32; 2], label: &str) -> Self {
        let max = device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(1, max), size[1].clamp(1, max)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
        }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Copies the target back to the CPU as tightly packed RGBA8 rows.
    ///
    /// Blocking; meant for tests and tooling, not the render loop (LED
    /// mapping will get an async version).
    pub fn read_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let [w, h] = self.size;
        let unpadded = w * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            self.texture.size(),
        );
        queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("wait for readback");
        let data = slice.get_mapped_range().expect("read mapped buffer");
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in data.chunks(padded as usize) {
            out.extend_from_slice(&row[..unpadded as usize]);
        }
        out
    }
}

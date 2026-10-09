use rustmap_media::ImageData;

use crate::target::OUTPUT_FORMAT;

/// An image on the GPU with its full mip chain.
///
/// Same format as output targets (gamma-encoded RGBA8), so it can also be
/// shown directly as a thumbnail in the UI.
pub struct GpuImage {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: [u32; 2],
}

impl GpuImage {
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &ImageData,
        label: &str,
    ) -> Self {
        puffin::profile_function!();
        let size = wgpu::Extent3d {
            width: data.width,
            height: data.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: data.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, pixels) in data.mips.iter().enumerate() {
            let level_size = size.mip_level_size(level as u32, wgpu::TextureDimension::D2);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(level_size.width * 4),
                    rows_per_image: Some(level_size.height),
                },
                level_size,
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size: [data.width, data.height],
        }
    }

    /// An empty single-level texture for live sources (NDI, later video),
    /// refilled every frame with [`GpuImage::write`].
    pub fn live(device: &wgpu::Device, size: [u32; 2], label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size: [size[0].max(1), size[1].max(1)],
        }
    }

    /// Replaces the pixels of a [`GpuImage::live`] texture. `data` is RGBA8
    /// with `stride` bytes per row and must match the texture size.
    pub fn write(&self, queue: &wgpu::Queue, data: &[u8], stride: u32) {
        puffin::profile_function!();
        let [w, h] = self.size;
        debug_assert!(stride >= w * 4 && data.len() >= (stride * (h - 1) + w * 4) as usize);
        queue.write_texture(
            self.texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn aspect(&self) -> f32 {
        self.size[0] as f32 / self.size[1].max(1) as f32
    }
}

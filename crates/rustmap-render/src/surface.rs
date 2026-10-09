use crate::gpu::Gpu;

/// A window's swapchain.
///
/// Always rendered through a non-sRGB view: everything in RUSTMAP is already
/// gamma-encoded (test patterns, video, egui), so the hardware must not
/// apply a second sRGB encode.
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    needs_reconfigure: bool,
}

/// A frame acquired from a window, ready to draw into and present.
pub struct SurfaceFrame {
    pub texture: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub size: [u32; 2],
}

impl WindowSurface {
    pub fn new(gpu: &Gpu, surface: wgpu::Surface<'static>, size: [u32; 2], vsync: bool) -> Self {
        let caps = surface.get_capabilities(&gpu.adapter);
        // Prefer a plain 8-bit format; fall back to whatever comes first.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .unwrap_or(caps.formats[0]);
        let view_format = format.remove_srgb_suffix();
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size[0].max(1),
            height: size[1].max(1),
            present_mode: if vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            },
            desired_maximum_frame_latency: 2,
            alpha_mode,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: if view_format == format {
                vec![]
            } else {
                vec![view_format]
            },
        };
        surface.configure(&gpu.device, &config);
        Self {
            surface,
            config,
            view_format,
            needs_reconfigure: false,
        }
    }

    pub fn resize(&mut self, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if size != [self.config.width, self.config.height] {
            self.config.width = size[0];
            self.config.height = size[1];
            self.needs_reconfigure = true;
        }
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        let mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        if mode != self.config.present_mode {
            self.config.present_mode = mode;
            self.needs_reconfigure = true;
        }
    }

    /// Format to build pipelines for when drawing into this surface.
    pub fn view_format(&self) -> wgpu::TextureFormat {
        self.view_format
    }

    pub fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    /// Gets the next frame, or `None` if this window should be skipped this
    /// frame (minimized, occluded, or being reconfigured).
    pub fn acquire(&mut self, gpu: &Gpu) -> Option<SurfaceFrame> {
        puffin::profile_function!();
        if self.needs_reconfigure {
            self.surface.configure(&gpu.device, &self.config);
            self.needs_reconfigure = false;
        }
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.needs_reconfigure = true;
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.needs_reconfigure = true;
                return None;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return None;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::warn!("surface validation error while acquiring frame");
                return None;
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        Some(SurfaceFrame {
            texture,
            view,
            format: self.view_format,
            size: self.size(),
        })
    }
}

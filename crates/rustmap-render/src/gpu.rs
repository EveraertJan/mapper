/// Errors while bringing up the GPU.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("no suitable GPU adapter found: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("could not create GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
}

/// The one GPU device shared by the control window and every output window.
///
/// Sharing a single device means output textures can be previewed in the UI
/// and presented to projectors without any copies between devices.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Picks the high-performance adapter (discrete GPU on dual-GPU Macs).
    ///
    /// `compatible_surface` should be any window surface we will present to,
    /// so the adapter is guaranteed to be able to drive it.
    pub fn new(
        instance: wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, GpuError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface,
            ..Default::default()
        }))?;

        let info = adapter.get_info();
        tracing::info!(
            name = %info.name,
            backend = ?info.backend,
            device_type = ?info.device_type,
            "using GPU adapter"
        );

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rustmap device"),
                required_features: wgpu::Features::empty(),
                // Use what the adapter can do; projectors at 4K and video textures need headroom.
                required_limits: adapter.limits(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))?;

        device.on_uncaptured_error(std::sync::Arc::new(|err| {
            // The Display impl is just "Validation Error"; the detail is in the source chain.
            let mut msg = err.to_string();
            let mut source = std::error::Error::source(&err);
            while let Some(s) = source {
                msg.push_str(&format!(": {s}"));
                source = s.source();
            }
            tracing::error!("wgpu error: {msg}");
        }));

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Device without any window, for tests and offline rendering.
    pub fn headless() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        Self::new(instance, None)
    }

    pub fn adapter_summary(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }
}

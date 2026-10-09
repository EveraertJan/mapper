use std::sync::Arc;

use rustmap_core::OutputConfig;
use rustmap_render::{Gpu, WindowSurface};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::monitors;

/// Where the window should be and whether it is fullscreen.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub monitor: Option<String>,
    pub fullscreen: bool,
}

impl Placement {
    pub fn of(config: &OutputConfig) -> Self {
        Self {
            monitor: config.monitor.clone(),
            fullscreen: config.fullscreen,
        }
    }
}

/// A projector window. It only ever shows its output's target texture.
pub struct OutputWindow {
    pub window: Arc<Window>,
    pub surface: WindowSurface,
    /// Placement last applied; `None` forces a (re)apply.
    applied: Option<Placement>,
}

impl OutputWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        gpu: &Gpu,
        config: &OutputConfig,
        vsync: bool,
    ) -> anyhow::Result<Self> {
        let attrs = Window::default_attributes()
            .with_title(format!("RUSTMAP — {}", config.name))
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(event_loop.create_window(attrs)?);
        let surface = gpu.instance.create_surface(window.clone())?;
        let size = window.inner_size();
        let surface = WindowSurface::new(gpu, surface, [size.width, size.height], vsync);
        Ok(Self {
            window,
            surface,
            applied: None,
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Moves the window to its monitor and enters/leaves fullscreen, if the
    /// desired placement differs from what was last applied.
    pub fn sync_placement(&mut self, event_loop: &ActiveEventLoop, config: &OutputConfig) {
        let desired = Placement::of(config);
        if self.applied.as_ref() == Some(&desired) {
            return;
        }
        let monitor = match &desired.monitor {
            Some(key) => monitors::find(event_loop, key),
            None => self.window.current_monitor(),
        };
        if desired.monitor.is_some() && monitor.is_none() {
            tracing::warn!(output = %config.id, monitor = ?desired.monitor, "monitor not connected; leaving window where it is");
        }

        set_fullscreen(&self.window, None);
        if let Some(m) = &monitor {
            let p = m.position();
            // Inset a little so the OS considers the window to be on that monitor.
            self.window
                .set_outer_position(PhysicalPosition::new(p.x + 40, p.y + 40));
        }
        if desired.fullscreen {
            set_fullscreen(&self.window, monitor);
        }
        // No cursor over a fullscreen projector image.
        self.window.set_cursor_visible(!desired.fullscreen);
        self.applied = Some(desired);
    }

    pub fn set_title(&self, config: &OutputConfig) {
        self.window.set_title(&format!("RUSTMAP — {}", config.name));
    }
}

/// On macOS use "simple" fullscreen: no Spaces animation, no separate desktop,
/// and it stays put on the projector, which is what a show needs.
#[cfg(target_os = "macos")]
fn set_fullscreen(window: &Window, monitor: Option<winit::monitor::MonitorHandle>) {
    use winit::platform::macos::WindowExtMacOS;
    // The caller already moved the window onto `monitor`; simple fullscreen
    // fills whichever screen the window is on.
    window.set_simple_fullscreen(monitor.is_some());
}

#[cfg(not(target_os = "macos"))]
fn set_fullscreen(window: &Window, monitor: Option<winit::monitor::MonitorHandle>) {
    use winit::window::Fullscreen;
    match monitor {
        Some(m) => window.set_fullscreen(Some(Fullscreen::Borderless(Some(m)))),
        None => window.set_fullscreen(None),
    }
}

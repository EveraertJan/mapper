use std::sync::Arc;

use rustmap_render::{Gpu, WindowSurface, wgpu};
use winit::window::Window;

/// The operator's window: egui UI with previews of every output.
pub struct ControlWindow {
    pub window: Arc<Window>,
    pub surface: WindowSurface,
    pub egui_ctx: egui::Context,
    pub egui_state: egui_winit::State,
    pub renderer: egui_wgpu::Renderer,
}

/// What the UI produced this frame, ready to be drawn by [`ControlWindow::paint`].
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

impl ControlWindow {
    pub fn new(
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        gpu: &Gpu,
        vsync: bool,
    ) -> Self {
        let size = window.inner_size();
        let surface = WindowSurface::new(gpu, surface, [size.width, size.height], vsync);

        let egui_ctx = egui::Context::default();
        rustmap_ui::apply_theme(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            surface.view_format(),
            egui_wgpu::RendererOptions::default(),
        );

        Self {
            window,
            surface,
            egui_ctx,
            egui_state,
            renderer,
        }
    }

    /// Runs the UI closure and tessellates the result.
    pub fn run_ui(&mut self, mut f: impl FnMut(&mut egui::Ui)) -> UiFrame {
        puffin::profile_function!();
        let input = self.egui_state.take_egui_input(&self.window);
        let output = self.egui_ctx.run_ui(input, |ui| f(ui));
        self.egui_state
            .handle_platform_output(&self.window, output.platform_output);
        let primitives = self
            .egui_ctx
            .tessellate(output.shapes, output.pixels_per_point);
        UiFrame {
            primitives,
            textures_delta: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
        }
    }

    /// Uploads new/changed egui textures. Must run every frame, even when
    /// the window is not drawn, or font/image updates would be lost.
    pub fn upload_textures(&mut self, gpu: &Gpu, ui: &UiFrame) {
        for (id, deltas) in &ui.textures_delta.set {
            for delta in deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
    }

    /// Frees textures egui no longer needs. Call after the frame was submitted.
    pub fn free_textures(&mut self, ui: &mut UiFrame) {
        for id in &ui.textures_delta.free {
            self.renderer.free_texture(id);
        }
        ui.textures_delta.clear();
    }

    /// Renders the UI into `frame`. Returns extra command buffers produced by
    /// egui callbacks, which must be submitted before `encoder`.
    pub fn paint(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        frame: &rustmap_render::SurfaceFrame,
        ui: &UiFrame,
    ) -> Vec<wgpu::CommandBuffer> {
        puffin::profile_function!();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: frame.size,
            pixels_per_point: ui.pixels_per_point,
        };
        let extra =
            self.renderer
                .update_buffers(&gpu.device, &gpu.queue, encoder, &ui.primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &ui.primitives, &screen);
        }
        extra
    }
}

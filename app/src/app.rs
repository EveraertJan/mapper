use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustmap_core::{OutputConfig, OutputId, Project, SurfaceId};
use rustmap_geom::MeshGrid;
use rustmap_render::{
    Blitter, EdgeBlendRenderer, EdgeBlendState, FrameReadback, Gpu, LedSampler, LedSamples,
    OutputTarget, OverlayRenderer, PatternParams, PatternState, QuadDraw, QuadFill, QuadRenderer,
    QuadState, TestPatternRenderer, wgpu,
};
use rustmap_ui::{FrameStats, MonitorInfo, NdiOutStatus, NdiView, UiAction, UiState, ViewData};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::control::ControlWindow;
use crate::document::{Document, Unsaved};
use crate::guides;
use crate::led::{LedOutput, SampleLayout};
use crate::media::MediaManager;
use crate::monitors;
use crate::output_window::OutputWindow;

const PROFILER_ADDR: &str = "127.0.0.1:8585";
const MONITOR_SCAN_INTERVAL: Duration = Duration::from_secs(1);

pub struct Options {
    pub profiler: bool,
    /// Images to import at startup (each gets a quad on the first output).
    pub images: Vec<PathBuf>,
}

/// Top-level application state, driven by winit.
pub struct App {
    options: Options,
    project: Project,
    ui_state: UiState,
    stats: FrameStats,
    vsync: bool,
    profiler: Option<puffin_http::Server>,
    monitors: Vec<MonitorInfo>,
    last_monitor_scan: Option<Instant>,
    start: Instant,
    last_frame: Option<Instant>,
    gfx: Option<Graphics>,
    fatal: Option<anyhow::Error>,
    ndi: Option<rustmap_ndi::Ndi>,
    ndi_discovery: Option<rustmap_ndi::Discovery>,
    ndi_view: NdiView,
    doc: Document,
    /// Presentation mode (Cmd+T): every output fullscreen, guides hidden.
    /// Not part of the show, so it never ends up in undo or the file.
    presenting: bool,
    title: String,
    /// Seconds since the previous frame (drives video clocks).
    frame_dt: f64,
    led: LedOutput,
    osc: crate::osc::OscInput,
}

/// Everything that needs the GPU; exists once the event loop has started.
struct Graphics {
    gpu: Gpu,
    patterns: TestPatternRenderer,
    blitter: Blitter,
    quads: QuadRenderer,
    edge_blend: EdgeBlendRenderer,
    led_sampler: LedSampler,
    overlay: OverlayRenderer,
    media: MediaManager,
    quad_states: HashMap<SurfaceId, QuadState>,
    shaders: crate::shaders::ShaderCache,
    control: ControlWindow,
    outputs: BTreeMap<OutputId, OutputRuntime>,
    previews: HashMap<OutputId, egui::TextureId>,
}

/// GPU-side state of one output. The target and preview exist even while the
/// projector window is closed, so the operator can prepare outputs offline.
struct OutputRuntime {
    requested_size: [u32; 2],
    target: OutputTarget,
    pattern: PatternState,
    blend: EdgeBlendState,
    /// LED samples taken from this output, and whose they are.
    led: Option<LedSamples>,
    led_layout: SampleLayout,
    preview: egui::TextureId,
    window: Option<OutputWindow>,
    title: String,
    ndi_out: Option<NdiOut>,
}

/// An output published over NDI.
struct NdiOut {
    sender: rustmap_ndi::Sender,
    readback: FrameReadback,
}

impl App {
    pub fn new(options: Options) -> Self {
        let mut project = Project::default();
        project.add_output();
        let doc = Document::new(&project);
        Self {
            options,
            project,
            ui_state: UiState::default(),
            stats: FrameStats::default(),
            vsync: true,
            profiler: None,
            monitors: Vec::new(),
            last_monitor_scan: None,
            start: Instant::now(),
            last_frame: None,
            gfx: None,
            fatal: None,
            ndi: None,
            ndi_discovery: None,
            ndi_view: NdiView::default(),
            doc,
            presenting: false,
            title: String::new(),
            frame_dt: 0.0,
            led: LedOutput::default(),
            osc: crate::osc::OscInput::default(),
        }
    }

    pub fn into_result(self) -> anyhow::Result<()> {
        match self.fatal {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn init_graphics(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<Graphics> {
        let attrs = Window::default_attributes()
            .with_title("RUSTMAP")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs)?);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone())?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        let control = ControlWindow::new(window, surface, &gpu, self.vsync);

        Ok(Graphics {
            patterns: TestPatternRenderer::new(&gpu.device),
            blitter: Blitter::new(&gpu.device),
            quads: QuadRenderer::new(&gpu.device, &gpu.queue),
            edge_blend: EdgeBlendRenderer::new(&gpu.device),
            led_sampler: LedSampler::new(&gpu.device),
            overlay: OverlayRenderer::new(&gpu.device),
            media: MediaManager::default(),
            quad_states: HashMap::new(),
            shaders: crate::shaders::ShaderCache::default(),
            gpu,
            control,
            outputs: BTreeMap::new(),
            previews: HashMap::new(),
        })
    }

    fn set_profiler(&mut self, on: bool) {
        if on && self.profiler.is_none() {
            match puffin_http::Server::new(PROFILER_ADDR) {
                Ok(server) => {
                    tracing::info!(
                        "profiler streaming on {PROFILER_ADDR} — open it with `puffin_viewer`"
                    );
                    self.profiler = Some(server);
                }
                Err(err) => tracing::error!("could not start profiler: {err}"),
            }
        } else if !on {
            self.profiler = None;
        }
        puffin::set_scopes_on(self.profiler.is_some());
    }

    fn set_vsync(&mut self, vsync: bool) {
        self.vsync = vsync;
        if let Some(gfx) = &mut self.gfx {
            gfx.control.surface.set_vsync(vsync);
            for window in gfx.outputs.values_mut().filter_map(|o| o.window.as_mut()) {
                window.surface.set_vsync(vsync);
            }
        }
    }

    /// Adds images to the project. The first goes onto `assign_to`, or, with
    /// no target, every image gets a new quad on the selected output.
    fn import_images(&mut self, paths: Vec<PathBuf>, assign_to: Option<SurfaceId>) {
        for (i, path) in paths.into_iter().enumerate() {
            let media = if rustmap_media::is_image_path(&path) {
                self.project.add_image(path)
            } else if rustmap_media::is_video_path(&path) {
                self.project.add_video(path)
            } else {
                tracing::warn!("not an image or video we can load: {}", path.display());
                continue;
            };
            let target = match assign_to {
                Some(id) if i == 0 => Some(id),
                Some(_) => None,
                None => self
                    .ui_state
                    .selected
                    .or_else(|| self.project.outputs.first().map(|o| o.id))
                    .and_then(|o| self.project.add_surface(o)),
            };
            if let Some(surface) = target.and_then(|id| self.project.surface_mut(id)) {
                surface.media = Some(media);
                let id = surface.id;
                self.ui_state.select_surface(Some(id));
            }
        }
    }

    fn pick_images(&mut self, assign_to: Option<SurfaceId>) {
        let picked = rfd::FileDialog::new()
            .set_title("Import images or videos")
            .add_filter(
                "Images and videos",
                &[
                    rustmap_media::IMAGE_EXTENSIONS,
                    rustmap_media::VIDEO_EXTENSIONS,
                ]
                .concat(),
            )
            .add_filter("Images", rustmap_media::IMAGE_EXTENSIONS)
            .add_filter("Videos", rustmap_media::VIDEO_EXTENSIONS)
            .pick_files();
        if let Some(paths) = picked {
            self.import_images(paths, assign_to);
        }
    }

    /// Imports DMX fixture types from `.rmfixture` files.
    fn import_dmx_types(&mut self) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Import fixture types")
            .set_directory(fixture_library())
            .add_filter(
                "RUSTMAP fixture type",
                &[rustmap_core::file::FIXTURE_EXTENSION],
            )
            .pick_files()
        else {
            return;
        };
        let mut errors = Vec::new();
        for path in paths {
            match rustmap_core::file::load_dmx_type(&path) {
                Ok(ty) => {
                    let id = self.project.add_dmx_type(ty);
                    tracing::info!(?id, path = %path.display(), "imported fixture type");
                }
                Err(err) => errors.push(err.to_string()),
            }
        }
        if !errors.is_empty() {
            show_error(
                "Some fixture types could not be imported",
                &errors.join("\n"),
            );
        }
    }

    /// Saves a DMX fixture type to a file (defaults to the user's fixture library folder).
    /// Lets the user choose a WGSL shader filter for a surface.
    fn pick_shader(&mut self, id: SurfaceId) {
        let current = self
            .project
            .surface(id)
            .and_then(|s| s.shader.as_ref())
            .and_then(|f| f.path.parent().map(Path::to_path_buf));
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose a shader filter")
            .add_filter("WGSL shader", &["wgsl"]);
        if let Some(dir) = current.or_else(crate::shaders::examples_dir) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };
        if let Some(s) = self.project.surface_mut(id) {
            match &mut s.shader {
                Some(f) => f.path = path,
                None => s.shader = Some(rustmap_core::ShaderFilter::new(path)),
            }
        }
    }

    fn export_dmx_type(&mut self, id: rustmap_core::DmxTypeId) {
        let Some(ty) = self.project.dmx_type(id) else {
            return;
        };
        let name: String = ty
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = fixture_library();
        let _ = std::fs::create_dir_all(&dir);
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export fixture type")
            .set_directory(dir)
            .set_file_name(format!("{name}.{}", rustmap_core::file::FIXTURE_EXTENSION))
            .add_filter(
                "RUSTMAP fixture type",
                &[rustmap_core::file::FIXTURE_EXTENSION],
            )
            .save_file()
        else {
            return;
        };
        if let Err(err) = rustmap_core::file::save_dmx_type(ty, &path) {
            show_error("Could not export the fixture type", &err.to_string());
        }
    }

    /// Loads the NDI runtime once and starts looking for sources.
    fn init_ndi(&mut self) {
        match rustmap_ndi::Ndi::load() {
            Ok(ndi) => {
                self.ndi_view.runtime = Ok(format!("NDI {}", ndi.version()));
                self.ndi_discovery = Some(ndi.discover());
                self.ndi = Some(ndi);
            }
            Err(err) => {
                tracing::info!("NDI unavailable: {err}");
                self.ndi_view.runtime = Err(err.to_string());
            }
        }
    }

    fn update_ndi_view(&mut self) {
        if let Some(d) = &self.ndi_discovery {
            self.ndi_view.sources = d.sources();
        }
        self.ndi_view.outputs.clear();
        if let Some(gfx) = &self.gfx {
            for (id, rt) in &gfx.outputs {
                if let Some(out) = &rt.ndi_out {
                    self.ndi_view.outputs.insert(
                        *id,
                        NdiOutStatus {
                            name: out.sender.name().to_owned(),
                            sent: out.sender.frames_sent(),
                            dropped: out.sender.frames_dropped(),
                        },
                    );
                }
            }
        }
    }

    /// Starts mapping this frame's readbacks and sends any finished ones.
    fn send_ndi_frames(&mut self) {
        let Some(gfx) = &mut self.gfx else { return };
        let mut any = false;
        for rt in gfx.outputs.values_mut() {
            if let Some(out) = &mut rt.ndi_out {
                out.readback.after_submit();
                any = true;
            }
        }
        if !any {
            return;
        }
        puffin::profile_scope!("ndi send");
        // Non-blocking: just lets finished map callbacks run.
        let _ = gfx.gpu.device.poll(wgpu::PollType::Poll);
        let fps = match self.stats.average_ms() {
            ms if ms > 0.0 => 1000.0 / ms,
            _ => 60.0,
        };
        for rt in gfx.outputs.values_mut() {
            let Some(out) = &mut rt.ndi_out else { continue };
            let sender = &out.sender;
            out.readback.read_latest(|frame| {
                let mut data = sender.buffer();
                data.extend_from_slice(frame.data);
                sender.send(rustmap_ndi::VideoFrame {
                    width: frame.size[0],
                    height: frame.size[1],
                    stride: frame.stride,
                    data,
                    frame_rate: fps.round(),
                })
            });
        }
    }

    /// Picks up finished LED samples and sends the universes when due.
    fn send_leds(&mut self) {
        let Some(gfx) = &mut self.gfx else { return };
        let mut any = false;
        for rt in gfx.outputs.values_mut() {
            if let Some(led) = &mut rt.led {
                led.after_submit();
                any = true;
            }
        }
        if any {
            let _ = gfx.gpu.device.poll(wgpu::PollType::Poll);
            for rt in gfx.outputs.values_mut() {
                if let Some(samples) = rt.led.as_mut().and_then(LedSamples::read_latest) {
                    self.led.take_samples(&rt.led_layout, &samples);
                }
            }
        }
        let now = self.start.elapsed().as_secs_f64();
        self.osc.update(&mut self.project, &mut self.led, now);
        let restarts = self.led.take_restarts();
        if let Some(gfx) = &mut self.gfx {
            for id in restarts {
                gfx.media.seek(id, 0.0);
            }
        }
        self.led.update(&mut self.project, now, self.frame_dt);
    }

    fn handle_action(&mut self, action: UiAction) {
        match action {
            UiAction::SetProfiler(on) => self.set_profiler(on),
            UiAction::SetVsync(on) => self.set_vsync(on),
            UiAction::ImportImages { assign_to } => self.pick_images(assign_to),
            UiAction::ImportDmxTypes => self.import_dmx_types(),
            UiAction::ExportDmxType(id) => self.export_dmx_type(id),
            UiAction::PickShader(id) => self.pick_shader(id),
            UiAction::GoCue(id) => {
                let now = self.start.elapsed().as_secs_f64();
                self.led.go(&mut self.project, id, now);
            }
            UiAction::CurrentCue => {
                let now = self.start.elapsed().as_secs_f64();
                self.led.again(&mut self.project, now);
            }
            UiAction::StepCue(forward) => {
                let now = self.start.elapsed().as_secs_f64();
                self.led.step(&mut self.project, forward, now);
            }
            UiAction::SeekVideo(id, t) => {
                if let Some(gfx) = &mut self.gfx {
                    gfx.media.seek(id, t);
                }
            }
            UiAction::NewShow => {
                if matches!(self.doc.confirm_discard(&self.project), Unsaved::Proceed) {
                    let mut project = Project::default();
                    project.add_output();
                    self.replace_project(project, Document::new);
                }
            }
            UiAction::OpenShow => {
                if matches!(self.doc.confirm_discard(&self.project), Unsaved::Proceed)
                    && let Some(path) = crate::document::pick_open_path()
                {
                    self.open_show(&path);
                }
            }
            UiAction::SaveShow => {
                self.doc.save(&self.project, false);
            }
            UiAction::SaveShowAs => {
                self.doc.save(&self.project, true);
            }
            UiAction::SaveProduction => self.save_production(),
            UiAction::RecoverAutosave => {
                if let Some(auto) = self.doc.newer_autosave() {
                    match rustmap_core::file::load(&auto) {
                        Ok(project) => {
                            // Keep the show's path; the recovered content counts as unsaved.
                            let path = self.doc.path.clone();
                            self.replace_project(project, |p| {
                                let mut d = Document::new(&Project::default());
                                d.history = rustmap_core::History::new(p);
                                d.path = path;
                                d
                            });
                        }
                        Err(err) => crate::document::error_dialog(
                            "Could not recover the autosave",
                            &err.to_string(),
                        ),
                    }
                }
            }
            UiAction::TogglePresentation => {
                self.presenting = !self.presenting;
                tracing::info!(presenting = self.presenting, "presentation mode");
            }
            UiAction::Undo => {
                self.led.stop_fade();
                self.doc.history.undo(&mut self.project);
            }
            UiAction::Redo => {
                self.led.stop_fade();
                self.doc.history.redo(&mut self.project);
            }
            UiAction::RelinkMedia(id) => {
                let picked = rfd::FileDialog::new()
                    .set_title("Relink media")
                    .add_filter(
                        "Images and videos",
                        &[
                            rustmap_media::IMAGE_EXTENSIONS,
                            rustmap_media::VIDEO_EXTENSIONS,
                        ]
                        .concat(),
                    )
                    .pick_file();
                if let Some(path) = picked {
                    let n = crate::document::relink(&mut self.project, id, &path);
                    tracing::info!("relinked {n} media file(s)");
                }
            }
        }
    }

    fn open_show(&mut self, path: &std::path::Path) {
        match rustmap_core::file::load(path) {
            Ok(project) => {
                let production = project.production;
                let path = path.to_owned();
                self.replace_project(project, |p| Document::opened(p, path));
                // Production files go straight to the projectors.
                self.presenting = production;
                tracing::info!(production, "opened show");
            }
            Err(err) => crate::document::error_dialog("Could not open the show", &err.to_string()),
        }
    }

    /// Saves a production copy next to the working file (`Facade.rustmap` ->
    /// `Facade.production.rustmap`), or wherever the person picks.
    fn save_production(&mut self) {
        let suggested = self.doc.path.as_ref().map(|p| {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            p.with_file_name(format!(
                "{stem}.production.{}",
                rustmap_core::file::EXTENSION
            ))
        });
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save production file")
            .add_filter("RUSTMAP show", &[rustmap_core::file::EXTENSION])
            .set_file_name(suggested.as_ref().and_then(|p| p.file_name()).map_or_else(
                || "Show.production.rustmap".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            ));
        if let Some(dir) = suggested.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let mut copy = self.project.clone();
        copy.production = true;
        match rustmap_core::file::save(&copy, &path) {
            Ok(()) => tracing::info!("saved production file {}", path.display()),
            Err(err) => crate::document::error_dialog(
                "Could not save the production file",
                &err.to_string(),
            ),
        }
    }

    /// Swaps in another show and rebuilds everything that depended on the old one.
    fn replace_project(&mut self, project: Project, doc: impl FnOnce(&Project) -> Document) {
        self.doc = doc(&project);
        self.project = project;
        self.ui_state = UiState::default();
        self.presenting = false;
        self.led.reset_cues();
        if let Some(gfx) = &mut self.gfx {
            for (_, rt) in std::mem::take(&mut gfx.outputs) {
                gfx.control.renderer.free_texture(&rt.preview);
            }
            gfx.previews.clear();
            gfx.quad_states.clear();
            gfx.media.reset(&mut gfx.control.renderer);
        }
    }

    fn output_for_window(&self, id: WindowId) -> Option<OutputId> {
        let gfx = self.gfx.as_ref()?;
        gfx.outputs
            .iter()
            .find(|(_, rt)| rt.window.as_ref().is_some_and(|w| w.id() == id))
            .map(|(id, _)| *id)
    }

    /// Makes windows and GPU resources match the project.
    fn reconcile(&mut self, event_loop: &ActiveEventLoop) {
        puffin::profile_function!();
        let Some(gfx) = &mut self.gfx else { return };
        let project = &mut self.project;

        // Outputs that were removed from the project.
        gfx.outputs.retain(|id, rt| {
            let keep = project.output(*id).is_some();
            if !keep {
                gfx.control.renderer.free_texture(&rt.preview);
                gfx.previews.remove(id);
            }
            keep
        });

        let presenting = self.presenting;
        for config in &mut project.outputs {
            // While presenting, every output is open and fullscreen.
            let live = {
                let mut c = config.clone();
                if presenting {
                    c.enabled = true;
                    c.fullscreen = true;
                }
                c
            };
            let rt = gfx.outputs.entry(config.id).or_insert_with(|| {
                let target = OutputTarget::new(&gfx.gpu.device, config.resolution, &config.name);
                let preview = gfx.control.renderer.register_native_texture(
                    &gfx.gpu.device,
                    &target.view,
                    wgpu::FilterMode::Linear,
                );
                gfx.previews.insert(config.id, preview);
                OutputRuntime {
                    requested_size: config.resolution,
                    target,
                    pattern: gfx.patterns.create_state(&gfx.gpu.device),
                    blend: gfx.edge_blend.create_state(&gfx.gpu.device),
                    led: None,
                    led_layout: Vec::new(),
                    preview,
                    window: None,
                    title: String::new(),
                    ndi_out: None,
                }
            });

            if rt.requested_size != config.resolution {
                rt.requested_size = config.resolution;
                rt.target = OutputTarget::new(&gfx.gpu.device, config.resolution, &config.name);
                gfx.control.renderer.update_egui_texture_from_wgpu_texture(
                    &gfx.gpu.device,
                    &rt.target.view,
                    wgpu::FilterMode::Linear,
                    rt.preview,
                );
            }

            match (live.enabled, rt.window.is_some()) {
                (true, false) => match OutputWindow::new(event_loop, &gfx.gpu, &live, self.vsync) {
                    Ok(window) => {
                        rt.window = Some(window);
                        rt.title.clear();
                    }
                    Err(err) => {
                        tracing::error!(output = %config.id, "could not open output window: {err:#}");
                        config.enabled = false;
                    }
                },
                (false, true) => rt.window = None,
                _ => {}
            }

            // NDI out follows the toggle, the output name and its resolution.
            let want_ndi = config.ndi_send && self.ndi.is_some();
            let ndi_name = config.ndi_name();
            let stale = rt.ndi_out.as_ref().is_some_and(|o| {
                o.sender.name() != ndi_name || o.readback.size() != rt.target.size()
            });
            if !want_ndi || stale {
                rt.ndi_out = None;
            }
            if want_ndi
                && rt.ndi_out.is_none()
                && let Some(ndi) = &self.ndi
            {
                match ndi.send(&ndi_name) {
                    Ok(sender) => {
                        tracing::info!(output = %config.id, name = %ndi_name, "publishing over NDI");
                        rt.ndi_out = Some(NdiOut {
                            sender,
                            readback: FrameReadback::new(&gfx.gpu.device, rt.target.size()),
                        });
                    }
                    Err(err) => {
                        tracing::error!(output = %config.id, "NDI out failed: {err}");
                        config.ndi_send = false;
                    }
                }
            }

            if let Some(window) = &mut rt.window {
                window.sync_placement(event_loop, &live);
                if rt.title != config.name {
                    window.set_title(config);
                    rt.title.clone_from(&config.name);
                }
            }
        }

        gfx.media.update(
            project,
            &gfx.gpu,
            &gfx.quads,
            &mut gfx.control.renderer,
            self.ndi.as_ref(),
            self.frame_dt,
        );
        gfx.quad_states
            .retain(|id, _| project.surface(*id).is_some());
        for s in &project.surfaces {
            gfx.quad_states
                .entry(s.id)
                .or_insert_with(|| gfx.quads.create_state(&gfx.gpu.device));
        }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        puffin::GlobalProfiler::lock().new_frame();
        puffin::profile_function!();

        let now = Instant::now();
        if let Some(last) = self.last_frame {
            self.stats.push((now - last).as_secs_f32() * 1000.0);
            // Clamp so a stall (dialog, window drag) doesn't make videos jump.
            self.frame_dt = (now - last).as_secs_f64().min(0.1);
        }
        self.last_frame = Some(now);

        if self
            .last_monitor_scan
            .is_none_or(|t| now - t > MONITOR_SCAN_INTERVAL)
        {
            self.monitors = monitors::list(event_loop);
            self.last_monitor_scan = Some(now);
        }

        self.update_ndi_view();

        // 1. UI: edits the project.
        let actions = {
            let Some(gfx) = &mut self.gfx else { return };
            let adapter = gfx.gpu.adapter_summary();
            let mut file_view = self.doc.view(&self.project);
            file_view.presenting = self.presenting;
            let view = ViewData {
                monitors: &self.monitors,
                previews: &gfx.previews,
                media: &gfx.media.status,
                ndi: &self.ndi_view,
                dmx: &self.led.view,
                osc: &self.osc.view,
                video: &gfx.media.video_times,
                shader_errors: &gfx.shaders.errors,
                stats: &self.stats,
                adapter: &adapter,
                profiler_enabled: self.profiler.is_some(),
                vsync: self.vsync,
                file: &file_view,
            };
            let mut actions = Vec::new();
            let project = &mut self.project;
            let ui_state = &mut self.ui_state;
            let ui_frame = gfx.control.run_ui(|ui| {
                actions = rustmap_ui::draw(ui, project, ui_state, &view);
            });
            // An edit has settled once the mouse is up and no text field is active.
            let ctx = &gfx.control.egui_ctx;
            let settled =
                !ctx.input(|i| i.pointer.any_down()) && ctx.memory(|m| m.focused().is_none());
            (actions, ui_frame, settled)
        };
        let (actions, mut ui_frame, settled) = actions;
        for action in actions {
            self.handle_action(action);
        }
        // A running cue fade changes the fixtures every frame: one undo step
        // for the whole fade, taken when it ends.
        self.doc
            .history
            .observe(&self.project, settled && !self.led.fading());
        self.doc.autosave(&self.project);
        let title = self.doc.window_title(&self.project);
        if title != self.title
            && let Some(gfx) = &self.gfx
        {
            gfx.control.window.set_title(&title);
            self.title = title;
        }

        // 2. Windows and GPU resources follow the project.
        self.reconcile(event_loop);

        // 3. Render.
        let Some(gfx) = &mut self.gfx else { return };
        let time = self.start.elapsed().as_secs_f32();
        let mut encoder = gfx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        for config in &self.project.outputs {
            let Some(rt) = gfx.outputs.get(&config.id) else {
                continue;
            };
            gfx.patterns.render(
                &gfx.gpu.queue,
                &mut encoder,
                &rt.pattern,
                &rt.target,
                &pattern_params(config, time),
            );
            let surfaces: Vec<_> = self
                .project
                .surfaces_on(config.id)
                .filter(|s| s.visible)
                .collect();
            gfx.shaders.refresh(
                &gfx.quads,
                &gfx.gpu.device,
                self.project
                    .surfaces
                    .iter()
                    .filter_map(|s| s.shader.as_ref().map(|f| f.path.as_path())),
            );
            // Built first so the draws can borrow them.
            let meshes: Vec<Option<MeshGrid>> = surfaces
                .iter()
                .map(|s| {
                    s.mesh.as_ref().and_then(|m| {
                        MeshGrid::from_arrays(m.cols as usize, m.rows as usize, &m.points, m.smooth)
                    })
                })
                .collect();
            let draws: Vec<QuadDraw<'_>> = surfaces
                .iter()
                .zip(&meshes)
                .filter_map(|(s, mesh)| {
                    let fill = match s.media.and_then(|m| gfx.media.binding(m)) {
                        Some(binding) => QuadFill::Image(binding),
                        None => QuadFill::Checker,
                    };
                    Some(QuadDraw {
                        state: gfx.quad_states.get(&s.id)?,
                        corners: rustmap_geom::quad_from_arrays(&s.corners),
                        mesh: mesh.as_ref(),
                        opacity: s.opacity,
                        fill,
                        blend: s.blend,
                        shape: s.shape,
                        input: rustmap_geom::quad_from_arrays(&s.input),
                        mask: s.mask.as_ref(),
                        color: s.color,
                        filter: s.shader.as_ref().and_then(|f| gfx.shaders.get(&f.path)),
                        params: s.shader.as_ref().map_or([0.0; 4], |f| f.params),
                        time,
                    })
                })
                .collect();
            gfx.quads.render(
                &gfx.gpu.device,
                &gfx.gpu.queue,
                &mut encoder,
                &rt.target,
                &draws,
            );
            // LEDs see the image before edge blending (that's for projectors).
            let (points, layout) = crate::led::sample_points(&self.project, config.id);
            if let Some(rt) = gfx.outputs.get_mut(&config.id) {
                gfx.led_sampler.sample(
                    &gfx.gpu.device,
                    &gfx.gpu.queue,
                    &mut encoder,
                    &mut rt.led,
                    &rt.target,
                    &points,
                );
                rt.led_layout = layout;
            }
            let Some(rt) = gfx.outputs.get(&config.id) else {
                continue;
            };
            // Edge blending applies to the finished output, after every surface.
            gfx.edge_blend.render(
                &gfx.gpu.queue,
                &mut encoder,
                &rt.blend,
                &rt.target,
                &config.edge_blend,
            );
        }

        // NDI out: copy finished output images back for sending.
        for rt in gfx.outputs.values_mut() {
            if let Some(out) = &mut rt.ndi_out {
                out.readback.capture(&mut encoder, &rt.target);
            }
        }

        let mut presents = Vec::new();
        for (output_id, rt) in gfx.outputs.iter_mut() {
            let Some(window) = &mut rt.window else {
                continue;
            };
            let Some(frame) = window.surface.acquire(&gfx.gpu) else {
                continue;
            };
            gfx.blitter.blit(
                &gfx.gpu.device,
                &mut encoder,
                &rt.target.view,
                rt.target.size(),
                &frame.view,
                frame.format,
                frame.size,
            );
            if self
                .project
                .output(*output_id)
                .is_some_and(|o| o.show_guides && !self.presenting)
            {
                let shapes = guides::build(
                    &self.project,
                    *output_id,
                    rt.target.size(),
                    frame.size,
                    &self.ui_state,
                );
                gfx.overlay.render(
                    &gfx.gpu.device,
                    &mut encoder,
                    &frame.view,
                    frame.format,
                    frame.size,
                    &shapes,
                );
            }
            presents.push((window.window.clone(), frame));
        }

        gfx.control.upload_textures(&gfx.gpu, &ui_frame);
        let mut extra = Vec::new();
        if let Some(frame) = gfx.control.surface.acquire(&gfx.gpu) {
            extra = gfx.control.paint(&gfx.gpu, &mut encoder, &frame, &ui_frame);
            presents.push((gfx.control.window.clone(), frame));
        }

        {
            puffin::profile_scope!("submit");
            gfx.gpu
                .queue
                .submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        }
        gfx.control.free_textures(&mut ui_frame);
        let presented = !presents.is_empty();
        {
            puffin::profile_scope!("present");
            for (window, frame) in presents {
                window.pre_present_notify();
                gfx.gpu.queue.present(frame.texture);
            }
        }

        self.send_ndi_frames();
        self.send_leds();

        // Everything hidden or minimized: vsync isn't pacing us, so don't spin.
        if !presented {
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}

fn pattern_params(config: &OutputConfig, time: f32) -> PatternParams {
    PatternParams {
        pattern: config.test_pattern,
        time_seconds: time,
        grid_cell_px: config.grid_cell_px,
        output_index: config.id.0,
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        match self.init_graphics(event_loop) {
            Ok(gfx) => {
                self.gfx = Some(gfx);
                self.init_ndi();
                let mut images = std::mem::take(&mut self.options.images);
                if let Some(i) = images.iter().position(|p| {
                    p.extension()
                        .is_some_and(|e| e == rustmap_core::file::EXTENSION)
                }) {
                    let show = images.remove(i);
                    self.open_show(&show);
                }
                if !images.is_empty() {
                    self.import_images(images, None);
                }
                if self.options.profiler {
                    self.set_profiler(true);
                }
            }
            Err(err) => {
                tracing::error!("startup failed: {err:#}");
                self.fatal = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(gfx) = &mut self.gfx else { return };

        if window_id == gfx.control.window.id() {
            let _ = gfx
                .control
                .egui_state
                .on_window_event(&gfx.control.window, &event);
            match event {
                WindowEvent::CloseRequested => {
                    if matches!(self.doc.confirm_discard(&self.project), Unsaved::Proceed) {
                        event_loop.exit();
                    }
                }
                WindowEvent::Destroyed => {
                    event_loop.exit();
                    // Drop GPU state now so nothing touches the dead window.
                    self.gfx = None;
                }
                WindowEvent::Resized(size) => gfx.control.surface.resize([size.width, size.height]),
                WindowEvent::DroppedFile(path) => {
                    let target = self
                        .ui_state
                        .selected_surface
                        .filter(|id| self.project.surface(*id).is_some_and(|s| s.media.is_none()));
                    self.import_images(vec![path], target);
                }
                _ => {}
            }
            return;
        }

        let Some(output_id) = self.output_for_window(window_id) else {
            return;
        };
        match event {
            // Destroyed covers windows that vanish without asking (display
            // unplugged, killed by the window manager): stop rendering to them.
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                if let Some(config) = self.project.output_mut(output_id) {
                    config.enabled = false;
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(window) = self
                    .gfx
                    .as_mut()
                    .and_then(|g| g.outputs.get_mut(&output_id))
                    .and_then(|rt| rt.window.as_mut())
                {
                    window.surface.resize([size.width, size.height]);
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let Some(config) = self.project.output_mut(output_id) else {
                    return;
                };
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Escape) => {
                        if self.presenting {
                            self.presenting = false;
                        } else {
                            config.fullscreen = false;
                        }
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("f") => {
                        config.fullscreen = !config.fullscreen
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("g") => {
                        config.show_guides = !config.show_guides
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // After exit was requested the control window may already be gone.
        if self.gfx.is_some() && !event_loop.exiting() {
            self.frame(event_loop);
        }
    }
}

/// Where exported fixture types go by default, so they're easy to find again.
fn fixture_library() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("RUSTMAP")
        .join("Fixtures")
}

fn show_error(title: &str, detail: &str) {
    tracing::error!("{title}: {detail}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(detail)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

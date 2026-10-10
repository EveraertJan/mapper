//! RUSTMAP control interface.
//!
//! The UI edits the [`Project`] directly and reports anything outside the
//! project (file dialogs, the profiler) as [`UiAction`]s. The app reconciles
//! windows and GPU resources against the project afterwards, so the UI never
//! touches windows or the GPU.

mod dmx;
mod editor;
mod fixtures;
mod input;
mod lights;
mod media;
mod outputs;
mod stats;
mod surfaces;
mod theme;
pub mod warp;
mod zoom;

use std::collections::HashMap;

use rustmap_core::{
    CueId, DmxFixtureId, DmxTypeId, FixtureId, MediaId, OutputId, Project, SurfaceId, UniverseKey,
};

pub use stats::FrameStats;
pub use theme::{SELECTION, apply_theme};

/// A display connected to the computer, as reported by the OS.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    /// Unique key: name plus position, because identical projectors share a name.
    pub key: String,
    pub name: String,
    pub size: [u32; 2],
    pub position: [i32; 2],
    pub refresh_hz: Option<f32>,
}

/// Load state of a media item, as the UI needs it.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaStatus {
    Loading,
    /// A live source that has not delivered a frame yet ("Connecting…", "No signal").
    Waiting(String),
    Ready {
        size: [u32; 2],
        thumbnail: egui::TextureId,
        /// Extra info for live sources, e.g. frame rate or "No signal".
        detail: Option<String>,
    },
    Failed(String),
}

/// Playback position of a video, for the transport controls.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VideoTime {
    pub position: f64,
    pub duration: f64,
}

/// Which handles the Output view shows for the selected surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditLayer {
    /// Corners, or mesh points when the surface has a mesh.
    #[default]
    Warp,
    /// The mask polygon's points.
    Mask,
}

impl UiState {
    /// Switches what the editor's handles move, dropping the handle selection.
    pub fn set_layer(&mut self, layer: EditLayer) {
        if self.layer != layer {
            self.layer = layer;
            self.selected_corner = None;
        }
    }
}

/// What the output mapping column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorMode {
    /// Where surfaces land on the selected output.
    #[default]
    Output,
    /// Faders for the DMX fixtures.
    Console,
    /// Live DMX values of every universe the LED fixtures use.
    Dmx,
}

/// LED output state for the UI: what goes out on every universe.
#[derive(Debug, Clone, Default)]
pub struct DmxView {
    /// Every universe the fixtures use, with its current channel values.
    pub universes: Vec<(UniverseKey, Box<[u8; 512]>)>,
    /// Network sender counters while sending.
    pub packets: u64,
    pub frames: u64,
    /// Problems to show: bad destinations, socket errors, patch overlaps.
    pub errors: Vec<String>,
    /// The cue last gone to, and how far its fade is (0..1) while one runs.
    pub current_cue: Option<CueId>,
    pub fade: Option<f32>,
}

/// OSC input state for the UI.
#[derive(Debug, Clone, Default)]
pub struct OscView {
    /// Port being listened on.
    pub listening: Option<u16>,
    /// Why listening failed (port in use, ...).
    pub error: Option<String>,
    /// Messages received since start.
    pub received: u64,
    /// Latest messages first, with what they did.
    pub recent: Vec<String>,
}

/// NDI state for the UI.
#[derive(Debug, Clone)]
pub struct NdiView {
    /// `Ok(version)` if the NDI runtime loaded, else why not.
    pub runtime: Result<String, String>,
    /// Sources currently seen on the network.
    pub sources: Vec<String>,
    /// Outputs being published.
    pub outputs: HashMap<OutputId, NdiOutStatus>,
}

impl Default for NdiView {
    fn default() -> Self {
        Self {
            runtime: Err("not loaded".into()),
            sources: Vec::new(),
            outputs: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NdiOutStatus {
    pub name: String,
    pub sent: u64,
    pub dropped: u64,
}

impl MediaStatus {
    pub fn size(&self) -> Option<[u32; 2]> {
        match self {
            MediaStatus::Ready { size, .. } => Some(*size),
            _ => None,
        }
    }
}

/// Read-only data the app hands to the UI each frame.
pub struct ViewData<'a> {
    pub monitors: &'a [MonitorInfo],
    /// egui texture showing each output's rendered image.
    pub previews: &'a HashMap<OutputId, egui::TextureId>,
    pub media: &'a HashMap<MediaId, MediaStatus>,
    pub ndi: &'a NdiView,
    pub dmx: &'a DmxView,
    pub osc: &'a OscView,
    pub video: &'a HashMap<MediaId, VideoTime>,
    pub file: &'a FileView,
    pub stats: &'a FrameStats,
    pub adapter: &'a str,
    pub profiler_enabled: bool,
    pub vsync: bool,
}

/// Show-file state for the UI.
#[derive(Debug, Clone, Default)]
pub struct FileView {
    /// File name of the open show, if it was saved.
    pub name: Option<String>,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// An autosave newer than the saved show exists.
    pub autosave_available: bool,
    /// Presentation mode: all outputs fullscreen, no guides.
    pub presenting: bool,
}

/// Requests the UI can't fulfil by editing the project.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    SetProfiler(bool),
    SetVsync(bool),
    /// Open a file dialog to import images; optionally put the first one on a surface.
    ImportImages {
        assign_to: Option<SurfaceId>,
    },
    NewShow,
    OpenShow,
    SaveShow,
    SaveShowAs,
    RecoverAutosave,
    /// Save a copy that opens straight into presentation.
    SaveProduction,
    TogglePresentation,
    Undo,
    Redo,
    /// Pick a new file for missing media.
    RelinkMedia(MediaId),
    SeekVideo(MediaId, f64),
    /// Load DMX fixture types from files into the show.
    ImportDmxTypes,
    /// Save a DMX fixture type to a file, to reuse it in other shows.
    ExportDmxType(DmxTypeId),
    /// Go to a cue (fixtures take its values, the output fades).
    GoCue(CueId),
    /// Next (true) or previous cue.
    StepCue(bool),
    /// Re-apply the current cue.
    CurrentCue,
}

/// UI-only state that is not part of the show file.
#[derive(Debug)]
pub struct UiState {
    pub selected: Option<OutputId>,
    pub selected_surface: Option<SurfaceId>,
    /// Selected LED fixture (never at the same time as a surface).
    pub selected_fixture: Option<FixtureId>,
    /// Selected DMX fixture (shown in the inspector and highlighted in the console).
    pub selected_dmx: Option<DmxFixtureId>,
    /// Media shown in the input mapping column (follows the selected surface).
    pub selected_media: Option<MediaId>,
    /// Arrow keys nudge in the input mapping column (last clicked there)
    /// rather than on the output.
    pub input_keys: bool,
    /// Surface whose media `selected_media` was last synced to.
    media_follows: Option<SurfaceId>,
    /// Zoom of the input canvas, per input.
    input_zoom: HashMap<MediaId, zoom::CanvasZoom>,
    /// Zoom of the output canvas, per output.
    output_zoom: HashMap<OutputId, zoom::CanvasZoom>,
    /// Console: cue list expanded (collapsed keeps only its buttons).
    pub cues_open: bool,
    /// Handle of the selected surface that arrow keys move: a corner (TL, TR,
    /// BR, BL), a mesh point, or a mask point, depending on [`Self::layer`].
    /// For a selected fixture: one of its handles.
    pub selected_corner: Option<usize>,
    /// What the Output view edits on the selected surface.
    pub layer: EditLayer,
    pub snapping: bool,
    pub editor_mode: EditorMode,
    drag: Option<editor::Drag>,
    /// Universe shown in the DMX monitor.
    dmx_universe: Option<UniverseKey>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected: None,
            selected_surface: None,
            selected_fixture: None,
            selected_dmx: None,
            selected_media: None,
            input_keys: false,
            media_follows: None,
            input_zoom: HashMap::new(),
            output_zoom: HashMap::new(),
            cues_open: true,
            selected_corner: None,
            layer: EditLayer::Warp,
            snapping: true,
            editor_mode: EditorMode::Output,
            drag: None,
            dmx_universe: None,
        }
    }
}

impl UiState {
    pub fn select_surface(&mut self, id: Option<SurfaceId>) {
        if self.selected_surface != id {
            self.selected_corner = None;
            self.layer = EditLayer::Warp;
        }
        self.selected_surface = id;
        if id.is_some() {
            self.selected_fixture = None;
            self.selected_dmx = None;
        }
    }

    pub fn select_fixture(&mut self, id: Option<FixtureId>) {
        if self.selected_fixture != id {
            self.selected_corner = None;
        }
        self.selected_fixture = id;
        if id.is_some() {
            self.select_surface(None);
            self.layer = EditLayer::Warp;
            self.selected_dmx = None;
        }
    }

    pub fn select_dmx(&mut self, id: Option<DmxFixtureId>) {
        self.selected_dmx = id;
        if id.is_some() {
            self.select_surface(None);
            self.select_fixture(None);
        }
    }

    /// Drops selections that point at things that no longer exist, and keeps
    /// the selected surface on the selected output.
    fn validate(&mut self, project: &Project) {
        // Selecting a surface shows its media in the input column (once,
        // so another input can still be picked to give it).
        if self.selected_surface != self.media_follows {
            self.media_follows = self.selected_surface;
            if let Some(m) = self
                .selected_surface
                .and_then(|id| project.surface(id))
                .and_then(|s| s.media)
            {
                self.selected_media = Some(m);
            }
        }
        if self
            .selected_media
            .is_some_and(|id| project.media_item(id).is_none())
        {
            self.selected_media = None;
        }
        if self.selected_media.is_none() {
            self.selected_media = project.media.first().map(|m| m.id);
        }
        if self.selected.is_some_and(|id| project.output(id).is_none()) {
            self.selected = None;
        }
        if self.selected.is_none() {
            self.selected = project.outputs.first().map(|o| o.id);
        }
        let surface_ok = self
            .selected_surface
            .and_then(|id| project.surface(id))
            .is_some_and(|s| Some(s.output) == self.selected);
        if self.selected_surface.is_some() && !surface_ok {
            self.select_surface(None);
            self.drag = None;
        }
        let fixture_ok = self
            .selected_fixture
            .and_then(|id| project.fixture(id))
            .is_some_and(|f| Some(f.output) == self.selected);
        if self
            .selected_dmx
            .is_some_and(|id| project.dmx_fixture(id).is_none())
        {
            self.selected_dmx = None;
        }
        if self.selected_fixture.is_some() && !fixture_ok {
            self.select_fixture(None);
            self.drag = None;
        }
    }
}

/// Draws the whole control window.
pub fn draw(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
) -> Vec<UiAction> {
    puffin::profile_function!();
    let mut actions = Vec::new();
    state.validate(project);
    shortcuts(ui.ctx(), &mut actions);

    egui::Panel::top("menu").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.strong("RUSTMAP");
            ui.separator();
            ui.menu_button("File", |ui| {
                for (label, shortcut, action) in [
                    ("New", cmd("N"), UiAction::NewShow),
                    ("Open…", cmd("O"), UiAction::OpenShow),
                    ("Save", cmd("S"), UiAction::SaveShow),
                    ("Save As…", cmd("Shift+S"), UiAction::SaveShowAs),
                ] {
                    if ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked() {
                        actions.push(action);
                    }
                }
                if ui
                    .button("Save Production File…")
                    .on_hover_text("A copy of this show that opens straight into presentation: every output fullscreen on its projector")
                    .clicked()
                {
                    actions.push(UiAction::SaveProduction);
                }
                ui.separator();
                if ui
                    .add_enabled(view.file.autosave_available, egui::Button::new("Recover autosave"))
                    .on_hover_text("Open the last autosave, made after the last save")
                    .clicked()
                {
                    actions.push(UiAction::RecoverAutosave);
                }
            });
            ui.menu_button("View", |ui| {
                let label = if view.file.presenting { "Stop presenting" } else { "Present" };
                if ui
                    .add(egui::Button::new(label).shortcut_text(cmd("T")))
                    .on_hover_text("All outputs fullscreen on their projectors, guides hidden. Esc in an output window stops.")
                    .clicked()
                {
                    actions.push(UiAction::TogglePresentation);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(view.file.can_undo, egui::Button::new("Undo").shortcut_text(cmd("Z")))
                    .clicked()
                {
                    actions.push(UiAction::Undo);
                }
                if ui
                    .add_enabled(view.file.can_redo, egui::Button::new("Redo").shortcut_text(cmd("Shift+Z")))
                    .clicked()
                {
                    actions.push(UiAction::Redo);
                }
            });
            ui.menu_button("Output", |ui| {
                if ui.button("Add output").clicked() {
                    state.selected = Some(project.add_output());
                }
            });
            ui.menu_button("Surface", |ui| {
                let output = state.selected;
                if ui
                    .add_enabled(output.is_some(), egui::Button::new("Add quad"))
                    .clicked()
                    && let Some(id) = output.and_then(|o| project.add_surface(o))
                {
                    state.select_surface(Some(id));
                }
            });
            ui.menu_button("Media", |ui| {
                if ui.button("Import images or videos…").clicked() {
                    actions.push(UiAction::ImportImages { assign_to: None });
                }
                ui.menu_button("NDI source", |ui| media::ndi_menu(ui, project, state, view));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if view.file.presenting
                    && ui
                        .add(egui::Button::new(egui::RichText::new("PRESENTING").color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(170, 40, 40)))
                        .on_hover_text("Click or Cmd+T to stop")
                        .clicked()
                {
                    actions.push(UiAction::TogglePresentation);
                }
                let name = view.file.name.as_deref().unwrap_or("Unsaved show");
                let dirty = if view.file.dirty { " •" } else { "" };
                ui.weak(format!("{name}{dirty}"))
                    .on_hover_text(if view.file.dirty { "Unsaved changes" } else { "Saved" });
            });
        });
    });

    egui::Panel::bottom("status").show(ui, |ui| {
        stats::status_bar(ui, view, &mut actions);
    });

    // The cue list runs along the bottom, under all four columns.
    lights::cue_panel(ui, project, state, view, &mut actions);

    // Four columns, in the order a signal travels:
    // 1 input sources · 2 input mapping · 3 outputs · 4 output mapping.
    let before = (
        state.selected_dmx,
        state.selected_surface,
        state.selected_fixture,
        state.selected,
    );
    let total = ui.available_width();
    egui::Panel::left("col_inputs")
        .resizable(true)
        .default_size(210.0)
        .size_range(160.0..=360.0)
        .show(ui, |ui| {
            column_title(ui, "1", "INPUTS", "Media: images, videos and NDI sources");
            egui::ScrollArea::vertical().show(ui, |ui| {
                media::list(ui, project, state, view, &mut actions);
            });
        });
    egui::Panel::left("col_input_mapping")
        .resizable(true)
        .default_size(((total - 210.0 - 250.0) * 0.42).max(260.0))
        .min_size(200.0)
        .show(ui, |ui| {
            column_title(
                ui,
                "2",
                "INPUT MAPPING",
                "Which part of the selected input each surface uses",
            );
            input::show(ui, project, state, view);
        });
    egui::Panel::left("col_outputs")
        .resizable(true)
        .default_size(250.0)
        .size_range(190.0..=420.0)
        .show(ui, |ui| {
            column_title(
                ui,
                "3",
                "OUTPUTS",
                "Projectors with their surfaces and LED fixtures, and DMX fixtures",
            );
            egui::ScrollArea::vertical().show(ui, |ui| {
                outputs::list(ui, project, state);
                ui.add_space(10.0);
                surfaces::list(ui, project, state);
                ui.add_space(10.0);
                fixtures::list(ui, project, state);
                ui.add_space(10.0);
                lights::list(ui, project, state, &mut actions);
                ui.add_space(14.0);
                // Settings of what is selected in this column.
                if state.selected_dmx.is_some() {
                    egui::CollapsingHeader::new(egui::RichText::new("DMX fixture").strong())
                        .default_open(true)
                        .show(ui, |ui| lights::inspector(ui, project, state, &mut actions));
                } else if state.selected_fixture.is_some() {
                    egui::CollapsingHeader::new(egui::RichText::new("LED fixture").strong())
                        .default_open(true)
                        .show(ui, |ui| fixtures::inspector(ui, project, state));
                }
                egui::CollapsingHeader::new(egui::RichText::new("Output settings").strong())
                    .default_open(state.selected_dmx.is_none() && state.selected_fixture.is_none())
                    .show(ui, |ui| outputs::inspector(ui, project, state, view));
            });
        });

    // Picking something in a column shows where it is edited in column 4.
    let after = (
        state.selected_dmx,
        state.selected_surface,
        state.selected_fixture,
        state.selected,
    );
    if after != before {
        if after.0.is_some() && after.0 != before.0 {
            if state.editor_mode == EditorMode::Output {
                state.editor_mode = EditorMode::Console;
            }
        } else if after.1 != before.1 || after.2 != before.2 || after.3 != before.3 {
            state.editor_mode = EditorMode::Output;
        }
    }

    egui::CentralPanel::default().show(ui, |ui| {
        column_title(
            ui,
            "4",
            "OUTPUT MAPPING",
            "Where everything lands on the selected output, or the DMX console",
        );
        if state.editor_mode == EditorMode::Output && state.selected_surface.is_some() {
            egui::Panel::bottom("surface_settings")
                .resizable(true)
                .default_size(230.0)
                .min_size(60.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.strong("Surface settings");
                        surfaces::inspector(ui, project, state, view, &mut actions);
                    });
                });
        }
        editor::show(ui, project, state, view)
    });

    actions
}

/// Numbered heading of one of the four columns.
fn column_title(ui: &mut egui::Ui, number: &str, title: &str, help: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(number)
                .monospace()
                .strong()
                .color(theme::SELECTION),
        );
        ui.label(egui::RichText::new(title).strong().size(13.0))
            .on_hover_text(help);
    });
    ui.add_space(2.0);
}

/// Shortcut label: ⌘ on macOS, Ctrl elsewhere.
fn cmd(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("Cmd+{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// File and edit shortcuts (⌘ on macOS, Ctrl elsewhere).
fn shortcuts(ctx: &egui::Context, actions: &mut Vec<UiAction>) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    ctx.input_mut(|i| {
        // Most specific first: Cmd+Shift+S must not also trigger Cmd+S.
        for (mods, key, action) in [
            (cmd_shift, Key::S, UiAction::SaveShowAs),
            (cmd_shift, Key::Z, UiAction::Redo),
            (cmd, Key::S, UiAction::SaveShow),
            (cmd, Key::O, UiAction::OpenShow),
            (cmd, Key::N, UiAction::NewShow),
            (cmd, Key::T, UiAction::TogglePresentation),
            (cmd, Key::Z, UiAction::Undo),
            (cmd, Key::Y, UiAction::Redo),
        ] {
            if i.consume_shortcut(&KeyboardShortcut::new(mods, key)) {
                actions.push(action);
            }
        }
    });
}

use rustmap_core::{OutputConfig, Project, TestPattern};

use crate::{UiState, ViewData};

const RESOLUTION_PRESETS: &[(&str, [u32; 2])] = &[
    ("1280 × 720", [1280, 720]),
    ("1280 × 800 (WXGA)", [1280, 800]),
    ("1920 × 1080", [1920, 1080]),
    ("1920 × 1200 (WUXGA)", [1920, 1200]),
    ("2560 × 1440", [2560, 1440]),
    ("2560 × 1600 (WQXGA)", [2560, 1600]),
    ("3840 × 2160 (4K UHD)", [3840, 2160]),
];

pub(crate) fn list(ui: &mut egui::Ui, project: &mut Project, state: &mut UiState) {
    ui.horizontal(|ui| {
        ui.strong("PROJECTORS");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+").on_hover_text("Add output").clicked() {
                state.selected = Some(project.add_output());
            }
        });
    });
    ui.separator();

    if project.outputs.is_empty() {
        ui.weak("No outputs yet. Add one for each projector.");
        return;
    }

    let mut remove = None;
    for output in &mut project.outputs {
        let selected = state.selected == Some(output.id);
        ui.horizontal(|ui| {
            // Live indicator: filled when the window is open.
            let dot = if output.enabled { "●" } else { "○" };
            ui.checkbox(&mut output.enabled, "")
                .on_hover_text("Open output window");
            if ui
                .selectable_label(selected, format!("{dot} {}", output.name))
                .clicked()
            {
                state.selected = Some(output.id);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("🗑")
                    .on_hover_text("Remove output")
                    .clicked()
                {
                    remove = Some(output.id);
                }
            });
        });
    }
    if let Some(id) = remove {
        project.remove_output(id);
    }
}

pub(crate) fn inspector(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
) {
    let Some(output) = state.selected.and_then(|id| project.output_mut(id)) else {
        ui.weak("Select an output.");
        return;
    };

    egui::Grid::new("output props")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut output.name);
            ui.end_row();

            ui.label("Resolution");
            resolution_editor(ui, output);
            ui.end_row();

            ui.label("Window");
            ui.checkbox(&mut output.enabled, "Open");
            ui.end_row();

            ui.label("Monitor");
            monitor_picker(ui, output, view);
            ui.end_row();

            ui.label("Fullscreen");
            ui.add_enabled_ui(output.enabled, |ui| {
                ui.checkbox(&mut output.fullscreen, "")
                    .on_hover_text("Also: F in the output window. Esc leaves fullscreen.");
            });
            ui.end_row();

            ui.label("NDI out");
            ndi_out(ui, output, view);
            ui.end_row();

            ui.label("Guides");
            ui.checkbox(&mut output.show_guides, "On projector")
                .on_hover_text("Show surface outlines and corner handles on the projector while you map. Also: G in the output window.");
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.strong("Test pattern");
    ui.separator();
    pattern_editor(ui, output);

    ui.add_space(8.0);
    egui::CollapsingHeader::new("Edge blend")
        .default_open(output.edge_blend.is_active())
        .show(ui, |ui| edge_blend_editor(ui, output));

    if let Some(key) = &output.monitor
        && let Some(m) = view.monitors.iter().find(|m| &m.key == key)
        && m.size != output.resolution
    {
        ui.add_space(8.0);
        ui.colored_label(
            egui::Color32::from_rgb(230, 160, 60),
            format!(
                "Monitor reports {} × {} but the output renders {} × {}. \
                 Match them for pixel-exact mapping.",
                m.size[0], m.size[1], output.resolution[0], output.resolution[1]
            ),
        );
        if ui.button("Use monitor resolution").clicked() {
            output.resolution = m.size;
        }
    }
}

fn resolution_editor(ui: &mut egui::Ui, output: &mut OutputConfig) {
    ui.vertical(|ui| {
        let current = RESOLUTION_PRESETS
            .iter()
            .find(|(_, r)| *r == output.resolution)
            .map(|(name, _)| (*name).to_owned())
            .unwrap_or_else(|| "Custom".to_owned());
        egui::ComboBox::from_id_salt("resolution preset")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (name, res) in RESOLUTION_PRESETS {
                    ui.selectable_value(&mut output.resolution, *res, *name);
                }
            });
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut output.resolution[0])
                    .range(16..=8192)
                    .suffix(" w"),
            );
            ui.add(
                egui::DragValue::new(&mut output.resolution[1])
                    .range(16..=8192)
                    .suffix(" h"),
            );
        });
    });
}

fn monitor_picker(ui: &mut egui::Ui, output: &mut OutputConfig, view: &ViewData<'_>) {
    let selected_text = match &output.monitor {
        None => "Any".to_owned(),
        Some(key) if view.monitors.iter().any(|m| &m.key == key) => key.clone(),
        Some(key) => format!("{key} (missing)"),
    };
    egui::ComboBox::from_id_salt("monitor")
        .selected_text(selected_text)
        .width(180.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut output.monitor, None, "Any");
            for m in view.monitors {
                let hz = m
                    .refresh_hz
                    .map(|hz| format!(" @ {hz:.0} Hz"))
                    .unwrap_or_default();
                let label = format!("{} — {} × {}{hz}", m.key, m.size[0], m.size[1]);
                ui.selectable_value(&mut output.monitor, Some(m.key.clone()), label);
            }
        });
}

fn pattern_editor(ui: &mut egui::Ui, output: &mut OutputConfig) {
    let solid = match output.test_pattern {
        TestPattern::Solid(c) => TestPattern::Solid(c),
        _ => TestPattern::Solid([1.0, 1.0, 1.0]),
    };
    ui.horizontal_wrapped(|ui| {
        for p in [
            TestPattern::Off,
            TestPattern::Grid,
            TestPattern::ColorBars,
            solid,
        ] {
            let active = std::mem::discriminant(&p) == std::mem::discriminant(&output.test_pattern);
            if ui.selectable_label(active, p.label()).clicked() && !active {
                output.test_pattern = p;
            }
        }
    });
    match &mut output.test_pattern {
        TestPattern::Grid => {
            ui.horizontal(|ui| {
                ui.label("Cell size");
                ui.add(
                    egui::DragValue::new(&mut output.grid_cell_px)
                        .range(16..=1024)
                        .suffix(" px"),
                );
            });
        }
        TestPattern::Solid(color) => {
            ui.horizontal(|ui| {
                ui.label("Color");
                ui.color_edit_button_rgb(color);
            });
        }
        _ => {}
    }
}

fn ndi_out(ui: &mut egui::Ui, output: &mut OutputConfig, view: &ViewData<'_>) {
    ui.vertical(|ui| {
        let available = view.ndi.runtime.is_ok();
        ui.add_enabled(
            available,
            egui::Checkbox::new(&mut output.ndi_send, "Publish"),
        )
        .on_hover_text("Send this output to the network as an NDI source")
        .on_disabled_hover_text("NDI runtime not installed");
        if output.ndi_send {
            match view.ndi.outputs.get(&output.id) {
                Some(s) => {
                    ui.weak(format!("as \"{}\" · {} frames", s.name, s.sent));
                    if s.dropped > 0 {
                        ui.weak(format!("{} dropped (network or CPU busy)", s.dropped));
                    }
                }
                None => {
                    ui.weak(format!("as \"{}\"", output.ndi_name()));
                }
            }
        }
    });
}

/// Soft ramps where this projector overlaps its neighbors.
fn edge_blend_editor(ui: &mut egui::Ui, output: &mut OutputConfig) {
    let [w, h] = output.resolution.map(|v| v.max(1) as f32);
    let blend = &mut output.edge_blend;
    egui::Grid::new("edge blend")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            // Widths edited in output pixels; stored as fractions.
            for (name, value, size) in [
                ("Left", &mut blend.left, w),
                ("Right", &mut blend.right, w),
                ("Top", &mut blend.top, h),
                ("Bottom", &mut blend.bottom, h),
            ] {
                ui.label(name);
                let mut px = *value * size;
                if ui
                    .add(
                        egui::Slider::new(&mut px, 0.0..=size * 0.5)
                            .suffix(" px")
                            .max_decimals(0),
                    )
                    .on_hover_text("Width of the overlap with the neighboring projector")
                    .changed()
                {
                    *value = (px / size).clamp(0.0, 0.5);
                }
                ui.end_row();
            }
            ui.label("Gamma");
            ui.add(egui::Slider::new(&mut blend.gamma, 1.0..=3.0))
                .on_hover_text("Projector gamma; raise it if the overlap looks too bright");
            ui.end_row();
            ui.label("Curve");
            ui.add(egui::Slider::new(&mut blend.curve, 1.0..=4.0))
                .on_hover_text(
                    "1 = linear ramp; higher = S-curve, which hides small misalignments",
                );
            ui.end_row();
        });
    if ui
        .add_enabled(
            *blend != rustmap_core::EdgeBlend::default(),
            egui::Button::new("Reset blend"),
        )
        .clicked()
    {
        *blend = rustmap_core::EdgeBlend::default();
    }
}

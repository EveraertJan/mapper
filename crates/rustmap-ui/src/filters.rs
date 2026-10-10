//! Column 3, filters: what happens to the picture between taking a region of
//! an input and placing it on an output. For now the selected input's video
//! transport and the selected surface's color adjustments.

use rustmap_core::{MediaSource, Project};

use crate::{UiAction, UiState, ViewData};

pub(crate) fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    // ---- the input ----
    if let Some(media) = state.selected_media.and_then(|id| project.media_item(id)) {
        let id = media.id;
        let name = media.name.clone();
        let video = matches!(media.source, MediaSource::Video { .. });
        ui.label(egui::RichText::new(format!("Input: {name}")).strong());
        if video {
            ui.add_space(2.0);
            crate::media::video_controls(ui, project, id, view, actions);
        } else {
            ui.weak("No playback controls for this input.");
        }
    } else {
        ui.weak("No input selected.");
    }
    ui.add_space(8.0);
    ui.separator();

    // ---- the selected surface ----
    let Some(id) = state
        .selected_surface
        .filter(|id| project.surface(*id).is_some())
    else {
        ui.weak("Select a surface (a region in column 2, or in column 5) to filter it.");
        return;
    };
    let surface = project.surface_mut(id).expect("checked");
    ui.label(egui::RichText::new(format!("Surface: {}", surface.name)).strong());
    ui.weak("Applied in this order: shader, color, posterize.");
    ui.add_space(4.0);

    egui::CollapsingHeader::new(egui::RichText::new("Shader").strong())
        .id_salt("filter_shader")
        .default_open(true)
        .show(ui, |ui| shader(ui, surface, view, actions));
    egui::CollapsingHeader::new(egui::RichText::new("Color").strong())
        .id_salt("filter_color")
        .default_open(true)
        .show(ui, |ui| {
            crate::surfaces::color_editor(ui, &mut surface.color)
        });
    egui::CollapsingHeader::new(egui::RichText::new("Posterize").strong())
        .id_salt("filter_posterize")
        .default_open(true)
        .show(ui, |ui| posterize(ui, &mut surface.color.posterize));
}

/// A WGSL file and its four knobs.
fn shader(
    ui: &mut egui::Ui,
    surface: &mut rustmap_core::Surface,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    let id = surface.id;
    let Some(filter) = &mut surface.shader else {
        ui.horizontal(|ui| {
            ui.weak("None");
            if ui
                .button("Choose file…")
                .on_hover_text("A WGSL shader filter (examples come with RUSTMAP)")
                .clicked()
            {
                actions.push(UiAction::PickShader(id));
            }
        });
        return;
    };
    let name = filter
        .path
        .file_name()
        .map_or_else(|| "?".into(), |n| n.to_string_lossy().into_owned());
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&name).monospace())
            .on_hover_text(filter.path.display().to_string());
        if ui.small_button("Change…").clicked() {
            actions.push(UiAction::PickShader(id));
        }
        if ui.small_button("Remove").clicked() {
            remove = true;
        }
    });
    match view.shader_errors.get(&filter.path) {
        Some(err) => {
            ui.colored_label(crate::editor::ERROR, err);
            ui.weak("Fix the file and save it: it reloads by itself.");
        }
        None => {
            ui.weak("Reloads when the file is saved.");
        }
    }
    egui::Grid::new("shader_params")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (i, label) in ["A", "B", "C", "D"].iter().enumerate() {
                ui.label(*label)
                    .on_hover_text(format!("rm_param({i}u) in the shader"));
                ui.add(egui::Slider::new(&mut filter.params[i], 0.0..=1.0));
                ui.end_row();
            }
        });
    if remove {
        surface.shader = None;
    }
}

/// Bits per channel: off, or 1..=8 with quick picks.
fn posterize(ui: &mut egui::Ui, bits: &mut u8) {
    ui.horizontal(|ui| {
        ui.selectable_value(bits, 0, "Off");
        for b in [2u8, 6, 8] {
            ui.selectable_value(bits, b, format!("{b}-bit"))
                .on_hover_text(format!("{} levels per channel", 1u32 << b));
        }
    });
    let mut on = *bits > 0;
    let mut value = (*bits).max(1);
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Slider::new(&mut value, 1..=8)
                    .suffix(" bit")
                    .text("per channel"),
            )
            .changed()
        {
            on = true;
        }
    });
    if on {
        *bits = value;
    }
    if *bits == 8 {
        ui.weak("8 bit is the output's own depth: no visible change.");
    }
}

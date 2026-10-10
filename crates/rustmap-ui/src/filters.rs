//! Column 3, filters: what happens to the picture between taking a region of
//! an input and placing it on an output. The selected input's video
//! transport, the selected surface's shader chain, and the shader library.

use rustmap_core::{FilterUse, MediaSource, Project};

use crate::{ShaderEntry, ShaderOrigin, UiAction, UiState, ViewData};

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

    // ---- the selected surface's chain ----
    match state
        .selected_surface
        .filter(|id| project.surface(*id).is_some())
    {
        Some(id) => chain(ui, project, id, view),
        None => {
            ui.weak("Select a surface (a region in column 2, or in column 5) to give it shaders.");
        }
    }

    ui.add_space(10.0);
    ui.separator();
    library(ui, view, actions);
}

fn find<'a>(view: &'a ViewData<'_>, id: &str) -> Option<&'a ShaderEntry> {
    view.shaders.iter().find(|e| e.info.id == id)
}

/// The surface's shaders, top to bottom, each with its sliders.
fn chain(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: rustmap_core::SurfaceId,
    view: &ViewData<'_>,
) {
    let Some(surface) = project.surface_mut(id) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Shaders: {}", surface.name)).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("+ Add", |ui| {
                ui.set_min_width(200.0);
                let mut last = None;
                for entry in view.shaders {
                    if last.is_some() && last != Some(entry.origin) {
                        ui.separator();
                    }
                    last = Some(entry.origin);
                    let label = if entry.error.is_some() {
                        format!("{} (broken)", entry.info.name)
                    } else {
                        entry.info.name.clone()
                    };
                    let r = ui.button(label);
                    let r = if entry.info.description.is_empty() {
                        r
                    } else {
                        r.on_hover_text(&entry.info.description)
                    };
                    if r.clicked() {
                        surface.filters.push(FilterUse::new(&entry.info));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Add a shader to this surface's chain");
        });
    });
    if let Some(err) = view.shader_errors.get(&id) {
        ui.colored_label(crate::editor::ERROR, err);
        ui.weak("The surface draws without its shaders until this is fixed.");
    }
    if surface.filters.is_empty() {
        ui.weak("No shaders: the picture goes through unchanged. + Add puts one on.");
        return;
    }
    ui.weak("Run top to bottom.");

    let n = surface.filters.len();
    let mut remove = None;
    let mut swap = None;
    for (i, f) in surface.filters.iter_mut().enumerate() {
        let entry = find(view, &f.shader);
        let name = entry.map_or_else(|| f.shader.clone(), |e| e.info.name.clone());
        ui.push_id(("filter", i), |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.enabled, "").on_hover_text("On / off");
                    let title = ui.label(egui::RichText::new(&name).strong());
                    if let Some(e) = entry
                        && !e.info.description.is_empty()
                    {
                        title.on_hover_text(&e.info.description);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("🗑")
                            .on_hover_text("Remove from chain")
                            .clicked()
                        {
                            remove = Some(i);
                        }
                        if ui
                            .add_enabled(i + 1 < n, egui::Button::new("v").small())
                            .on_hover_text("Move down")
                            .clicked()
                        {
                            swap = Some((i, i + 1));
                        }
                        if ui
                            .add_enabled(i > 0, egui::Button::new("^").small())
                            .on_hover_text("Move up")
                            .clicked()
                        {
                            swap = Some((i - 1, i));
                        }
                    });
                });
                let Some(entry) = entry else {
                    ui.colored_label(
                        crate::editor::ERROR,
                        format!("“{}” isn't in the shader library.", f.shader),
                    );
                    return;
                };
                f.fit(&entry.info);
                if i > 0 {
                    ui.checkbox(&mut f.own_pass, egui::RichText::new("Own pass").small())
                        .on_hover_text(
                            "Render the shaders above into an image first, so this shader \
                             sees their result when it samples other pixels (pixelate, \
                             wave, RGB split…). Costs an extra render pass.",
                        );
                }
                ui.add_enabled_ui(f.enabled, |ui| {
                    egui::Grid::new("params")
                        .num_columns(2)
                        .spacing([8.0, 3.0])
                        .show(ui, |ui| {
                            for (j, p) in entry.info.params.iter().enumerate() {
                                ui.label(&p.label).on_hover_text(format!("p_{}()", p.id));
                                let mut slider = egui::Slider::new(&mut f.params[j], p.min..=p.max);
                                if p.step > 0.0 {
                                    slider = slider.step_by(f64::from(p.step));
                                }
                                if p.step >= 1.0 {
                                    slider = slider.fixed_decimals(0);
                                }
                                if ui
                                    .add(slider)
                                    .on_hover_text(
                                        "Double-click the number to type; right-click: default",
                                    )
                                    .secondary_clicked()
                                {
                                    f.params[j] = p.default;
                                }
                                ui.end_row();
                            }
                        });
                });
            });
        });
    }
    if let Some((a, b)) = swap {
        surface.filters.swap(a, b);
    }
    if let Some(i) = remove {
        surface.filters.remove(i);
    }
}

/// Every shader there is, and adding or removing your own.
fn library(ui: &mut egui::Ui, view: &ViewData<'_>, actions: &mut Vec<UiAction>) {
    egui::CollapsingHeader::new(egui::RichText::new("Shader library").strong())
        .id_salt("shader_library")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Add file…")
                    .on_hover_text("Copy a .wgsl shader into your library: every show can use it")
                    .clicked()
                {
                    actions.push(UiAction::AddShaderFile);
                }
                if ui
                    .button("New shader…")
                    .on_hover_text("Start a shader from a template in your library and open it")
                    .clicked()
                {
                    actions.push(UiAction::NewShader);
                }
                if ui.button("Open folder").clicked() {
                    actions.push(UiAction::OpenShaderFolder);
                }
            });
            ui.weak("Library files reload when saved.");
            ui.add_space(4.0);
            for e in view.shaders {
                ui.horizontal(|ui| {
                    let tag = match e.origin {
                        ShaderOrigin::BuiltIn => "built in",
                        ShaderOrigin::Library => "library",
                        ShaderOrigin::Show => "this show",
                    };
                    let name = ui.label(&e.info.name);
                    if !e.info.description.is_empty() {
                        name.on_hover_text(&e.info.description);
                    }
                    ui.weak(tag);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| match e.origin {
                            ShaderOrigin::Library => {
                                if ui
                                    .small_button("🗑")
                                    .on_hover_text("Delete from your library")
                                    .clicked()
                                {
                                    actions.push(UiAction::RemoveShader(e.info.id.clone()));
                                }
                                if let Some(path) = &e.path
                                    && ui
                                        .small_button("Edit")
                                        .on_hover_text("Open the file")
                                        .clicked()
                                {
                                    actions.push(UiAction::OpenShader(path.clone()));
                                }
                            }
                            ShaderOrigin::Show => {
                                if ui
                                    .small_button("Keep")
                                    .on_hover_text("Add to your library so every show can use it")
                                    .clicked()
                                {
                                    actions.push(UiAction::KeepShader(e.info.id.clone()));
                                }
                            }
                            ShaderOrigin::BuiltIn => {}
                        },
                    );
                });
                if let Some(err) = &e.error {
                    ui.colored_label(crate::editor::ERROR, err);
                }
            }
        });
}

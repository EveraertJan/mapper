use rustmap_core::{BlendMode, ColorAdjust, MASK_MAX_POINTS, Mask, Mesh, Project, Shape, Surface};
use rustmap_geom::{MeshGrid, Vec2, centered_rect, is_convex, quad_from_arrays, quad_to_arrays};

use crate::warp::{corner_handle, set_warp_handle, warp_handles};
use crate::{EditLayer, UiAction, UiState, ViewData};

/// Largest mesh in either direction.
const MESH_MAX: u32 = 16;

const CORNER_NAMES: [&str; 4] = ["Top left", "Top right", "Bottom right", "Bottom left"];

pub(crate) fn list(ui: &mut egui::Ui, project: &mut Project, state: &mut UiState) {
    let Some(output) = state.selected else { return };
    ui.horizontal(|ui| {
        ui.strong("SURFACES");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+").on_hover_text("Add quad").clicked()
                && let Some(id) = project.add_surface(output)
            {
                state.select_surface(Some(id));
            }
        });
    });
    ui.separator();

    let ids: Vec<_> = project.surfaces_on(output).map(|s| s.id).collect();
    if ids.is_empty() {
        ui.weak("No surfaces on this output.");
        return;
    }
    let mut remove = None;
    // Top of the list = drawn on top, like layers in an image editor.
    for id in ids.into_iter().rev() {
        let Some(surface) = project.surface_mut(id) else {
            continue;
        };
        ui.horizontal(|ui| {
            ui.checkbox(&mut surface.visible, "")
                .on_hover_text("Visible");
            let selected = state.selected_surface == Some(id);
            if ui.selectable_label(selected, &surface.name).clicked() {
                state.select_surface(Some(id));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("🗑")
                    .on_hover_text("Remove surface")
                    .clicked()
                {
                    remove = Some(id);
                }
            });
        });
    }
    if let Some(id) = remove {
        project.remove_surface(id);
    }
}

pub(crate) fn inspector(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    let Some(id) = state.selected_surface else {
        return;
    };
    let Some(surface) = project.surface(id) else {
        return;
    };
    let Some(output_res) = project.output(surface.output).map(|o| o.resolution) else {
        return;
    };
    let media_names: Vec<_> = project
        .media
        .iter()
        .map(|m| (m.id, m.name.clone()))
        .collect();
    let media_size = surface
        .media
        .and_then(|m| view.media.get(&m))
        .and_then(|s| s.size());

    let mut restack = None;
    let Some(surface) = project.surface_mut(id) else {
        return;
    };

    egui::Grid::new("surface props")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut surface.name);
            ui.end_row();

            ui.label("Media");
            ui.horizontal(|ui| {
                let current = surface
                    .media
                    .and_then(|m| media_names.iter().find(|(id, _)| *id == m))
                    .map_or("None (checker)", |(_, name)| name.as_str())
                    .to_owned();
                egui::ComboBox::from_id_salt("surface media")
                    .selected_text(current)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut surface.media, None, "None (checker)");
                        for (mid, name) in &media_names {
                            ui.selectable_value(&mut surface.media, Some(*mid), name);
                        }
                    });
                if ui.button("…").on_hover_text("Load image").clicked() {
                    actions.push(UiAction::ImportImages {
                        assign_to: Some(id),
                    });
                }
            });
            ui.end_row();

            ui.label("Opacity");
            ui.add(egui::Slider::new(&mut surface.opacity, 0.0..=1.0));
            ui.end_row();

            ui.label("Shape");
            ui.horizontal(|ui| {
                for shape in Shape::ALL {
                    ui.selectable_value(&mut surface.shape, shape, shape.label());
                }
            });
            ui.end_row();

            ui.label("Blend");
            egui::ComboBox::from_id_salt("blend")
                .selected_text(surface.blend.label())
                .show_ui(ui, |ui| {
                    for mode in BlendMode::ALL {
                        ui.selectable_value(&mut surface.blend, mode, mode.label());
                    }
                })
                .response
                .on_hover_text("How this surface combines with what is below it");
            ui.end_row();

            ui.label("Input");
            ui.horizontal(|ui| {
                let editing = state.editor_mode == crate::EditorMode::Input;
                if ui
                    .selectable_label(editing, "Edit region")
                    .on_hover_text("Choose which part of the media this surface shows (Input view)")
                    .clicked()
                {
                    state.editor_mode = if editing {
                        crate::EditorMode::Output
                    } else {
                        crate::EditorMode::Input
                    };
                }
                if ui
                    .add_enabled(
                        surface.input != Surface::FULL,
                        egui::Button::new("Full frame"),
                    )
                    .clicked()
                {
                    surface.input = Surface::FULL;
                }
            });
            ui.end_row();

            ui.label("Visible");
            ui.checkbox(&mut surface.visible, "");
            ui.end_row();

            ui.label("Order");
            ui.horizontal(|ui| {
                if ui.button("Bring forward").clicked() {
                    restack = Some(true);
                }
                if ui.button("Send back").clicked() {
                    restack = Some(false);
                }
            });
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.strong("Corners (output pixels)");
    ui.separator();
    corner_editor(ui, surface, output_res, state);

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Reset").clicked() {
            set_corners(surface, Surface::DEFAULT_CORNERS);
        }
        if ui
            .button("Fill output")
            .on_hover_text("Corners on the output corners")
            .clicked()
        {
            set_corners(surface, Surface::FULL);
        }
        let match_button = ui
            .add_enabled(
                media_size.is_some(),
                egui::Button::new("Match media aspect"),
            )
            .on_hover_text("Upright rectangle with the image's proportions")
            .on_disabled_hover_text("Assign a loaded image first");
        if match_button.clicked()
            && let Some([w, h]) = media_size
        {
            set_corners(
                surface,
                quad_to_arrays(&centered_rect(w as f32 / h.max(1) as f32, output_res, 0.6)),
            );
        }
    });

    if surface.mesh.is_none() && !is_convex(&quad_from_arrays(&surface.corners)) {
        ui.add_space(6.0);
        ui.colored_label(
            egui::Color32::from_rgb(235, 90, 70),
            "This quad is folded or bent inward, so it can't be mapped. Drag a corner back out.",
        );
    }

    ui.add_space(10.0);
    egui::CollapsingHeader::new("Mesh warp")
        .default_open(surface.mesh.is_some())
        .show(ui, |ui| mesh_editor(ui, surface, state));
    egui::CollapsingHeader::new("Mask")
        .default_open(surface.mask.is_some())
        .show(ui, |ui| mask_editor(ui, surface, state));
    egui::CollapsingHeader::new("Color")
        .default_open(!surface.color.is_identity())
        .show(ui, |ui| color_editor(ui, &mut surface.color));

    if let Some(up) = restack {
        project.restack_surface(id, up);
    }
}

/// Places the four corners; a mesh is rebuilt flat inside the new quad.
fn set_corners(surface: &mut Surface, corners: [[f32; 2]; 4]) {
    surface.corners = corners;
    if let Some(m) = &mut surface.mesh {
        let grid = MeshGrid::from_quad(
            &quad_from_arrays(&corners),
            m.cols as usize,
            m.rows as usize,
            m.smooth,
        );
        m.points = grid.points_as_arrays();
    }
}

fn mesh_from_grid(grid: &MeshGrid) -> Mesh {
    Mesh {
        cols: grid.cols as u32,
        rows: grid.rows as u32,
        points: grid.points_as_arrays(),
        smooth: grid.smooth,
    }
}

fn mesh_editor(ui: &mut egui::Ui, surface: &mut Surface, state: &mut UiState) {
    let mut on = surface.mesh.is_some();
    if ui
        .checkbox(&mut on, "Mesh warp")
        .on_hover_text("Bend the surface with a grid of points, for curved or uneven objects")
        .changed()
    {
        surface.mesh = on.then(|| {
            mesh_from_grid(&MeshGrid::from_quad(
                &quad_from_arrays(&surface.corners),
                3,
                3,
                true,
            ))
        });
        state.selected_corner = None;
        state.set_layer(EditLayer::Warp);
    }
    let Some(mesh) = &mut surface.mesh else {
        ui.weak("Off: the four corners warp in perspective.");
        return;
    };
    let Some(grid) = MeshGrid::from_arrays(
        mesh.cols as usize,
        mesh.rows as usize,
        &mesh.points,
        mesh.smooth,
    ) else {
        ui.colored_label(crate::editor::ERROR, "This mesh is damaged.");
        if ui.button("Remove mesh").clicked() {
            surface.mesh = None;
        }
        return;
    };
    let (mut cols, mut rows) = (mesh.cols, mesh.rows);
    ui.horizontal(|ui| {
        ui.label("Points");
        ui.add(
            egui::DragValue::new(&mut cols)
                .range(2..=MESH_MAX)
                .prefix("cols "),
        );
        ui.add(
            egui::DragValue::new(&mut rows)
                .range(2..=MESH_MAX)
                .prefix("rows "),
        );
    });
    if (cols, rows) != (mesh.cols, mesh.rows) {
        // Keep the current shape with the new number of points.
        *mesh = mesh_from_grid(&grid.resample(cols as usize, rows as usize));
        state.selected_corner = None;
    }
    ui.checkbox(&mut mesh.smooth, "Smooth curves")
        .on_hover_text("Curves through the points (on) or straight lines between them (off)");
    if ui
        .button("Flatten")
        .on_hover_text("Undo all bending; keep the corners")
        .clicked()
    {
        let corners = mesh.corners();
        *mesh = mesh_from_grid(&MeshGrid::from_quad(
            &quad_from_arrays(&corners),
            mesh.cols as usize,
            mesh.rows as usize,
            mesh.smooth,
        ));
    }
    match state
        .selected_corner
        .filter(|i| state.layer == EditLayer::Warp && *i < mesh.points.len())
    {
        Some(i) => {
            let (c, r) = (i as u32 % mesh.cols, i as u32 / mesh.cols);
            ui.weak(format!(
                "Point {}, {} selected · arrows nudge it",
                c + 1,
                r + 1
            ));
        }
        None => {
            ui.weak("Drag the points in the editor. Tab steps through them.");
        }
    }
}

fn mask_editor(ui: &mut egui::Ui, surface: &mut Surface, state: &mut UiState) {
    let Some(mask) = &mut surface.mask else {
        if ui
            .button("Add mask")
            .on_hover_text("Cut the surface to a polygon, e.g. to skip a window or a beam")
            .clicked()
        {
            surface.mask = Some(Mask::inset());
            state.set_layer(EditLayer::Mask);
        }
        return;
    };
    let mut remove = false;
    ui.horizontal(|ui| {
        let editing = state.layer == EditLayer::Mask;
        if ui
            .selectable_label(editing, "Edit points")
            .on_hover_text("Drag points in the editor; double-click an edge to add one")
            .clicked()
        {
            state.set_layer(if editing {
                EditLayer::Warp
            } else {
                EditLayer::Mask
            });
        }
        if ui.button("Remove mask").clicked() {
            remove = true;
        }
    });
    ui.checkbox(&mut mask.invert, "Invert")
        .on_hover_text("Hide the inside of the polygon instead of the outside");
    ui.horizontal(|ui| {
        ui.label("Feather");
        ui.add(egui::Slider::new(&mut mask.feather, 0.0..=0.25))
            .on_hover_text("Soft edge width, as a fraction of the surface");
    });
    ui.weak(format!(
        "{} of {MASK_MAX_POINTS} points · Delete removes the selected point",
        mask.points.len()
    ));
    if remove {
        surface.mask = None;
        state.set_layer(EditLayer::Warp);
    }
}

fn color_editor(ui: &mut egui::Ui, color: &mut ColorAdjust) {
    egui::Grid::new("color")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Brightness");
            ui.add(egui::Slider::new(&mut color.brightness, -1.0..=1.0));
            ui.end_row();
            ui.label("Contrast");
            ui.add(egui::Slider::new(&mut color.contrast, 0.0..=2.0));
            ui.end_row();
            ui.label("Saturation");
            ui.add(egui::Slider::new(&mut color.saturation, 0.0..=2.0));
            ui.end_row();
            ui.label("Hue");
            ui.add(egui::Slider::new(&mut color.hue, -180.0..=180.0).suffix("°"));
            ui.end_row();
            for (i, name) in ["Red", "Green", "Blue"].iter().enumerate() {
                ui.label(*name);
                ui.add(egui::Slider::new(&mut color.gain[i], 0.0..=2.0))
                    .on_hover_text("Channel gain, e.g. to match projectors");
                ui.end_row();
            }
        });
    if ui
        .add_enabled(!color.is_identity(), egui::Button::new("Reset color"))
        .clicked()
    {
        *color = ColorAdjust::default();
    }
}

fn corner_editor(ui: &mut egui::Ui, surface: &mut Surface, res: [u32; 2], state: &mut UiState) {
    let (w, h) = (res[0] as f32, res[1] as f32);
    let warp_layer = state.layer == EditLayer::Warp;
    egui::Grid::new("corners")
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for (corner, name) in CORNER_NAMES.iter().enumerate() {
                // With a mesh, the corners are its outer points.
                let i = corner_handle(surface, corner);
                let selected = warp_layer && state.selected_corner == Some(i);
                if ui.selectable_label(selected, *name).clicked() {
                    state.set_layer(EditLayer::Warp);
                    state.selected_corner = if selected { None } else { Some(i) };
                }
                let Some(c) = warp_handles(surface).get(i).copied() else {
                    continue;
                };
                let mut x = c.x * w;
                let mut y = c.y * h;
                let dx = ui.add(
                    egui::DragValue::new(&mut x)
                        .speed(0.5)
                        .max_decimals(1)
                        .prefix("x "),
                );
                let dy = ui.add(
                    egui::DragValue::new(&mut y)
                        .speed(0.5)
                        .max_decimals(1)
                        .prefix("y "),
                );
                if dx.changed() || dy.changed() {
                    set_warp_handle(surface, i, Vec2::new(x / w, y / h));
                }
                ui.end_row();
            }
        });
}

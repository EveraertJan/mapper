//! The output editor: the selected output's live image with its surfaces,
//! where corners are dragged, nudged and snapped.

use egui::{Color32, CursorIcon, Key, Pos2, Rect, Sense, Shape, Stroke};
use rustmap_core::{
    FixtureId, FixtureShape, MASK_MAX_POINTS, OutputId, Project, Surface, SurfaceId,
};
use rustmap_geom::{
    SnapTargets, Vec2, is_convex, polygon_insert_index, quad_from_arrays, snap_point,
};

use crate::theme::SELECTION;
use crate::warp::{Warp, corner_handle, mask_handles, set_warp_handle, warp_handles};
use crate::{EditLayer, UiAction, UiState, ViewData};

/// Grab radius around a corner, in screen points.
pub(crate) const HANDLE_HIT: f32 = 12.0;
pub(crate) const HANDLE_HALF: f32 = 5.0;
pub(crate) const SNAP_RADIUS: f32 = 8.0;
pub(crate) const HELP_HEIGHT: f32 = 22.0;
pub(crate) const ERROR: Color32 = Color32::from_rgb(235, 90, 70);

#[derive(Debug, Clone)]
pub(crate) enum Drag {
    /// A corner or mesh point (index into [`warp_handles`]).
    Corner { surface: SurfaceId, corner: usize },
    /// The whole surface; `last` is the previous pointer position.
    Move { surface: SurfaceId, last: Vec2 },
    /// A mask point.
    MaskPoint { surface: SurfaceId, point: usize },
    /// The whole mask, in surface uv; `last` is the previous pointer uv.
    MaskMove { surface: SurfaceId, last: Vec2 },
    /// An LED fixture handle (strip end, matrix corner, free point).
    FixtureHandle { fixture: FixtureId, handle: usize },
    /// A whole LED fixture.
    FixtureMove { fixture: FixtureId, last: Vec2 },
    /// Same, for a surface's input region (Input view).
    InputCorner { surface: SurfaceId, corner: usize },
    InputMove {
        surface: SurfaceId,
        start: [[f32; 2]; 4],
        origin: Vec2,
    },
}

/// Maps between normalized output space and the on-screen preview.
#[derive(Clone, Copy)]
pub(crate) struct View {
    pub(crate) rect: Rect,
}

impl View {
    pub(crate) fn to_screen(self, p: [f32; 2]) -> Pos2 {
        self.rect.min + egui::vec2(p[0] * self.rect.width(), p[1] * self.rect.height())
    }

    pub(crate) fn to_norm(self, p: Pos2) -> Vec2 {
        Vec2::new(
            (p.x - self.rect.min.x) / self.rect.width(),
            (p.y - self.rect.min.y) / self.rect.height(),
        )
    }

    /// Normalized units -> screen points, per axis.
    pub(crate) fn scale(self) -> Vec2 {
        Vec2::new(self.rect.width(), self.rect.height())
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    let Some(output_id) = state.selected else {
        ui.centered_and_justified(|ui| {
            if ui.button("Add your first output").clicked() {
                state.selected = Some(project.add_output());
            }
        });
        return;
    };
    let Some(output) = project.output(output_id) else {
        return;
    };
    let resolution = output.resolution;
    let preview = view.previews.get(&output_id).copied();

    // Output / Input tabs.
    let mode_before = state.editor_mode;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.editor_mode, crate::EditorMode::Output, "Output")
            .on_hover_text("Where surfaces land on the projector");
        ui.selectable_value(&mut state.editor_mode, crate::EditorMode::Input, "Input")
            .on_hover_text("Which part of its media the selected surface shows");
        ui.selectable_value(
            &mut state.editor_mode,
            crate::EditorMode::Console,
            "Console",
        )
        .on_hover_text("Faders for the DMX fixtures");
        ui.selectable_value(&mut state.editor_mode, crate::EditorMode::Dmx, "DMX")
            .on_hover_text("What goes out to the LEDs, channel by channel");
        if state.editor_mode == crate::EditorMode::Output
            && let Some(s) = state.selected_surface.and_then(|id| project.surface(id))
        {
            ui.separator();
            let has_mask = s.mask.is_some();
            let mut layer = state.layer;
            ui.selectable_value(&mut layer, EditLayer::Warp, "Warp")
                .on_hover_text("Drag corners (or mesh points)");
            ui.add_enabled_ui(has_mask, |ui| {
                ui.selectable_value(&mut layer, EditLayer::Mask, "Mask")
                    .on_hover_text("Drag mask points; double-click an edge to add one")
                    .on_disabled_hover_text("Add a mask in the inspector first");
            });
            state.set_layer(layer);
        }
    });
    if state.editor_mode != mode_before {
        state.selected_corner = None;
        state.drag = None;
    }
    match state.editor_mode {
        crate::EditorMode::Input => {
            crate::input::show(ui, project, state, view);
            return;
        }
        crate::EditorMode::Console => {
            crate::lights::console(ui, project, state, view, actions);
            return;
        }
        crate::EditorMode::Dmx => {
            crate::dmx::show(ui, project, state, view);
            return;
        }
        crate::EditorMode::Output => {}
    }

    if project.outputs.len() > 1 {
        output_strip(ui, project, state, view);
        ui.add_space(6.0);
    }

    // Fit the output's aspect ratio into the space left above the help line.
    let avail = ui.available_rect_before_wrap();
    let area = Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - HELP_HEIGHT))
        .shrink(8.0);
    let aspect = resolution[0] as f32 / resolution[1].max(1) as f32;
    let size = if area.width() / area.height().max(1.0) > aspect {
        egui::vec2(area.height() * aspect, area.height())
    } else {
        egui::vec2(area.width(), area.width() / aspect)
    };
    if size.x < 16.0 || size.y < 16.0 {
        return;
    }
    let v = View {
        rect: Rect::from_center_size(area.center(), size),
    };
    let response = ui.allocate_rect(v.rect, Sense::click_and_drag());

    handle_pointer(ui, &response, project, state, output_id, v);
    handle_keys(ui, project, state, resolution);

    let painter = ui.painter_at(avail);
    match preview {
        Some(tex) => {
            painter.image(
                tex,
                v.rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(v.rect, 0.0, Color32::BLACK);
        }
    }
    painter.rect_stroke(
        v.rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(70)),
        egui::StrokeKind::Outside,
    );

    let hover_fixture = response
        .hover_pos()
        .and_then(|p| fixture_hit(project, state, output_id, v.to_norm(p), v.scale()));
    let hovered = response
        .hover_pos()
        .filter(|_| hover_fixture.is_none())
        .and_then(|p| hit(project, state, output_id, v.to_norm(p), v.scale()));
    draw_surfaces(&painter, project, state, output_id, v, hovered);
    crate::fixtures::draw(&painter, project, state, output_id, v, hover_fixture);

    if project.surfaces_on(output_id).next().is_none() {
        painter.text(
            v.rect.center(),
            egui::Align2::CENTER_CENTER,
            "Add a quad (Surface > Add quad), an NDI source (Media > NDI source) or drop an image here",
            egui::FontId::proportional(15.0),
            Color32::from_gray(200),
        );
    }

    // Live coordinates while dragging a corner.
    if let (Some(Drag::Corner { surface, corner }), Some(pos)) = (&state.drag, response.hover_pos())
        && let Some(c) = project
            .surface(*surface)
            .and_then(|s| warp_handles(s).get(*corner).copied())
    {
        painter.text(
            pos + egui::vec2(14.0, 14.0),
            egui::Align2::LEFT_TOP,
            format!(
                "x {:.1}  y {:.1}",
                c.x * resolution[0] as f32,
                c.y * resolution[1] as f32
            ),
            egui::FontId::monospace(12.0),
            Color32::WHITE,
        );
    }

    let snap_hint = if state.snapping {
        "Alt: no snapping"
    } else {
        "snapping off"
    };
    painter.text(
        Pos2::new(avail.min.x + 8.0, avail.max.y - 6.0),
        egui::Align2::LEFT_BOTTOM,
        if state.selected_fixture.is_some() {
            "Drag the fixture's handles or the fixture · Arrows: 1 px (Shift 10) · Tab: next handle · Esc: deselect".to_owned()
        } else if state.layer == EditLayer::Mask {
            "Drag mask points or the mask · Double-click: add point · Delete: remove point · Arrows: nudge · Esc: back to warp".to_owned()
        } else {
            format!(
                "Drag corners or the quad · Arrows: 1 px (Shift 10) · Tab: next corner · Esc: deselect · {snap_hint}"
            )
        },
        egui::FontId::proportional(12.0),
        Color32::from_gray(140),
    );
    let snap_rect = Rect::from_min_size(
        Pos2::new(avail.max.x - 110.0, avail.max.y - HELP_HEIGHT + 2.0),
        egui::vec2(110.0, 20.0),
    );
    ui.put(
        snap_rect,
        egui::Checkbox::new(&mut state.snapping, "Snapping"),
    );
}

/// Thumbnails of all outputs, to switch which one is being edited.
fn output_strip(ui: &mut egui::Ui, project: &Project, state: &mut UiState, view: &ViewData<'_>) {
    ui.horizontal(|ui| {
        for output in &project.outputs {
            let selected = state.selected == Some(output.id);
            let size = egui::vec2(56.0 * output.aspect().clamp(0.5, 3.0), 56.0);
            ui.vertical(|ui| {
                let response = match view.previews.get(&output.id) {
                    Some(tex) => ui.add(egui::Image::new((*tex, size)).sense(Sense::click())),
                    None => ui.allocate_response(size, Sense::click()),
                };
                let stroke = if selected {
                    Stroke::new(2.0, SELECTION)
                } else {
                    Stroke::new(1.0, Color32::from_gray(60))
                };
                ui.painter()
                    .rect_stroke(response.rect, 0.0, stroke, egui::StrokeKind::Outside);
                if response.on_hover_text(&output.name).clicked() {
                    state.selected = Some(output.id);
                }
                ui.small(&output.name);
            });
        }
    });
}

/// What is under `p`: a handle of the selected surface first (corner, mesh
/// point or mask point, per layer), then any surface's outer corner (as a
/// warp handle index), then a surface body, topmost first.
fn hit(
    project: &Project,
    state: &UiState,
    output: OutputId,
    p: Vec2,
    scale: Vec2,
) -> Option<(SurfaceId, Option<usize>)> {
    let nearest = |pts: &[Vec2]| {
        pts.iter()
            .enumerate()
            .map(|(i, c)| (i, ((*c - p) * scale).length()))
            .filter(|(_, d)| *d <= HANDLE_HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if let Some(s) = state.selected_surface.and_then(|id| project.surface(id)) {
        let handles = match state.layer {
            EditLayer::Warp => warp_handles(s),
            EditLayer::Mask => mask_handles(s, &Warp::of(s)),
        };
        if let Some(i) = nearest(&handles) {
            return Some((s.id, Some(i)));
        }
        if state.layer == EditLayer::Mask {
            // Mask editing: only the selected surface reacts.
            return Warp::of(s).contains(p).then_some((s.id, None));
        }
    }
    let surfaces: Vec<&Surface> = project.surfaces_on(output).filter(|s| s.visible).collect();
    for s in surfaces.iter().rev() {
        let corners = s.corners.map(Vec2::from);
        if let Some(c) = nearest(&corners) {
            return Some((s.id, Some(corner_handle(s, c))));
        }
    }
    surfaces
        .iter()
        .rev()
        .find(|s| Warp::of(s).contains(p))
        .map(|s| (s.id, None))
}

fn handle_pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    project: &mut Project,
    state: &mut UiState,
    output: OutputId,
    v: View,
) {
    let (press_origin, alt) = ui.input(|i| (i.pointer.press_origin(), i.modifiers.alt));
    let mask_layer = state.layer == EditLayer::Mask;
    if !mask_layer && fixture_pointer(ui, response, project, state, output, v, alt) {
        return;
    }

    if response.drag_started()
        && let Some(origin) = press_origin
    {
        let p = v.to_norm(origin);
        state.drag = match hit(project, state, output, p, v.scale()) {
            Some((id, Some(i))) if mask_layer => {
                state.selected_corner = Some(i);
                Some(Drag::MaskPoint {
                    surface: id,
                    point: i,
                })
            }
            Some((id, None)) if mask_layer => project
                .surface(id)
                .and_then(|s| Warp::of(s).uv_at(p))
                .map(|uv| Drag::MaskMove {
                    surface: id,
                    last: uv,
                }),
            Some((id, Some(corner))) => {
                state.select_surface(Some(id));
                state.selected_corner = Some(corner);
                Some(Drag::Corner {
                    surface: id,
                    corner,
                })
            }
            Some((id, None)) => {
                state.select_surface(Some(id));
                Some(Drag::Move {
                    surface: id,
                    last: p,
                })
            }
            None => None,
        };
    }

    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let p = v.to_norm(pos);
        match state.drag.clone() {
            Some(Drag::Corner { surface, corner }) => {
                let target = if state.snapping && !alt {
                    let mut targets = SnapTargets::output_frame();
                    targets.points.extend(
                        project
                            .surfaces_on(output)
                            .filter(|s| s.id != surface && s.visible)
                            .flat_map(|s| s.corners.map(Vec2::from)),
                    );
                    snap_point(p, &targets, v.scale(), SNAP_RADIUS)
                } else {
                    p
                };
                if let Some(s) = project.surface_mut(surface) {
                    set_warp_handle(s, corner, target);
                }
            }
            Some(Drag::Move { surface, last }) => {
                let d = p - last;
                if let Some(s) = project.surface_mut(surface) {
                    s.translate(d.to_array());
                }
                state.drag = Some(Drag::Move { surface, last: p });
            }
            Some(Drag::MaskPoint { surface, point }) => {
                if let Some(s) = project.surface_mut(surface)
                    && let Some(uv) = Warp::of(s).uv_at(p)
                    && let Some(pt) = s.mask.as_mut().and_then(|m| m.points.get_mut(point))
                {
                    *pt = uv.clamp(Vec2::ZERO, Vec2::ONE).to_array();
                }
            }
            Some(Drag::MaskMove { surface, last }) => {
                if let Some(s) = project.surface_mut(surface)
                    && let Some(uv) = Warp::of(s).uv_at(p)
                {
                    let d = uv - last;
                    if let Some(m) = &mut s.mask {
                        for pt in &mut m.points {
                            *pt = (Vec2::from(*pt) + d).to_array();
                        }
                    }
                    state.drag = Some(Drag::MaskMove { surface, last: uv });
                }
            }
            _ => {}
        }
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }

    if response.drag_stopped() {
        state.drag = None;
    }

    // Double-click on the selected surface adds a mask point on the nearest edge.
    if mask_layer
        && response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(s) = state
            .selected_surface
            .and_then(|id| project.surface_mut(id))
        && let Some(uv) = Warp::of(s).uv_at(v.to_norm(pos))
        && let Some(mask) = &mut s.mask
        && mask.points.len() < MASK_MAX_POINTS
    {
        let uv = uv.clamp(Vec2::ZERO, Vec2::ONE);
        let pts: Vec<Vec2> = mask.points.iter().map(|p| Vec2::from(*p)).collect();
        let at = polygon_insert_index(&pts, uv);
        mask.points.insert(at, uv.to_array());
        state.selected_corner = Some(at);
        return;
    }

    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        match hit(project, state, output, v.to_norm(pos), v.scale()) {
            Some((_, corner)) if mask_layer => state.selected_corner = corner,
            None if mask_layer => state.selected_corner = None,
            Some((id, corner)) => {
                state.select_surface(Some(id));
                state.selected_corner = corner;
            }
            None => {
                state.select_surface(None);
                state.select_fixture(None);
            }
        }
    }

    if state.drag.is_none()
        && let Some(pos) = response.hover_pos()
    {
        match hit(project, state, output, v.to_norm(pos), v.scale()) {
            Some((_, Some(_))) => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
            Some((_, None)) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
            None => {}
        }
    }
}

/// Arrow keys nudge the selected handle (or the whole surface / mask) by
/// output pixels; Tab cycles handles; Delete removes a mask point. Only when
/// no text field has focus.
fn handle_keys(ui: &egui::Ui, project: &mut Project, state: &mut UiState, resolution: [u32; 2]) {
    if state.selected_fixture.is_some() {
        fixture_keys(ui, project, state, resolution);
        return;
    }
    let Some(id) = state.selected_surface else {
        return;
    };
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    let (delta, tab, back_tab, escape, delete) = ui.ctx().input_mut(|i| {
        let step = if i.modifiers.shift { 10.0 } else { 1.0 };
        let mut d = Vec2::ZERO;
        if i.key_pressed(Key::ArrowLeft) {
            d.x -= step;
        }
        if i.key_pressed(Key::ArrowRight) {
            d.x += step;
        }
        if i.key_pressed(Key::ArrowUp) {
            d.y -= step;
        }
        if i.key_pressed(Key::ArrowDown) {
            d.y += step;
        }
        let back_tab = i.consume_key(egui::Modifiers::SHIFT, Key::Tab);
        let tab = i.consume_key(egui::Modifiers::NONE, Key::Tab);
        let escape = i.consume_key(egui::Modifiers::NONE, Key::Escape);
        let delete = i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace);
        (d, tab, back_tab, escape, delete)
    });
    let Some(s) = project.surface_mut(id) else {
        return;
    };
    let mask_layer = state.layer == EditLayer::Mask;
    let count = if mask_layer {
        s.mask.as_ref().map_or(0, |m| m.points.len())
    } else {
        warp_handles(s).len()
    };

    if (tab || back_tab) && count > 0 {
        let step = if back_tab { count - 1 } else { 1 };
        state.selected_corner = Some(
            state
                .selected_corner
                .filter(|c| *c < count)
                .map_or(0, |c| (c + step) % count),
        );
    }
    if escape {
        if state.selected_corner.is_some() {
            state.selected_corner = None;
        } else if mask_layer {
            state.set_layer(EditLayer::Warp);
        } else {
            state.select_surface(None);
        }
        return;
    }
    if delete
        && mask_layer
        && let Some(i) = state.selected_corner
        && let Some(mask) = &mut s.mask
        && mask.points.len() > 3
        && i < mask.points.len()
    {
        mask.points.remove(i);
        state.selected_corner = None;
    }
    if delta == Vec2::ZERO {
        return;
    }
    let d = delta / Vec2::new(resolution[0].max(1) as f32, resolution[1].max(1) as f32);
    match (mask_layer, state.selected_corner.filter(|c| *c < count)) {
        (false, Some(i)) => {
            let p = warp_handles(s)[i] + d;
            set_warp_handle(s, i, p);
        }
        (false, None) => s.translate(d.to_array()),
        (true, sel) => {
            // Nudge in output pixels, then map back into the surface.
            let warp = Warp::of(s);
            let Some(mask) = &mut s.mask else { return };
            for (i, pt) in mask.points.iter_mut().enumerate() {
                if sel.is_some_and(|sel| sel != i) {
                    continue;
                }
                if let Some(uv) = warp.uv_at(warp.map(Vec2::from(*pt)) + d) {
                    *pt = uv.to_array();
                }
            }
        }
    }
}

fn draw_surfaces(
    painter: &egui::Painter,
    project: &Project,
    state: &UiState,
    output: OutputId,
    v: View,
    hovered: Option<(SurfaceId, Option<usize>)>,
) {
    let to_screen = |p: &Vec2| v.to_screen(p.to_array());
    let mut selected_shapes = Vec::new();
    let mut label = None;
    for s in project.surfaces_on(output) {
        let warp = Warp::of(s);
        let pts: Vec<Pos2> = s.corners.iter().map(|c| v.to_screen(*c)).collect();
        // The visible outline: the quad (or mesh), or the triangle/circle inside it.
        let outline: Vec<Pos2> = warp.outline(s.shape).iter().map(to_screen).collect();
        let selected = state.selected_surface == Some(s.id);
        let convex = s.mesh.is_some() || is_convex(&quad_from_arrays(&s.corners));
        let mut color = if !convex {
            ERROR
        } else if selected {
            SELECTION
        } else {
            Color32::from_white_alpha(140)
        };
        if !s.visible {
            color = color.gamma_multiply(0.4);
        }

        if !selected {
            let hover = hovered.is_some_and(|(id, _)| id == s.id);
            let width = if hover { 2.0 } else { 1.0 };
            painter.add(Shape::closed_line(outline, Stroke::new(width, color)));
            for p in &pts {
                painter.circle_filled(*p, 2.5, color);
            }
            continue;
        }

        // Selected: drawn last so it sits above the other outlines.
        let faint = Stroke::new(1.0, color.gamma_multiply(0.4));
        let mask_layer = state.layer == EditLayer::Mask;
        for line in warp.grid_lines() {
            selected_shapes.push(Shape::line(line.iter().map(to_screen).collect(), faint));
        }
        if s.shape != rustmap_core::Shape::Quad || s.mesh.is_some() {
            // Faint frame around the whole warp area.
            selected_shapes.push(Shape::closed_line(
                warp.outline(rustmap_core::Shape::Quad)
                    .iter()
                    .map(to_screen)
                    .collect(),
                faint,
            ));
        }
        selected_shapes.push(Shape::closed_line(
            outline,
            Stroke::new(if mask_layer { 1.0 } else { 2.0 }, color),
        ));

        if let Some(mask) = &s.mask {
            let mask_color = Color32::from_rgb(90, 200, 250);
            let line: Vec<Pos2> = warp
                .map_polyline(&mask.points, true)
                .iter()
                .map(to_screen)
                .collect();
            let stroke = if mask_layer {
                Stroke::new(2.0, mask_color)
            } else {
                Stroke::new(1.0, mask_color.gamma_multiply(0.6))
            };
            selected_shapes.push(Shape::closed_line(line, stroke));
            if mask_layer {
                for (i, p) in mask_handles(s, &warp).iter().enumerate() {
                    let active =
                        state.selected_corner == Some(i) || hovered == Some((s.id, Some(i)));
                    let p = to_screen(p);
                    selected_shapes.push(Shape::circle_filled(
                        p,
                        HANDLE_HALF,
                        if active { mask_color } else { Color32::BLACK },
                    ));
                    selected_shapes.push(Shape::circle_stroke(
                        p,
                        HANDLE_HALF,
                        Stroke::new(1.5, mask_color),
                    ));
                }
            }
        }

        if !mask_layer {
            let handles = warp_handles(s);
            let corners: Vec<usize> = (0..4).map(|c| corner_handle(s, c)).collect();
            for (i, p) in handles.iter().enumerate() {
                let active = state.selected_corner == Some(i) || hovered == Some((s.id, Some(i)));
                // Mesh points that aren't corners get smaller handles.
                let half = if corners.contains(&i) {
                    HANDLE_HALF
                } else {
                    HANDLE_HALF * 0.75
                };
                let r = Rect::from_center_size(to_screen(p), egui::vec2(half * 2.0, half * 2.0));
                selected_shapes.push(Shape::rect_filled(
                    r,
                    1.0,
                    if active { color } else { Color32::BLACK },
                ));
                selected_shapes.push(Shape::rect_stroke(
                    r,
                    1.0,
                    Stroke::new(1.5, color),
                    egui::StrokeKind::Middle,
                ));
            }
        }
        // Below the middle, so it doesn't cover a mesh's center point.
        let center = to_screen(&warp.map(Vec2::splat(0.5)));
        let offset = if s.mesh.is_some() { 18.0 } else { 0.0 };
        label = Some((center + egui::vec2(0.0, offset), s.name.clone()));
    }
    painter.extend(selected_shapes);
    if let Some((pos, name)) = label {
        painter.text(
            pos,
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        );
    }
}

/// LED fixture under `p`, if fixtures should get the pointer there: the
/// selected fixture first, but the selected surface's own handles win over
/// other fixtures.
fn fixture_hit(
    project: &Project,
    state: &UiState,
    output: OutputId,
    p: Vec2,
    scale: Vec2,
) -> Option<(FixtureId, Option<usize>)> {
    if state.layer == EditLayer::Mask && state.selected_surface.is_some() {
        return None;
    }
    if state.selected_fixture.is_none()
        && let Some(sel) = state.selected_surface
        && let Some((id, Some(_))) = hit(project, state, output, p, scale)
        && id == sel
    {
        return None;
    }
    crate::fixtures::hit(project, state, output, p, scale)
}

/// Pointer handling for LED fixtures. Returns true if a fixture took it.
fn fixture_pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    project: &mut Project,
    state: &mut UiState,
    output: OutputId,
    v: View,
    alt: bool,
) -> bool {
    let press = ui.input(|i| i.pointer.press_origin());
    if response.drag_started()
        && let Some(origin) = press
        && let Some((id, handle)) =
            fixture_hit(project, state, output, v.to_norm(origin), v.scale())
    {
        state.select_fixture(Some(id));
        state.selected_corner = handle;
        state.drag = Some(match handle {
            Some(handle) => Drag::FixtureHandle {
                fixture: id,
                handle,
            },
            None => Drag::FixtureMove {
                fixture: id,
                last: v.to_norm(origin),
            },
        });
        return true;
    }
    match state.drag.clone() {
        Some(Drag::FixtureHandle { fixture, handle }) => {
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let p = v.to_norm(pos);
                let target = if state.snapping && !alt {
                    let mut targets = SnapTargets::output_frame();
                    targets.points.extend(
                        project
                            .surfaces_on(output)
                            .filter(|s| s.visible)
                            .flat_map(|s| s.corners.map(Vec2::from)),
                    );
                    snap_point(p, &targets, v.scale(), SNAP_RADIUS)
                } else {
                    p
                };
                if let Some(f) = project.fixture_mut(fixture) {
                    f.set_handle(handle, target.to_array());
                }
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            if response.drag_stopped() {
                state.drag = None;
            }
            return true;
        }
        Some(Drag::FixtureMove { fixture, last }) => {
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let p = v.to_norm(pos);
                if let Some(f) = project.fixture_mut(fixture) {
                    f.translate((p - last).to_array());
                }
                state.drag = Some(Drag::FixtureMove { fixture, last: p });
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            if response.drag_stopped() {
                state.drag = None;
            }
            return true;
        }
        _ => {}
    }
    if state.drag.is_some() {
        return false;
    }
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some((id, handle)) = fixture_hit(project, state, output, v.to_norm(pos), v.scale())
    {
        state.select_fixture(Some(id));
        state.selected_corner = handle;
        return true;
    }
    if let Some(pos) = response.hover_pos()
        && let Some((_, handle)) = fixture_hit(project, state, output, v.to_norm(pos), v.scale())
    {
        ui.ctx().set_cursor_icon(if handle.is_some() {
            CursorIcon::Crosshair
        } else {
            CursorIcon::Grab
        });
        return !response.clicked() && !response.drag_started();
    }
    false
}

/// Keys for a selected fixture: arrows nudge the handle (or the fixture),
/// Tab cycles handles, Delete removes a free point, Esc deselects.
fn fixture_keys(ui: &egui::Ui, project: &mut Project, state: &mut UiState, resolution: [u32; 2]) {
    let Some(id) = state.selected_fixture else {
        return;
    };
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    let (delta, tab, back_tab, escape, delete) = ui.ctx().input_mut(|i| {
        let step = if i.modifiers.shift { 10.0 } else { 1.0 };
        let mut d = Vec2::ZERO;
        if i.key_pressed(Key::ArrowLeft) {
            d.x -= step;
        }
        if i.key_pressed(Key::ArrowRight) {
            d.x += step;
        }
        if i.key_pressed(Key::ArrowUp) {
            d.y -= step;
        }
        if i.key_pressed(Key::ArrowDown) {
            d.y += step;
        }
        (
            d,
            i.consume_key(egui::Modifiers::NONE, Key::Tab),
            i.consume_key(egui::Modifiers::SHIFT, Key::Tab),
            i.consume_key(egui::Modifiers::NONE, Key::Escape),
            i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
        )
    });
    let Some(f) = project.fixture_mut(id) else {
        return;
    };
    let count = f.handles().len();
    if (tab || back_tab) && count > 0 {
        let step = if back_tab { count - 1 } else { 1 };
        state.selected_corner = Some(
            state
                .selected_corner
                .filter(|c| *c < count)
                .map_or(0, |c| (c + step) % count),
        );
    }
    if escape {
        if state.selected_corner.is_some() {
            state.selected_corner = None;
        } else {
            state.select_fixture(None);
        }
        return;
    }
    if delete
        && let Some(i) = state.selected_corner
        && let FixtureShape::Custom { points } = &mut f.shape
        && points.len() > 1
        && i < points.len()
    {
        points.remove(i);
        state.selected_corner = None;
    }
    if delta != Vec2::ZERO {
        let d = delta / Vec2::new(resolution[0].max(1) as f32, resolution[1].max(1) as f32);
        match state.selected_corner.filter(|c| *c < count) {
            Some(i) => {
                let h = f.handles()[i];
                f.set_handle(i, [h[0] + d.x, h[1] + d.y]);
            }
            None => f.translate(d.to_array()),
        }
    }
}

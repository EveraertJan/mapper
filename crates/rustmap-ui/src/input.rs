//! Column 2, input mapping: the selected input whole, with the region every
//! surface takes from it. Pick a region to select its surface; drag its
//! corners or the region itself. Any quad works, so a region can crop,
//! rotate or un-keystone the source. *+ Region* makes a new surface from
//! this input on the selected output.

use egui::{Color32, CursorIcon, Key, Pos2, Rect, Sense, Shape, Stroke};
use rustmap_core::{MediaId, Project, Surface, SurfaceId};
use rustmap_geom::{
    SnapTargets, Vec2, contains, map_uv, nearest_corner, quad_from_arrays, snap_point,
};

use crate::editor::{Drag, ERROR, HANDLE_HALF, HANDLE_HIT, HELP_HEIGHT, SNAP_RADIUS};
use crate::theme::SELECTION;
use crate::{MediaStatus, UiState, ViewData};

/// Regions of other surfaces on this input.
const OTHER: Color32 = Color32::from_rgb(120, 170, 230);

/// What a pointer position hits: a corner of the selected region, or the
/// inside of a region (selected one first).
#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Corner(SurfaceId, usize),
    Inside(SurfaceId),
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
) {
    let Some(media) = state
        .selected_media
        .filter(|m| project.media_item(*m).is_some())
    else {
        centered(
            ui,
            "Add an input in column 1: images, videos or NDI sources.",
        );
        return;
    };
    header(ui, project, state, media);

    let Some(MediaStatus::Ready {
        size, thumbnail, ..
    }) = view.media.get(&media)
    else {
        let missing = project
            .media_item(media)
            .and_then(|m| m.path())
            .is_some_and(|p| !p.exists());
        let text = match view.media.get(&media) {
            Some(MediaStatus::Failed(_)) if missing => {
                "This input's file is missing. Relink it in column 1.".to_owned()
            }
            Some(MediaStatus::Failed(err)) => format!("Can't show this input: {err}"),
            Some(MediaStatus::Waiting(_)) => "Waiting for the NDI source…".to_owned(),
            _ => "Loading…".to_owned(),
        };
        centered(ui, &text);
        return;
    };
    let (size, thumbnail) = (*size, *thumbnail);

    let avail = ui.available_rect_before_wrap();
    let area = Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - HELP_HEIGHT))
        .shrink(12.0);
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    let fit = if area.width() / area.height().max(1.0) > aspect {
        egui::vec2(area.height() * aspect, area.height())
    } else {
        egui::vec2(area.width(), area.width() / aspect)
    };
    if fit.x < 16.0 || fit.y < 16.0 {
        return;
    }
    let fitted = Rect::from_center_size(area.center(), fit);
    let canvas = Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - HELP_HEIGHT));
    let response = ui.allocate_rect(canvas, Sense::click_and_drag());
    let mut zoom = state.input_zoom.get(&media).copied().unwrap_or_default();
    let panning = zoom.interact(ui, &response, fitted);
    let rect = zoom.apply(fitted);
    let to_screen = |p: [f32; 2]| rect.min + egui::vec2(p[0] * rect.width(), p[1] * rect.height());
    let to_norm = |p: Pos2| {
        Vec2::new(
            (p.x - rect.min.x) / rect.width(),
            (p.y - rect.min.y) / rect.height(),
        )
    };
    let scale = Vec2::new(rect.width(), rect.height());

    // Surfaces using this input; the selected one is on top.
    let selected = state
        .selected_surface
        .filter(|id| project.surface(*id).is_some_and(|s| s.media == Some(media)));
    let mut regions: Vec<SurfaceId> = project
        .surfaces
        .iter()
        .filter(|s| s.media == Some(media) && Some(s.id) != selected)
        .map(|s| s.id)
        .collect();
    regions.extend(selected);

    // ---- pointer ----
    if response.is_pointer_button_down_on() {
        state.input_keys = true;
    }
    let (press_origin, alt) = ui.input(|i| (i.pointer.press_origin(), i.modifiers.alt));
    let hit = |project: &Project, p: Vec2| -> Option<Hit> {
        if let Some(id) = selected {
            let q = quad_from_arrays(&project.surface(id)?.input);
            if let Some(c) = nearest_corner(&q, p, scale, HANDLE_HIT) {
                return Some(Hit::Corner(id, c));
            }
        }
        regions.iter().rev().find_map(|id| {
            let q = quad_from_arrays(&project.surface(*id)?.input);
            contains(&q, p).then_some(Hit::Inside(*id))
        })
    };
    let select = |state: &mut UiState, project: &Project, id: SurfaceId| {
        if state.selected_surface != Some(id) {
            state.selected = project.surface(id).map(|s| s.output);
            state.select_surface(Some(id));
        }
    };
    if !panning
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = press_origin
    {
        let p = to_norm(origin);
        state.drag = match hit(project, p) {
            Some(Hit::Corner(surface, corner)) => {
                state.selected_corner = Some(corner);
                Some(Drag::InputCorner { surface, corner })
            }
            Some(Hit::Inside(surface)) => {
                select(state, project, surface);
                project.surface(surface).map(|s| Drag::InputMove {
                    surface,
                    start: s.input,
                    origin: p,
                })
            }
            None => None,
        };
    }
    if !panning
        && response.dragged_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let p = to_norm(pos);
        match state.drag.clone() {
            Some(Drag::InputCorner { surface, corner }) => {
                let target = if state.snapping && !alt {
                    snap_point(p, &SnapTargets::output_frame(), scale, SNAP_RADIUS)
                } else {
                    p
                };
                if let Some(s) = project.surface_mut(surface) {
                    s.input[corner] = target.to_array();
                }
            }
            Some(Drag::InputMove {
                surface,
                start,
                origin,
            }) => {
                let d = p - origin;
                if let Some(s) = project.surface_mut(surface) {
                    s.input = start.map(|c| [c[0] + d.x, c[1] + d.y]);
                }
            }
            _ => {}
        }
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if response.drag_stopped() {
        state.drag = None;
    }
    if !panning
        && response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        match hit(project, to_norm(pos)) {
            Some(Hit::Corner(_, c)) => state.selected_corner = Some(c),
            Some(Hit::Inside(id)) => {
                select(state, project, id);
                state.selected_corner = None;
            }
            None => state.selected_corner = None,
        }
    }
    let hovered = response.hover_pos().and_then(|p| hit(project, to_norm(p)));
    if state.drag.is_none() {
        match hovered {
            Some(Hit::Corner(..)) => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
            Some(Hit::Inside(_)) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
            None => {}
        }
    }

    // ---- keys: nudge by input pixels (after a click in this column) ----
    if state.input_keys
        && let Some(id) = selected
        && !ui.ctx().memory(|m| m.focused().is_some())
    {
        let (delta, tab, back_tab) = ui.ctx().input_mut(|i| {
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
            let back = i.consume_key(egui::Modifiers::SHIFT, Key::Tab);
            let tab = i.consume_key(egui::Modifiers::NONE, Key::Tab);
            (d, tab, back)
        });
        if tab || back_tab {
            let step = if back_tab { 3 } else { 1 };
            state.selected_corner = Some(
                state
                    .selected_corner
                    .filter(|c| *c < 4)
                    .map_or(0, |c| (c + step) % 4),
            );
        }
        if delta != Vec2::ZERO
            && let Some(s) = project.surface_mut(id)
        {
            let d = delta / Vec2::new(size[0].max(1) as f32, size[1].max(1) as f32);
            match state.selected_corner.filter(|c| *c < 4) {
                Some(c) => {
                    s.input[c][0] += d.x;
                    s.input[c][1] += d.y;
                }
                None => {
                    for c in &mut s.input {
                        c[0] += d.x;
                        c[1] += d.y;
                    }
                }
            }
        }
    }

    // ---- drawing ----
    let painter = ui.painter_at(canvas);
    painter.image(
        thumbnail,
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::from_gray(130), // dimmed; regions are outlined
    );
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(70)),
        egui::StrokeKind::Outside,
    );
    let mut folded = false;
    let mut labels: Vec<Pos2> = Vec::new();
    for id in &regions {
        let Some(surface) = project.surface(*id) else {
            continue;
        };
        let is_selected = Some(*id) == selected;
        let q = quad_from_arrays(&surface.input);
        let pts: Vec<Pos2> = surface.input.iter().map(|c| to_screen(*c)).collect();
        let convex = rustmap_geom::is_convex(&q);
        folded |= is_selected && !convex;
        let color = match (convex, is_selected) {
            (false, _) => ERROR,
            (true, true) => SELECTION,
            (true, false) => OTHER,
        };
        let hover = matches!(hovered, Some(Hit::Inside(h)) if h == *id);
        painter.add(Shape::closed_line(
            pts.clone(),
            Stroke::new(
                if is_selected || hover { 2.0 } else { 1.2 },
                if is_selected || hover {
                    color
                } else {
                    color.gamma_multiply(0.75)
                },
            ),
        ));
        if surface.shape != rustmap_core::Shape::Quad {
            let outline: Vec<Pos2> = map_uv(&q, &surface.shape.outline_uv(64))
                .into_iter()
                .map(|p| to_screen(p.to_array()))
                .collect();
            painter.add(Shape::closed_line(
                outline,
                Stroke::new(1.0, color.gamma_multiply(0.6)),
            ));
        }
        if is_selected {
            for (i, p) in pts.iter().enumerate() {
                let active =
                    state.selected_corner == Some(i) || hovered == Some(Hit::Corner(*id, i));
                let r =
                    Rect::from_center_size(*p, egui::vec2(HANDLE_HALF * 2.0, HANDLE_HALF * 2.0));
                painter.rect_filled(r, 1.0, if active { color } else { Color32::BLACK });
                painter.rect_stroke(r, 1.0, Stroke::new(1.5, color), egui::StrokeKind::Middle);
            }
        }
        // Regions often share a frame: stack their labels instead of
        // printing them on top of each other.
        let mut centroid =
            (pts.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.0).to_pos2();
        while labels.iter().any(|l| l.distance(centroid) < 26.0) {
            centroid.y += 30.0;
        }
        labels.push(centroid);
        let output = project
            .output(surface.output)
            .map_or("?", |o| o.name.as_str());
        painter.text(
            centroid,
            egui::Align2::CENTER_CENTER,
            format!("{}\non {output}", surface.name),
            egui::FontId::proportional(12.0),
            if is_selected {
                Color32::WHITE
            } else {
                Color32::from_gray(200)
            },
        );
    }
    if folded {
        painter.text(
            rect.center_bottom() + egui::vec2(0.0, -12.0),
            egui::Align2::CENTER_BOTTOM,
            "Region is folded; showing the full frame until it's fixed",
            egui::FontId::proportional(13.0),
            ERROR,
        );
    }
    if regions.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No surface uses this input yet: + Region",
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
    }
    zoom.overlay(ui, canvas);
    state.input_zoom.insert(media, zoom);
    ui.painter_at(avail).text(
        Pos2::new(avail.min.x + 8.0, avail.max.y - 6.0),
        egui::Align2::LEFT_BOTTOM,
        format!(
            "{} × {} px · click a region to select it · drag corners or the region · Arrows: 1 px (Shift 10) · pinch or {}+scroll: zoom · Space+drag: pan",
            size[0],
            size[1],
            if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }
        ),
        egui::FontId::proportional(12.0),
        Color32::from_gray(140),
    );
}

/// Input name, + Region, and for the selected surface: use this input /
/// full frame.
fn header(ui: &mut egui::Ui, project: &mut Project, state: &mut UiState, media: MediaId) {
    let name = project
        .media_item(media)
        .map(|m| m.name.clone())
        .unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(&name).strong());
        let target = state
            .selected
            .or_else(|| project.outputs.first().map(|o| o.id));
        let target_name = target
            .and_then(|o| project.output(o))
            .map_or_else(|| "a new output".to_owned(), |o| o.name.clone());
        if ui
            .button("+ Region")
            .on_hover_text(format!("New surface showing this input, on {target_name}"))
            .clicked()
        {
            let output = target.unwrap_or_else(|| project.add_output());
            if let Some(id) = project.add_surface(output) {
                if let Some(s) = project.surface_mut(id) {
                    s.media = Some(media);
                    s.name = format!("{name} {}", id.0);
                }
                state.selected = Some(output);
                state.select_surface(Some(id));
                state.input_keys = true;
            }
        }
        let Some(id) = state.selected_surface else {
            return;
        };
        let Some(s) = project.surface_mut(id) else {
            return;
        };
        ui.separator();
        if s.media == Some(media) {
            if ui
                .add_enabled(s.input != Surface::FULL, egui::Button::new("Full frame"))
                .on_hover_text(format!("{} uses the whole input", s.name))
                .clicked()
            {
                s.input = Surface::FULL;
            }
        } else if ui
            .button(format!("Use for {}", s.name))
            .on_hover_text("Show this input on the selected surface")
            .clicked()
        {
            s.media = Some(media);
        }
    });
    ui.separator();
}

fn centered(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.weak(text);
    });
}

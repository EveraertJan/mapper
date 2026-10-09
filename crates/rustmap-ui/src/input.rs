//! The Input view: which part of its media the selected surface shows.
//!
//! The media is shown whole; the surface's input region is a quad on top of
//! it with the same handles and keys as the Output view. Any quad works, so
//! the region can crop, rotate or un-keystone the source.

use egui::{Color32, CursorIcon, Key, Pos2, Rect, Sense, Shape, Stroke};
use rustmap_core::{Project, Surface};
use rustmap_geom::{
    SnapTargets, Vec2, contains, map_uv, nearest_corner, quad_from_arrays, snap_point,
};

use crate::editor::{Drag, ERROR, HANDLE_HALF, HANDLE_HIT, HELP_HEIGHT, SNAP_RADIUS};
use crate::theme::SELECTION;
use crate::{MediaStatus, UiState, ViewData};

pub(crate) fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
) {
    let Some(surface) = state.selected_surface.and_then(|id| project.surface(id)) else {
        centered(
            ui,
            "Select a surface to choose which part of its media it shows.",
        );
        return;
    };
    let id = surface.id;
    let name = surface.name.clone();
    let Some(media) = surface.media else {
        centered(
            ui,
            "This surface shows no media yet. Pick one in the Surface panel.",
        );
        return;
    };
    let Some(MediaStatus::Ready {
        size, thumbnail, ..
    }) = view.media.get(&media)
    else {
        centered(ui, "Waiting for the media to load…");
        return;
    };
    let (size, thumbnail) = (*size, *thumbnail);

    let avail = ui.available_rect_before_wrap();
    let area = Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - HELP_HEIGHT))
        .shrink(16.0);
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    let fit = if area.width() / area.height().max(1.0) > aspect {
        egui::vec2(area.height() * aspect, area.height())
    } else {
        egui::vec2(area.width(), area.width() / aspect)
    };
    if fit.x < 16.0 || fit.y < 16.0 {
        return;
    }
    let rect = Rect::from_center_size(area.center(), fit);
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let to_screen = |p: [f32; 2]| rect.min + egui::vec2(p[0] * rect.width(), p[1] * rect.height());
    let to_norm = |p: Pos2| {
        Vec2::new(
            (p.x - rect.min.x) / rect.width(),
            (p.y - rect.min.y) / rect.height(),
        )
    };
    let scale = Vec2::new(rect.width(), rect.height());

    // ---- pointer ----
    let (press_origin, alt) = ui.input(|i| (i.pointer.press_origin(), i.modifiers.alt));
    let hit = |project: &Project, p: Vec2| -> Option<Option<usize>> {
        let q = quad_from_arrays(&project.surface(id)?.input);
        if let Some(c) = nearest_corner(&q, p, scale, HANDLE_HIT) {
            return Some(Some(c));
        }
        contains(&q, p).then_some(None)
    };
    if response.drag_started()
        && let Some(origin) = press_origin
    {
        let p = to_norm(origin);
        state.drag = match hit(project, p) {
            Some(Some(corner)) => {
                state.selected_corner = Some(corner);
                Some(Drag::InputCorner {
                    surface: id,
                    corner,
                })
            }
            Some(None) => project.surface(id).map(|s| Drag::InputMove {
                surface: id,
                start: s.input,
                origin: p,
            }),
            None => None,
        };
    }
    if response.dragged()
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
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        state.selected_corner = hit(project, to_norm(pos)).flatten();
    }
    let hovered = response.hover_pos().and_then(|p| hit(project, to_norm(p)));
    if state.drag.is_none() {
        match hovered {
            Some(Some(_)) => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
            Some(None) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
            None => {}
        }
    }

    // ---- keys: nudge by media pixels ----
    if !ui.ctx().memory(|m| m.focused().is_some()) {
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
    let painter = ui.painter_at(avail);
    painter.image(
        thumbnail,
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::from_gray(120), // dimmed; the chosen region is outlined
    );
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(70)),
        egui::StrokeKind::Outside,
    );

    let Some(surface) = project.surface(id) else {
        return;
    };
    let q = quad_from_arrays(&surface.input);
    let pts: Vec<Pos2> = surface.input.iter().map(|c| to_screen(*c)).collect();
    let convex = rustmap_geom::is_convex(&q);
    let color = if convex { SELECTION } else { ERROR };
    // The part of the region the surface's shape actually shows.
    let shape_outline: Vec<Pos2> = map_uv(&q, &surface.shape.outline_uv(64))
        .into_iter()
        .map(|p| to_screen(p.to_array()))
        .collect();
    painter.add(Shape::closed_line(pts.clone(), Stroke::new(2.0, color)));
    if surface.shape != rustmap_core::Shape::Quad {
        painter.add(Shape::closed_line(
            shape_outline,
            Stroke::new(1.0, color.gamma_multiply(0.7)),
        ));
    }
    for (i, p) in pts.iter().enumerate() {
        let active = state.selected_corner == Some(i) || hovered == Some(Some(i));
        let r = Rect::from_center_size(*p, egui::vec2(HANDLE_HALF * 2.0, HANDLE_HALF * 2.0));
        painter.rect_filled(r, 1.0, if active { color } else { Color32::BLACK });
        painter.rect_stroke(r, 1.0, Stroke::new(1.5, color), egui::StrokeKind::Middle);
    }
    let centroid = pts.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.0;
    painter.text(
        centroid.to_pos2(),
        egui::Align2::CENTER_CENTER,
        format!("{name} input"),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    if !convex {
        painter.text(
            rect.center_bottom() + egui::vec2(0.0, -12.0),
            egui::Align2::CENTER_BOTTOM,
            "Region is folded; showing the full frame until it's fixed",
            egui::FontId::proportional(13.0),
            ERROR,
        );
    }

    painter.text(
        Pos2::new(avail.min.x + 8.0, avail.max.y - 6.0),
        egui::Align2::LEFT_BOTTOM,
        format!(
            "Input of {name} · {} × {} px · drag corners or the region · Arrows: 1 px (Shift 10) · Alt: no snap",
            size[0], size[1]
        ),
        egui::FontId::proportional(12.0),
        Color32::from_gray(140),
    );
    let full = Rect::from_min_size(
        Pos2::new(avail.max.x - 110.0, avail.max.y - HELP_HEIGHT + 2.0),
        egui::vec2(110.0, 20.0),
    );
    if ui
        .put(full, egui::Button::new("Full frame").small())
        .clicked()
        && let Some(s) = project.surface_mut(id)
    {
        s.input = Surface::FULL;
    }
}

fn centered(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.weak(text);
    });
}

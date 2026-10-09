//! Surface outlines and handles drawn on top of projector windows.

use rustmap_core::{FixtureShape, OutputId, Project, Shape};
use rustmap_geom::{Vec2, is_convex, quad_from_arrays};
use rustmap_render::{OverlayBuilder, Rgba, fit_rect};
use rustmap_ui::warp::{Warp, corner_handle, mask_handles, warp_handles};
use rustmap_ui::{EditLayer, UiState};

const SELECTION: Rgba = [0.9, 0.55, 0.16, 1.0];
const OTHER: Rgba = [1.0, 1.0, 1.0, 0.55];
const ERROR: Rgba = [0.92, 0.35, 0.27, 1.0];
const MASK: Rgba = [0.35, 0.78, 0.98, 1.0];
const LED: Rgba = [0.31, 0.86, 0.63, 1.0];

fn faded(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], c[3] * a]
}

/// Builds guides for one output window. `content` is the output resolution,
/// `window` the window size in pixels.
pub fn build(
    project: &Project,
    output: OutputId,
    content: [u32; 2],
    window: [u32; 2],
    state: &UiState,
) -> OverlayBuilder {
    let mut b = OverlayBuilder::default();
    let [x, y, w, h] = fit_rect(content, window);
    let to_px = |c: Vec2| Vec2::new(x + c.x * w, y + c.y * h);
    let map = |pts: Vec<Vec2>| pts.into_iter().map(to_px).collect::<Vec<_>>();
    // Scale handles with the window so they stay visible on a 4K projector.
    let unit = (w.min(h) / 1080.0).max(0.5);

    for s in project.surfaces_on(output) {
        let selected = state.selected_surface == Some(s.id);
        if !s.visible && !selected {
            continue;
        }
        let warp = Warp::of(s);
        let convex = s.mesh.is_some() || is_convex(&quad_from_arrays(&s.corners));
        // Triangles and circles: outline the visible shape, not the quad.
        let outline = map(warp.outline(s.shape));
        if !selected {
            b.polygon(&outline, 1.0 * unit, if convex { OTHER } else { ERROR });
            continue;
        }

        let color = if convex { SELECTION } else { ERROR };
        let mask_layer = state.layer == EditLayer::Mask;
        for line in warp.grid_lines() {
            let line = map(line);
            for pair in line.windows(2) {
                b.line(pair[0], pair[1], 1.0 * unit, faded(color, 0.4));
            }
        }
        if s.shape != Shape::Quad || s.mesh.is_some() {
            b.polygon(
                &map(warp.outline(Shape::Quad)),
                1.0 * unit,
                faded(color, 0.4),
            );
        }
        b.polygon(&outline, if mask_layer { 1.0 } else { 2.0 } * unit, color);

        if let Some(mask) = &s.mask {
            let line = map(warp.map_polyline(&mask.points, true));
            let alpha = if mask_layer { 1.0 } else { 0.6 };
            b.polygon(
                &line,
                if mask_layer { 2.0 } else { 1.0 } * unit,
                faded(MASK, alpha),
            );
            if mask_layer {
                for (i, p) in mask_handles(s, &warp).into_iter().enumerate() {
                    handle(
                        &mut b,
                        to_px(p),
                        unit,
                        MASK,
                        state.selected_corner == Some(i),
                        1.0,
                    );
                }
            }
        }
        if !mask_layer {
            let corners: Vec<usize> = (0..4).map(|c| corner_handle(s, c)).collect();
            for (i, p) in warp_handles(s).into_iter().enumerate() {
                let size = if corners.contains(&i) { 1.0 } else { 0.75 };
                handle(
                    &mut b,
                    to_px(p),
                    unit,
                    color,
                    state.selected_corner == Some(i),
                    size,
                );
            }
        }
    }

    // LED fixtures: outlines; the selected one also shows its pixels and handles.
    for f in project.fixtures_on(output) {
        let selected = state.selected_fixture == Some(f.id);
        let color = if selected { SELECTION } else { faded(LED, 0.7) };
        match &f.shape {
            FixtureShape::Strip { start, end, .. } => {
                b.line(to_px((*start).into()), to_px((*end).into()), unit, color);
            }
            FixtureShape::Matrix { corners, .. } => {
                let pts: Vec<Vec2> = corners.iter().map(|c| to_px((*c).into())).collect();
                b.polygon(&pts, unit, color);
            }
            FixtureShape::Custom { .. } => {}
        }
        let pixels = f.pixel_positions();
        if selected || matches!(f.shape, FixtureShape::Custom { .. }) {
            for p in pixels.iter().take(4096) {
                b.square(to_px((*p).into()), 1.5 * unit, color);
            }
        }
        if selected {
            for (i, h) in f.handles().into_iter().enumerate() {
                handle(
                    &mut b,
                    to_px(h.into()),
                    unit,
                    color,
                    state.selected_corner == Some(i),
                    1.0,
                );
            }
        }
    }
    b
}

fn handle(b: &mut OverlayBuilder, p: Vec2, unit: f32, color: Rgba, active: bool, size: f32) {
    if active {
        b.square(p, 9.0 * unit * size, color);
    } else {
        b.square(p, 7.0 * unit * size, [0.0, 0.0, 0.0, 0.8]);
        b.square_outline(p, 7.0 * unit * size, 2.0 * unit, color);
    }
}

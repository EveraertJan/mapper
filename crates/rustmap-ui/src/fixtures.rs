//! LED fixtures: the list, the inspector, and their handles in the editor.

use egui::{Color32, Pos2, Rect, Shape, Stroke};
use rustmap_core::led::{LightId, conflicts_for};
use rustmap_core::{
    ColorOrder, Corner, FIXTURE_MAX_PIXELS, Fixture, FixtureId, FixtureShape, OutputId, Project,
    Protocol,
};
use rustmap_geom::{Vec2, contains, quad_from_arrays};

use crate::UiState;
use crate::editor::{HANDLE_HALF, HANDLE_HIT, View};
use crate::theme::SELECTION;

/// Color of LED fixtures in the editor.
pub(crate) const LED: Color32 = Color32::from_rgb(80, 220, 160);

pub(crate) fn list(ui: &mut egui::Ui, project: &mut Project, state: &mut UiState) {
    let Some(output) = state.selected else { return };
    ui.horizontal(|ui| {
        ui.strong("LED FIXTURES");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("+", |ui| {
                for (label, shape) in [
                    ("Strip", Fixture::default_strip()),
                    ("Matrix", Fixture::default_matrix()),
                ] {
                    if ui.button(label).clicked()
                        && let Some(id) = project.add_fixture(output, shape)
                    {
                        state.select_fixture(Some(id));
                    }
                }
            })
            .response
            .on_hover_text("Add an LED strip or matrix");
        });
    });
    ui.separator();

    let ids: Vec<_> = project.fixtures_on(output).map(|f| f.id).collect();
    if ids.is_empty() {
        ui.weak("No LED fixtures on this output.");
    }
    let mut remove = None;
    for id in ids {
        let Some(f) = project.fixture_mut(id) else {
            continue;
        };
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.enabled, "")
                .on_hover_text("Send this fixture (off: its channels stay at zero)");
            let selected = state.selected_fixture == Some(id);
            if ui.selectable_label(selected, &f.name).clicked() {
                state.select_fixture(Some(id));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("🗑")
                    .on_hover_text("Remove fixture")
                    .clicked()
                {
                    remove = Some(id);
                }
            });
        });
    }
    if let Some(id) = remove {
        project.remove_fixture(id);
    }
}

/// Project-wide output switch and rate for all lights (LED and DMX).
pub(crate) fn send_settings(ui: &mut egui::Ui, project: &mut Project) {
    ui.horizontal(|ui| {
        let label = if project.led.send {
            egui::RichText::new("Sending").color(Color32::WHITE)
        } else {
            egui::RichText::new("Send to lights")
        };
        let button = egui::Button::selectable(project.led.send, label);
        if ui
            .add_enabled(project.has_lights(), button)
            .on_hover_text("Send all LED and DMX fixtures over Art-Net / sACN. Saved with the show, so a production file starts sending.")
            .clicked()
        {
            project.led.send = !project.led.send;
        }
        ui.add(
            egui::DragValue::new(&mut project.led.fps)
                .range(1.0..=120.0)
                .max_decimals(0)
                .suffix(" fps"),
        )
        .on_hover_text("Packets per second; most controllers want 30–44");
    });
}

pub(crate) fn inspector(ui: &mut egui::Ui, project: &mut Project, state: &mut UiState) {
    let Some(id) = state.selected_fixture else {
        return;
    };
    let conflicts = conflicts_for(project, LightId::Led(id));
    let Some(f) = project.fixture_mut(id) else {
        return;
    };

    egui::Grid::new("fixture props")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut f.name);
            ui.end_row();

            ui.label("Send");
            ui.checkbox(&mut f.enabled, "");
            ui.end_row();

            shape_editor(ui, f, state);

            ui.label("Color order");
            egui::ComboBox::from_id_salt("color order")
                .selected_text(f.color_order.label())
                .show_ui(ui, |ui| {
                    for order in ColorOrder::ALL {
                        ui.selectable_value(&mut f.color_order, order, order.label());
                    }
                })
                .response
                .on_hover_text("Channel order your LEDs expect, e.g. GRB for WS2812");
            ui.end_row();

            ui.label("Brightness");
            ui.add(egui::Slider::new(&mut f.brightness, 0.0..=1.0));
            ui.end_row();

            ui.label("Sampling");
            ui.checkbox(&mut f.smooth, "Average area")
                .on_hover_text("Average the image around each LED instead of one point, so fine detail doesn't flicker");
            ui.end_row();
        });

    ui.add_space(8.0);
    patch_editor(ui, &mut f.patch, "led patch");

    let pixels = f.pixel_count();
    let channels = pixels * f.color_order.channels();
    let range = match f.universe_range() {
        Some((a, b)) if a == b => format!("universe {a}"),
        Some((a, b)) => format!("universes {a}–{b}"),
        None => "no universes".to_owned(),
    };
    ui.add_space(4.0);
    ui.weak(format!("{pixels} pixels · {channels} channels · {range}"));
    if f.addresses().len() < pixels {
        ui.colored_label(
            crate::editor::ERROR,
            "Runs past the last universe; the rest isn't sent.",
        );
    }
    for c in conflicts {
        ui.colored_label(crate::editor::ERROR, c);
    }
}

fn shape_editor(ui: &mut egui::Ui, f: &mut Fixture, state: &mut UiState) {
    let max = FIXTURE_MAX_PIXELS as u32;
    let mut to_points = false;
    match &mut f.shape {
        FixtureShape::Strip { pixels, .. } => {
            ui.label("Pixels");
            ui.add(egui::DragValue::new(pixels).range(1..=max));
            ui.end_row();
        }
        FixtureShape::Matrix {
            cols, rows, wiring, ..
        } => {
            ui.label("Size");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(cols).range(1..=512).prefix("cols "));
                ui.add(egui::DragValue::new(rows).range(1..=512).prefix("rows "));
            });
            ui.end_row();

            ui.label("Wiring");
            ui.vertical(|ui| {
                egui::ComboBox::from_id_salt("wiring start")
                    .selected_text(format!("Starts {}", wiring.start.label().to_lowercase()))
                    .show_ui(ui, |ui| {
                        for c in Corner::ALL {
                            ui.selectable_value(&mut wiring.start, c, c.label());
                        }
                    });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut wiring.vertical, false, "Rows");
                    ui.selectable_value(&mut wiring.vertical, true, "Columns");
                });
                ui.checkbox(&mut wiring.serpentine, "Zigzag")
                    .on_hover_text("Every other row (or column) runs back the other way");
            });
            ui.end_row();
        }
        FixtureShape::Custom { points } => {
            ui.label("Pixels");
            ui.label(format!("{} free points", points.len()));
            ui.end_row();
        }
    }
    if over_cap(f) {
        ui.label("");
        ui.colored_label(
            crate::editor::ERROR,
            format!("Only the first {FIXTURE_MAX_PIXELS} pixels are used."),
        );
        ui.end_row();
    }
    if !matches!(f.shape, FixtureShape::Custom { .. }) {
        ui.label("");
        to_points = ui
            .add_enabled(f.pixel_count() <= 1024, egui::Button::new("Free points"))
            .on_hover_text("Turn every pixel into its own handle, for odd layouts")
            .on_disabled_hover_text("Only for fixtures up to 1024 pixels")
            .clicked();
        ui.end_row();
    }
    if to_points {
        f.shape = FixtureShape::Custom {
            points: f.pixel_positions(),
        };
        state.selected_corner = None;
    }
}

/// More pixels than a fixture can have (the rest are ignored).
fn over_cap(f: &Fixture) -> bool {
    match &f.shape {
        FixtureShape::Matrix { cols, rows, .. } => {
            *cols as usize * *rows as usize > FIXTURE_MAX_PIXELS
        }
        _ => false,
    }
}

/// Fixture under `p`: a handle of the selected fixture first, then any
/// fixture's handle, then a fixture body (strip line or matrix area).
pub(crate) fn hit(
    project: &Project,
    state: &UiState,
    output: OutputId,
    p: Vec2,
    scale: Vec2,
) -> Option<(FixtureId, Option<usize>)> {
    let near = |a: [f32; 2]| ((Vec2::from(a) - p) * scale).length() <= HANDLE_HIT;
    let nearest = |f: &Fixture| {
        f.handles()
            .into_iter()
            .enumerate()
            .filter(|(_, h)| near(*h))
            .min_by(|a, b| {
                let d = |h: [f32; 2]| ((Vec2::from(h) - p) * scale).length();
                d(a.1).total_cmp(&d(b.1))
            })
            .map(|(i, _)| i)
    };
    if let Some(f) = state.selected_fixture.and_then(|id| project.fixture(id))
        && let Some(i) = nearest(f)
    {
        return Some((f.id, Some(i)));
    }
    let fixtures: Vec<&Fixture> = project.fixtures_on(output).collect();
    for f in fixtures.iter().rev() {
        if let Some(i) = nearest(f) {
            return Some((f.id, Some(i)));
        }
    }
    fixtures
        .iter()
        .rev()
        .find(|f| on_body(f, p, scale))
        .map(|f| (f.id, None))
}

fn on_body(f: &Fixture, p: Vec2, scale: Vec2) -> bool {
    match &f.shape {
        FixtureShape::Strip { start, end, .. } => {
            let (a, b, q) = (
                Vec2::from(*start) * scale,
                Vec2::from(*end) * scale,
                p * scale,
            );
            let ab = b - a;
            let t = ((q - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (a + ab * t - q).length() <= HANDLE_HIT * 0.6
        }
        FixtureShape::Matrix { corners, .. } => contains(&quad_from_arrays(corners), p),
        FixtureShape::Custom { .. } => false,
    }
}

/// Draws every fixture on `output`: outline, pixels, and the first pixel
/// marked so the wiring direction is visible.
pub(crate) fn draw(
    painter: &egui::Painter,
    project: &Project,
    state: &UiState,
    output: OutputId,
    v: View,
    hovered: Option<(FixtureId, Option<usize>)>,
) {
    for f in project.fixtures_on(output) {
        let selected = state.selected_fixture == Some(f.id);
        let hover = hovered.is_some_and(|(id, _)| id == f.id);
        let mut color = if selected { SELECTION } else { LED };
        if !f.enabled {
            color = color.gamma_multiply(0.4);
        }
        let width = if selected || hover { 2.0 } else { 1.0 };
        let pixels: Vec<Pos2> = f
            .pixel_positions()
            .iter()
            .map(|p| v.to_screen(*p))
            .collect();

        match &f.shape {
            FixtureShape::Strip { start, end, .. } => {
                painter.line_segment(
                    [v.to_screen(*start), v.to_screen(*end)],
                    Stroke::new(width, color),
                );
            }
            FixtureShape::Matrix { corners, .. } => {
                let pts: Vec<Pos2> = corners.iter().map(|c| v.to_screen(*c)).collect();
                painter.add(Shape::closed_line(pts, Stroke::new(width, color)));
                // The wiring path, faintly, so the order is visible.
                if selected && pixels.len() <= 4096 {
                    painter.add(Shape::line(
                        pixels.clone(),
                        Stroke::new(1.0, color.gamma_multiply(0.35)),
                    ));
                }
            }
            FixtureShape::Custom { .. } => {
                if selected {
                    painter.add(Shape::line(
                        pixels.clone(),
                        Stroke::new(1.0, color.gamma_multiply(0.5)),
                    ));
                }
            }
        }

        // Pixels as dots, unless they'd be a smear.
        let spacing = pixels
            .windows(2)
            .take(64)
            .map(|w| w[0].distance(w[1]))
            .fold(f32::MAX, f32::min);
        if spacing >= 4.0 || pixels.len() == 1 {
            let r = (spacing / 4.0).clamp(1.0, 3.0);
            for p in &pixels {
                painter.circle_filled(*p, r, color.gamma_multiply(0.8));
            }
        }
        if let Some(first) = pixels.first() {
            painter.circle_stroke(*first, 5.0, Stroke::new(1.5, color));
        }

        if selected {
            for (i, h) in f.handles().iter().enumerate() {
                let active = state.selected_corner == Some(i) || hovered == Some((f.id, Some(i)));
                let r = Rect::from_center_size(
                    v.to_screen(*h),
                    egui::vec2(HANDLE_HALF * 2.0, HANDLE_HALF * 2.0),
                );
                painter.rect_filled(r, 1.0, if active { color } else { Color32::BLACK });
                painter.rect_stroke(r, 1.0, Stroke::new(1.5, color), egui::StrokeKind::Middle);
            }
            if let Some(first) = pixels.first() {
                painter.text(
                    *first + egui::vec2(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    &f.name,
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
        }
    }
}

/// Protocol, destination, universe and start channel of any light.
pub(crate) fn patch_editor(ui: &mut egui::Ui, patch: &mut rustmap_core::Patch, salt: &str) {
    ui.strong("Patch");
    ui.separator();
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Protocol");
            ui.horizontal(|ui| {
                for p in [Protocol::ArtNet, Protocol::Sacn] {
                    if ui.selectable_label(patch.protocol == p, p.label()).clicked() {
                        patch.protocol = p;
                    }
                }
            });
            patch.universe = patch
                .universe
                .clamp(patch.protocol.first_universe(), patch.protocol.last_universe());
            ui.end_row();

            ui.label("Destination");
            let hint = match patch.protocol {
                Protocol::ArtNet => "broadcast",
                Protocol::Sacn => "multicast",
            };
            ui.add(
                egui::TextEdit::singleline(&mut patch.destination)
                    .hint_text(hint)
                    .desired_width(130.0),
            )
            .on_hover_text(
                "IP address of the controller or node. Empty: broadcast (Art-Net) or multicast (sACN).",
            );
            ui.end_row();

            ui.label("Universe");
            ui.add(
                egui::DragValue::new(&mut patch.universe)
                    .range(patch.protocol.first_universe()..=patch.protocol.last_universe()),
            );
            ui.end_row();

            ui.label("Start channel");
            ui.add(egui::DragValue::new(&mut patch.channel).range(1..=512));
            ui.end_row();
        });
}

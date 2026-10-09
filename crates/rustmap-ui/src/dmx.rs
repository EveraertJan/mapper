//! DMX monitor: the channel values going out on each universe.

use egui::{Color32, Pos2, Rect, Sense, Stroke};
use rustmap_core::Project;
use rustmap_core::led::{channel_owners, describe_owner};

use crate::{UiState, ViewData};

const COLS: usize = 32;
const ROWS: usize = 16;

pub(crate) fn show(ui: &mut egui::Ui, project: &Project, state: &mut UiState, view: &ViewData<'_>) {
    let dmx = view.dmx;
    ui.horizontal_wrapped(|ui| {
        if project.led.send {
            ui.label(format!(
                "Sending at {:.0} fps · {} packets",
                project.led.fps, dmx.packets
            ));
        } else {
            ui.weak("Not sending (Send to lights, under the fixture lists or in the Console). Showing what would go out.");
        }
    });
    for err in &dmx.errors {
        ui.colored_label(crate::editor::ERROR, err);
    }
    if dmx.universes.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.weak(
                "No lights yet. Add LED fixtures or DMX fixtures with + in the lists on the left.",
            )
        });
        return;
    }

    if state
        .dmx_universe
        .as_ref()
        .is_none_or(|k| !dmx.universes.iter().any(|(u, _)| u == k))
    {
        state.dmx_universe = dmx.universes.first().map(|(k, _)| k.clone());
    }
    ui.horizontal_wrapped(|ui| {
        for (key, _) in &dmx.universes {
            let selected = state.dmx_universe.as_ref() == Some(key);
            if ui.selectable_label(selected, key.to_string()).clicked() {
                state.dmx_universe = Some(key.clone());
            }
        }
    });
    ui.add_space(6.0);

    let Some((key, data)) = dmx
        .universes
        .iter()
        .find(|(k, _)| Some(k) == state.dmx_universe.as_ref())
    else {
        return;
    };

    // 32 × 16 grid of channels, filling the space.
    let avail = ui.available_rect_before_wrap();
    let cell = ((avail.width() / COLS as f32).min((avail.height() - 24.0) / ROWS as f32)).floor();
    if cell < 6.0 {
        return;
    }
    let grid = Rect::from_min_size(
        avail.min,
        egui::vec2(cell * COLS as f32, cell * ROWS as f32),
    );
    let response = ui.allocate_rect(grid, Sense::hover());
    let painter = ui.painter_at(grid);
    let owners = channel_owners(project);
    let font = egui::FontId::monospace((cell * 0.32).clamp(7.0, 12.0));
    for ch in 0..512 {
        let (c, r) = (ch % COLS, ch / COLS);
        let rect = Rect::from_min_size(
            grid.min + egui::vec2(c as f32 * cell, r as f32 * cell),
            egui::vec2(cell, cell),
        )
        .shrink(0.5);
        let value = data[ch];
        let owned = owners.contains_key(&(key.clone(), ch));
        let fill = if owned {
            Color32::from_gray(value / 2 + 20)
        } else {
            Color32::from_gray(12)
        };
        painter.rect_filled(rect, 1.0, fill);
        if cell >= 22.0 && owned {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                value.to_string(),
                font.clone(),
                if value > 140 {
                    Color32::BLACK
                } else {
                    Color32::from_gray(200)
                },
            );
        }
    }
    // Hover: channel, value, and which fixture pixel drives it.
    if let Some(pos) = response.hover_pos() {
        let c = ((pos.x - grid.min.x) / cell) as usize;
        let r = ((pos.y - grid.min.y) / cell) as usize;
        let ch = r * COLS + c;
        if c < COLS && ch < 512 {
            let rect = Rect::from_min_size(
                grid.min + egui::vec2(c as f32 * cell, r as f32 * cell),
                egui::vec2(cell, cell),
            );
            painter.rect_stroke(
                rect,
                1.0,
                Stroke::new(1.5, crate::SELECTION),
                egui::StrokeKind::Inside,
            );
            let who = owners
                .get(&(key.clone(), ch))
                .map(|o| format!(" · {}", describe_owner(project, *o)))
                .unwrap_or_else(|| " · unused".to_owned());
            response.on_hover_text_at_pointer(format!("Channel {} = {}{who}", ch + 1, data[ch]));
        }
    }
    ui.painter().text(
        Pos2::new(grid.min.x, grid.max.y + 4.0),
        egui::Align2::LEFT_TOP,
        "Rows of 32 channels. Bright = high value; dark gray = not patched.",
        egui::FontId::proportional(12.0),
        Color32::from_gray(140),
    );
}

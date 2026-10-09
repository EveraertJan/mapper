use std::collections::VecDeque;

use crate::{UiAction, ViewData};

const HISTORY: usize = 240;

/// Rolling frame-time history for the status bar.
#[derive(Debug, Default)]
pub struct FrameStats {
    frame_ms: VecDeque<f32>,
    /// Frames where at least one output window was presented.
    pub presented_outputs: usize,
}

impl FrameStats {
    pub fn push(&mut self, frame_ms: f32) {
        if self.frame_ms.len() == HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(frame_ms);
    }

    pub fn average_ms(&self) -> f32 {
        if self.frame_ms.is_empty() {
            return 0.0;
        }
        self.frame_ms.iter().sum::<f32>() / self.frame_ms.len() as f32
    }

    pub fn worst_ms(&self) -> f32 {
        self.frame_ms.iter().copied().fold(0.0, f32::max)
    }

    pub fn history(&self) -> impl Iterator<Item = f32> + '_ {
        self.frame_ms.iter().copied()
    }
}

pub(crate) fn status_bar(ui: &mut egui::Ui, view: &ViewData<'_>, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        let avg = view.stats.average_ms();
        let fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
        ui.monospace(format!(
            "{fps:5.1} fps  {avg:5.2} ms  worst {:5.2} ms",
            view.stats.worst_ms()
        ));
        sparkline(ui, view.stats);
        ui.separator();

        let mut vsync = view.vsync;
        if ui
            .checkbox(&mut vsync, "VSync")
            .on_hover_text("Off only for benchmarking; projectors should always sync.")
            .changed()
        {
            actions.push(UiAction::SetVsync(vsync));
        }

        let mut profiler = view.profiler_enabled;
        if ui
            .checkbox(&mut profiler, "Profiler")
            .on_hover_text("Streams to puffin_viewer on 127.0.0.1:8585")
            .changed()
        {
            actions.push(UiAction::SetProfiler(profiler));
        }

        ui.separator();
        match &view.ndi.runtime {
            Ok(version) => {
                ui.weak(format!("NDI ✔ {} source(s)", view.ndi.sources.len()))
                    .on_hover_text(version);
            }
            Err(err) => {
                ui.weak("NDI not installed").on_hover_text(err);
            }
        }

        let dmx = view.dmx;
        if !dmx.universes.is_empty() {
            ui.separator();
            let text = if dmx.packets > 0 {
                format!(
                    "Lights {} univ. · {} pkts",
                    dmx.universes.len(),
                    dmx.packets
                )
            } else {
                format!("Lights {} univ.", dmx.universes.len())
            };
            let label = if dmx.errors.is_empty() {
                ui.weak(text)
            } else {
                ui.colored_label(
                    crate::editor::ERROR,
                    format!("{text} · {} issue(s)", dmx.errors.len()),
                )
            };
            label.on_hover_text(if dmx.errors.is_empty() {
                "See the DMX tab for channel values".to_owned()
            } else {
                dmx.errors.join("\n")
            });
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(view.adapter);
        });
    });
}

/// Tiny frame-time graph. The dashed line is 16.7 ms (60 fps).
fn sparkline(ui: &mut egui::Ui, stats: &FrameStats) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(HISTORY as f32 * 0.75, 18.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);

    let max_ms = 33.4_f32;
    let y_for = |ms: f32| rect.bottom() - (ms / max_ms).clamp(0.0, 1.0) * rect.height();
    painter.hline(
        rect.x_range(),
        y_for(16.7),
        egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
    );
    let step = rect.width() / HISTORY as f32;
    let points: Vec<egui::Pos2> = stats
        .history()
        .enumerate()
        .map(|(i, ms)| egui::pos2(rect.left() + i as f32 * step, y_for(ms)))
        .collect();
    let color = if stats.worst_ms() > 20.0 {
        egui::Color32::from_rgb(230, 120, 40)
    } else {
        egui::Color32::from_rgb(90, 200, 120)
    };
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_averages() {
        let mut s = FrameStats::default();
        for _ in 0..(HISTORY + 50) {
            s.push(10.0);
        }
        s.push(30.0);
        assert_eq!(s.history().count(), HISTORY);
        assert!((s.worst_ms() - 30.0).abs() < f32::EPSILON);
        assert!(s.average_ms() > 10.0 && s.average_ms() < 11.0);
    }
}

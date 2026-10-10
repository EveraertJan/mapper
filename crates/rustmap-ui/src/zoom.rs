//! Zoom and pan for the input and output canvases.
//!
//! - Pinch (trackpad) or Cmd/Ctrl + scroll: zoom around the pointer.
//! - Scroll / two-finger swipe while zoomed: pan.
//! - Middle-drag, or Space + drag: pan.
//! - *Fit* (top right while zoomed) or double-click with the middle button:
//!   back to the whole frame.

use egui::{Key, PointerButton, Pos2, Rect, Response, Vec2};

const MAX_ZOOM: f32 = 40.0;

/// How a canvas is zoomed. `pan` moves the content's center away from the
/// center of its fitted rect, in screen points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasZoom {
    pub zoom: f32,
    pub pan: Vec2,
}

impl Default for CanvasZoom {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl CanvasZoom {
    pub fn is_fit(&self) -> bool {
        self.zoom <= 1.0 && self.pan == Vec2::ZERO
    }

    /// Where the content is drawn, given where it fits unzoomed.
    pub fn apply(&self, fitted: Rect) -> Rect {
        Rect::from_center_size(fitted.center() + self.pan, fitted.size() * self.zoom)
    }

    /// Handles zoom and pan over the canvas. Returns true while the pointer
    /// is panning, so the caller leaves editing alone.
    pub fn interact(&mut self, ui: &egui::Ui, response: &Response, fitted: Rect) -> bool {
        let (zoom_delta, scroll, pointer, space, middle_double) = ui.input(|i| {
            (
                i.zoom_delta(),
                i.smooth_scroll_delta,
                i.pointer.hover_pos(),
                i.key_down(Key::Space),
                i.pointer.button_double_clicked(PointerButton::Middle),
            )
        });
        let mut panning = false;
        if response.hovered() {
            if middle_double {
                *self = Self::default();
                return false;
            }
            if zoom_delta != 1.0
                && let Some(p) = pointer
            {
                self.zoom_at(fitted, p, self.zoom * zoom_delta);
            } else if scroll != Vec2::ZERO && self.zoom > 1.0 {
                self.pan += scroll;
            }
        }
        if response.dragged_by(PointerButton::Middle)
            || (space && response.dragged_by(PointerButton::Primary))
        {
            self.pan += response.drag_delta();
            panning = true;
        }
        if space && response.hovered() {
            ui.ctx().set_cursor_icon(if panning {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }
        // Space held over the canvas: a press starts a pan, not an edit.
        panning |= space && response.is_pointer_button_down_on();
        self.clamp(fitted);
        panning
    }

    /// Zooms to `zoom`, keeping the content under `p` where it is.
    fn zoom_at(&mut self, fitted: Rect, p: Pos2, zoom: f32) {
        let zoom = zoom.clamp(1.0, MAX_ZOOM);
        let center = fitted.center() + self.pan;
        let new_center = p - (p - center) * (zoom / self.zoom);
        self.zoom = zoom;
        self.pan = new_center - fitted.center();
    }

    /// Keeps some of the content on screen; at 1× it snaps back to fit.
    fn clamp(&mut self, fitted: Rect) {
        if self.zoom <= 1.0 {
            *self = Self::default();
            return;
        }
        let half = fitted.size() * self.zoom * 0.5;
        let limit = half + fitted.size() * 0.5 - Vec2::splat(40.0);
        self.pan = self.pan.clamp(-limit, limit);
    }

    /// Zoom level and a Fit button in the top-right corner of `canvas`.
    pub fn overlay(&mut self, ui: &mut egui::Ui, canvas: Rect) {
        if self.is_fit() {
            return;
        }
        let r = Rect::from_min_size(
            Pos2::new(canvas.max.x - 118.0, canvas.min.y + 6.0),
            egui::vec2(112.0, 22.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:.0}%", self.zoom * 100.0))
                        .monospace()
                        .background_color(egui::Color32::from_black_alpha(160)),
                );
                if ui
                    .small_button("Fit")
                    .on_hover_text("Show the whole frame (or middle double-click)")
                    .clicked()
                {
                    *self = Self::default();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let fitted = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0));
        let mut z = CanvasZoom::default();
        let p = Pos2::new(100.0, 50.0);
        let before = z.apply(fitted);
        let uv = (p - before.min) / before.size();
        z.zoom_at(fitted, p, 4.0);
        let after = z.apply(fitted);
        let uv2 = (p - after.min) / after.size();
        assert!((uv - uv2).length() < 1e-5);
        assert_eq!(z.zoom, 4.0);
    }

    #[test]
    fn back_to_one_is_fit() {
        let fitted = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0));
        let mut z = CanvasZoom::default();
        z.zoom_at(fitted, Pos2::new(10.0, 10.0), 3.0);
        assert!(!z.is_fit());
        z.zoom_at(fitted, Pos2::new(10.0, 10.0), 0.5);
        z.clamp(fitted);
        assert!(z.is_fit());
    }
}

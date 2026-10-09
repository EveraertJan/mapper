/// Accent used for selections everywhere, including guides on the projector.
pub const SELECTION: egui::Color32 = egui::Color32::from_rgb(230, 140, 40);

/// Dark, low-glare look: control UIs get used in dark venues.
pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_gray(22);
    visuals.window_fill = egui::Color32::from_gray(26);
    visuals.extreme_bg_color = egui::Color32::from_gray(10);
    visuals.selection.bg_fill = egui::Color32::from_rgb(200, 120, 30);
    visuals.hyperlink_color = egui::Color32::from_rgb(230, 150, 60);
    ctx.set_visuals(visuals);
}

use rustmap_core::{MediaId, MediaSource, Playback, Project};

use crate::{MediaStatus, UiAction, UiState, VideoTime, ViewData};

const THUMB_HEIGHT: f32 = 34.0;

pub(crate) fn list(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    ui.horizontal(|ui| {
        ui.strong("MEDIA");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("+", |ui| {
                if ui.button("Images or videos…").clicked() {
                    actions.push(UiAction::ImportImages { assign_to: None });
                    ui.close();
                }
                ui.menu_button("NDI source", |ui| ndi_menu(ui, project, state, view));
            })
            .response
            .on_hover_text("Add media");
        });
    });
    ui.separator();

    if project.media.is_empty() {
        ui.weak("Add images, videos or NDI sources with +, or drop files on this window.");
        return;
    }

    let selected_surface = state.selected_surface;
    let mut remove = None;
    let mut assign = None;
    let mut playback_edits: Vec<(MediaId, Playback)> = Vec::new();
    for media in &project.media {
        ui.horizontal(|ui| {
            let status = view.media.get(&media.id);
            match status {
                Some(MediaStatus::Ready {
                    size, thumbnail, ..
                }) => {
                    let w = THUMB_HEIGHT * size[0] as f32 / size[1].max(1) as f32;
                    ui.add(egui::Image::new((
                        *thumbnail,
                        egui::vec2(w.min(THUMB_HEIGHT * 2.0), THUMB_HEIGHT),
                    )))
                    .on_hover_text(format!("{} × {}", size[0], size[1]));
                }
                Some(MediaStatus::Failed(_) | MediaStatus::Waiting(_)) if media.is_live() => {
                    ui.add_sized([THUMB_HEIGHT, THUMB_HEIGHT], egui::Label::new("NDI"));
                }
                Some(MediaStatus::Failed(_)) => {
                    ui.add_sized([THUMB_HEIGHT, THUMB_HEIGHT], egui::Label::new("⚠"));
                }
                _ => {
                    ui.add_sized([THUMB_HEIGHT, THUMB_HEIGHT], egui::Spinner::new());
                }
            }
            ui.vertical(|ui| {
                ui.label(&media.name).on_hover_text(media.origin());
                match status {
                    Some(MediaStatus::Failed(err)) => {
                        ui.horizontal(|ui| {
                            ui.colored_label(egui::Color32::from_rgb(235, 90, 70), "Missing")
                                .on_hover_text(err);
                            if media.path().is_some()
                                && ui
                                    .small_button("Relink…")
                                    .on_hover_text("Pick the file; other missing files in that folder are found too")
                                    .clicked()
                            {
                                actions.push(UiAction::RelinkMedia(media.id));
                            }
                        });
                    }
                    Some(MediaStatus::Waiting(msg)) => {
                        ui.weak(format!("NDI · {msg}"));
                    }
                    Some(MediaStatus::Ready {
                        detail: Some(d), ..
                    }) => {
                        let kind = if media.is_video() { "Video" } else { "NDI" };
                        ui.weak(format!("{kind} · {d}"));
                    }
                    _ => {}
                }
                if let MediaSource::Video { playback, .. } = &media.source {
                    let time = view.video.get(&media.id).copied().unwrap_or_default();
                    if let Some(new) = transport(ui, media.id, *playback, time, actions) {
                        playback_edits.push((media.id, new));
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("🗑").on_hover_text("Remove media").clicked() {
                    remove = Some(media.id);
                }
                if ui
                    .add_enabled(selected_surface.is_some(), egui::Button::new("Use").small())
                    .on_hover_text("Show on the selected surface")
                    .on_disabled_hover_text("Select a surface first")
                    .clicked()
                {
                    assign = Some(media.id);
                }
            });
        });
    }
    for (id, new) in playback_edits {
        if let Some(MediaSource::Video { playback, .. }) = project
            .media
            .iter_mut()
            .find(|m| m.id == id)
            .map(|m| &mut m.source)
        {
            *playback = new;
        }
    }
    if let Some(id) = remove {
        project.remove_media(id);
    }
    if let Some(media) = assign
        && let Some(surface) = selected_surface.and_then(|s| project.surface_mut(s))
    {
        surface.media = Some(media);
    }
}

/// Lists NDI sources on the network; picking one adds it as media (and puts
/// it on the selected surface, or a new quad).
pub(crate) fn ndi_menu(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
) {
    match &view.ndi.runtime {
        Err(err) => {
            ui.label("NDI is not available.");
            ui.weak(err);
            ui.hyperlink_to("Get NDI Tools", "https://ndi.video/tools/");
        }
        Ok(_) if view.ndi.sources.is_empty() => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching the network…");
            });
        }
        Ok(_) => {
            for source in &view.ndi.sources {
                if ui.button(source).clicked() {
                    let media = project.add_ndi(source);
                    let target = state
                        .selected_surface
                        .filter(|id| project.surface(*id).is_some_and(|s| s.media.is_none()))
                        .or_else(|| state.selected.and_then(|o| project.add_surface(o)));
                    if let Some(surface) = target.and_then(|id| project.surface_mut(id)) {
                        surface.media = Some(media);
                        let id = surface.id;
                        state.select_surface(Some(id));
                    }
                    ui.close();
                }
            }
        }
    }
}

/// Play/pause, loop, speed and a scrub bar for one video.
fn transport(
    ui: &mut egui::Ui,
    id: MediaId,
    mut playback: Playback,
    time: VideoTime,
    actions: &mut Vec<UiAction>,
) -> Option<Playback> {
    let before = playback;
    ui.horizontal(|ui| {
        let label = if playback.paused { "Play" } else { "Pause" };
        if ui.small_button(label).clicked() {
            playback.paused = !playback.paused;
        }
        if ui
            .small_button("|<")
            .on_hover_text("Back to start")
            .clicked()
        {
            actions.push(UiAction::SeekVideo(id, 0.0));
        }
        ui.toggle_value(&mut playback.looping, "Loop");
        ui.add(
            egui::DragValue::new(&mut playback.speed)
                .range(0.0..=8.0)
                .speed(0.01)
                .max_decimals(2)
                .suffix("x"),
        )
        .on_hover_text("Playback speed");
    });
    if time.duration > 0.0 {
        ui.horizontal(|ui| {
            let mut pos = time.position;
            let slider = ui.add(
                egui::Slider::new(&mut pos, 0.0..=time.duration)
                    .show_value(false)
                    .trailing_fill(true),
            );
            if slider.changed() {
                actions.push(UiAction::SeekVideo(id, pos));
            }
            ui.monospace(format!(
                "{} / {}",
                clock(time.position),
                clock(time.duration)
            ));
        });
    }
    (playback != before).then_some(playback)
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0);
    format!("{:02}:{:04.1}", (s / 60.0).floor() as u32, s % 60.0)
}

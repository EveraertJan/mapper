use std::collections::HashMap;

use rustmap_core::{MediaId, MediaSource, Project, Surface};
use rustmap_geom::{centered_rect, quad_to_arrays};
use rustmap_media::Loader;
use rustmap_ndi::{Ndi, ReceiverState};
use rustmap_render::{Gpu, GpuImage, ImageBinding, QuadRenderer, wgpu};
use rustmap_ui::{MediaStatus, VideoTime};

/// A texture with everything needed to draw it and show it in the UI.
struct Shown {
    image: GpuImage,
    binding: ImageBinding,
    thumbnail: egui::TextureId,
}

enum Runtime {
    Loading,
    Image(Shown),
    Live {
        receiver: rustmap_ndi::Receiver,
        shown: Option<Shown>,
    },
    #[cfg(feature = "video")]
    Video {
        player: rustmap_media::VideoPlayer,
        shown: Option<Shown>,
    },
    Failed,
}

impl Runtime {
    fn shown(&self) -> Option<&Shown> {
        match self {
            Runtime::Image(s) | Runtime::Live { shown: Some(s), .. } => Some(s),
            #[cfg(feature = "video")]
            Runtime::Video { shown: Some(s), .. } => Some(s),
            _ => None,
        }
    }
}

/// Keeps GPU copies of the project's media in sync with the project.
#[derive(Default)]
pub struct MediaManager {
    loader: Loader<MediaId>,
    items: HashMap<MediaId, Runtime>,
    /// Where each runtime was loaded from, to reload after relink or undo.
    loaded_from: HashMap<MediaId, MediaSource>,
    /// Playback position of each video.
    pub video_times: HashMap<MediaId, VideoTime>,
    /// What the UI shows for each item.
    pub status: HashMap<MediaId, MediaStatus>,
}

impl MediaManager {
    pub fn binding(&self, id: MediaId) -> Option<&ImageBinding> {
        match self.items.get(&id) {
            Some(
                Runtime::Image(shown)
                | Runtime::Live {
                    shown: Some(shown), ..
                },
            ) => Some(&shown.binding),
            #[cfg(feature = "video")]
            Some(Runtime::Video {
                shown: Some(shown), ..
            }) => Some(&shown.binding),
            _ => None,
        }
    }

    /// Forgets everything, e.g. when another show is opened.
    pub fn reset(&mut self, egui_renderer: &mut egui_wgpu::Renderer) {
        for rt in self.items.values() {
            if let Some(shown) = rt.shown() {
                egui_renderer.free_texture(&shown.thumbnail);
            }
        }
        self.items.clear();
        self.video_times.clear();
        self.loaded_from.clear();
        self.status.clear();
    }

    pub fn update(
        &mut self,
        project: &mut Project,
        gpu: &Gpu,
        quads: &QuadRenderer,
        egui_renderer: &mut egui_wgpu::Renderer,
        ndi: Option<&Ndi>,
        dt: f64,
    ) {
        puffin::profile_function!();
        // Removed media, or media whose source changed (relink, undo): drop
        // the runtime (this also stops NDI receivers) so it reloads below.
        let loaded_from = &mut self.loaded_from;
        self.items.retain(|id, rt| {
            let keep = project
                .media_item(*id)
                .is_some_and(|m| loaded_from.get(id) == Some(&m.source.load_key()));
            if !keep {
                loaded_from.remove(id);
            }
            if !keep && let Some(shown) = rt.shown() {
                egui_renderer.free_texture(&shown.thumbnail);
            }
            keep
        });
        self.status
            .retain(|id, _| project.media_item(*id).is_some());

        // New media: start loading or connecting.
        let max_dim = gpu.device.limits().max_texture_dimension_2d;
        for media in &project.media {
            if self.items.contains_key(&media.id) {
                continue;
            }
            let (runtime, status) = match &media.source {
                MediaSource::Image { path } => {
                    self.loader.load(media.id, path.clone(), max_dim);
                    (Runtime::Loading, MediaStatus::Loading)
                }
                #[cfg(feature = "video")]
                MediaSource::Video { path, playback } => match rustmap_media::VideoPlayer::open(path, playback.looping) {
                    Ok(player) => (
                        Runtime::Video { player, shown: None },
                        MediaStatus::Waiting("Starting…".into()),
                    ),
                    Err(err) => (Runtime::Failed, MediaStatus::Failed(err.to_string())),
                },
                #[cfg(not(feature = "video"))]
                MediaSource::Video { .. } => (
                    Runtime::Failed,
                    MediaStatus::Failed("This build has no video support (built without the `video` feature / FFmpeg)".into()),
                ),
                MediaSource::Ndi { source } => match ndi.map(|n| n.receive(source)) {
                    Some(Ok(receiver)) => (
                        Runtime::Live {
                            receiver,
                            shown: None,
                        },
                        MediaStatus::Waiting("Connecting…".into()),
                    ),
                    Some(Err(err)) => (Runtime::Failed, MediaStatus::Failed(err.to_string())),
                    None => (
                        Runtime::Failed,
                        MediaStatus::Failed(
                            "NDI runtime not installed (see the NDI status at the bottom)".into(),
                        ),
                    ),
                },
            };
            self.items.insert(media.id, runtime);
            self.loaded_from.insert(media.id, media.source.load_key());
            self.status.insert(media.id, status);
        }

        self.finish_image_loads(project, gpu, quads, egui_renderer);
        self.update_live(project, gpu, quads, egui_renderer);
        #[cfg(feature = "video")]
        self.update_video(project, gpu, quads, egui_renderer, dt);
        #[cfg(not(feature = "video"))]
        let _ = dt;
    }

    fn finish_image_loads(
        &mut self,
        project: &mut Project,
        gpu: &Gpu,
        quads: &QuadRenderer,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        for (id, result) in self.loader.poll() {
            // Skip results for media that was removed or relinked meanwhile.
            let current = project.media_item(id).map(|m| &m.source);
            if current.is_none() || !matches!(self.items.get(&id), Some(Runtime::Loading)) {
                continue;
            }
            match result {
                Ok(data) => {
                    let image = GpuImage::upload(&gpu.device, &gpu.queue, &data, "media");
                    let shown = show(gpu, quads, egui_renderer, image, None);
                    self.status.insert(
                        id,
                        MediaStatus::Ready {
                            size: shown.image.size,
                            thumbnail: shown.thumbnail,
                            detail: None,
                        },
                    );
                    fit_untouched_surfaces(project, id, data.aspect());
                    self.items.insert(id, Runtime::Image(shown));
                }
                Err(err) => {
                    tracing::warn!("{err}");
                    self.status.insert(id, MediaStatus::Failed(err.to_string()));
                    self.items.insert(id, Runtime::Failed);
                }
            }
        }
    }

    /// Uploads the newest frame of every live source.
    fn update_live(
        &mut self,
        project: &mut Project,
        gpu: &Gpu,
        quads: &QuadRenderer,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        puffin::profile_function!();
        for (id, rt) in &mut self.items {
            let Runtime::Live { receiver, shown } = rt else {
                continue;
            };
            if let Some(frame) = receiver.take_frame() {
                let size = [frame.width, frame.height];
                if shown.as_ref().is_none_or(|s| s.image.size != size) {
                    let image = GpuImage::live(&gpu.device, size, "ndi");
                    let first = shown.is_none();
                    let reuse = shown.take().map(|s| s.thumbnail);
                    *shown = Some(show(gpu, quads, egui_renderer, image, reuse));
                    if first {
                        fit_untouched_surfaces(
                            project,
                            *id,
                            size[0] as f32 / size[1].max(1) as f32,
                        );
                    }
                }
                if let Some(s) = shown {
                    s.image.write(&gpu.queue, &frame.data, frame.stride);
                }
            }

            let status = receiver.status();
            let new_status = match (shown.as_ref(), status.state) {
                (None, ReceiverState::NoSignal) => MediaStatus::Waiting("No signal".into()),
                (None, _) => MediaStatus::Waiting("Connecting…".into()),
                (Some(s), state) => MediaStatus::Ready {
                    size: s.image.size,
                    thumbnail: s.thumbnail,
                    detail: Some(if state == ReceiverState::NoSignal {
                        "No signal".into()
                    } else {
                        format!("{:.2} fps", status.frame_rate)
                    }),
                },
            };
            self.status.insert(*id, new_status);
        }
    }
}

impl MediaManager {
    /// Jumps a video to `seconds`.
    pub fn seek(&mut self, id: MediaId, seconds: f64) {
        #[cfg(feature = "video")]
        if let Some(Runtime::Video { player, .. }) = self.items.get_mut(&id) {
            player.seek(seconds);
        }
        #[cfg(not(feature = "video"))]
        let _ = (id, seconds);
    }

    /// Advances every video's clock and uploads frames that became due.
    #[cfg(feature = "video")]
    fn update_video(
        &mut self,
        project: &mut Project,
        gpu: &Gpu,
        quads: &QuadRenderer,
        egui_renderer: &mut egui_wgpu::Renderer,
        dt: f64,
    ) {
        puffin::profile_function!();
        self.video_times
            .retain(|id, _| project.media_item(*id).is_some());
        for (id, rt) in &mut self.items {
            let Runtime::Video { player, shown } = rt else {
                continue;
            };
            let Some(MediaSource::Video { playback, .. }) =
                project.media_item(*id).map(|m| m.source.clone())
            else {
                continue;
            };
            player.set_looping(playback.looping);
            if let Some(frame) = player.update(dt, !playback.paused, f64::from(playback.speed)) {
                let size = [frame.width, frame.height];
                if shown.as_ref().is_none_or(|s| s.image.size != size) {
                    let image = GpuImage::live(&gpu.device, size, "video");
                    let first = shown.is_none();
                    let reuse = shown.take().map(|s| s.thumbnail);
                    *shown = Some(show(gpu, quads, egui_renderer, image, reuse));
                    if first {
                        fit_untouched_surfaces(
                            project,
                            *id,
                            size[0] as f32 / size[1].max(1) as f32,
                        );
                    }
                }
                if let Some(s) = shown {
                    s.image.write(&gpu.queue, &frame.data, frame.stride);
                }
            }
            let info = player.info();
            self.video_times.insert(
                *id,
                VideoTime {
                    position: player.position(),
                    duration: info.duration,
                },
            );
            let status = match (shown.as_ref(), player.error()) {
                (_, Some(err)) => MediaStatus::Failed(err),
                (None, None) => MediaStatus::Waiting("Starting…".into()),
                (Some(s), None) => MediaStatus::Ready {
                    size: s.image.size,
                    thumbnail: s.thumbnail,
                    detail: Some(format!(
                        "{:.0} fps · {}",
                        info.frame_rate,
                        player.backend().label()
                    )),
                },
            };
            self.status.insert(*id, status);
        }
    }
}

/// Binds an image for drawing and as a UI thumbnail (reusing a thumbnail id
/// when a live source changes size, so the UI doesn't flicker).
fn show(
    gpu: &Gpu,
    quads: &QuadRenderer,
    egui_renderer: &mut egui_wgpu::Renderer,
    image: GpuImage,
    reuse_thumbnail: Option<egui::TextureId>,
) -> Shown {
    let binding = quads.bind_image(&gpu.device, &image);
    let thumbnail = match reuse_thumbnail {
        Some(id) => {
            egui_renderer.update_egui_texture_from_wgpu_texture(
                &gpu.device,
                &image.view,
                wgpu::FilterMode::Linear,
                id,
            );
            id
        }
        None => egui_renderer.register_native_texture(
            &gpu.device,
            &image.view,
            wgpu::FilterMode::Linear,
        ),
    };
    Shown {
        image,
        binding,
        thumbnail,
    }
}

/// Surfaces still at their default placement take the media's proportions,
/// so freshly added media doesn't appear stretched.
fn fit_untouched_surfaces(project: &mut Project, media: MediaId, aspect: f32) {
    let resolutions: HashMap<_, _> = project
        .outputs
        .iter()
        .map(|o| (o.id, o.resolution))
        .collect();
    for s in &mut project.surfaces {
        if s.media == Some(media)
            && s.corners == Surface::DEFAULT_CORNERS
            && let Some(res) = resolutions.get(&s.output)
        {
            s.corners = quad_to_arrays(&centered_rect(aspect, *res, 0.6));
        }
    }
}

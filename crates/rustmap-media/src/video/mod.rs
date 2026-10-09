//! Video playback.
//!
//! One decoder thread per video keeps a few frames ahead in a small queue.
//! The main thread owns the playback clock: each frame it advances the clock
//! (respecting pause and speed) and shows the newest decoded frame that is due.
//!
//! - **Seamless loops**: at the end of the file the decoder rewinds itself and
//!   keeps the timeline running (`pts + n × duration`), so there is no gap
//!   waiting for a seek round-trip.
//! - **Seeking** bumps an epoch; frames from before the seek are discarded.
//!
//! Backends only implement [`Decoder`] (open, seek, next frame):
//! - macOS: AVFoundation, hardware decoding through VideoToolbox.
//! - elsewhere (and optionally on macOS): FFmpeg.

#[cfg(has_avf)]
mod avf;
#[cfg(has_ffmpeg)]
mod ffmpeg;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::image_data::MediaError;

/// Frames decoded ahead of the clock.
const QUEUE: usize = 4;

/// What a video file contains.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    /// Seconds (0 if the container doesn't say).
    pub duration: f64,
    pub frame_rate: f64,
}

/// A decoded frame: opaque RGBA8, rows of `stride` bytes.
pub struct DecodedFrame {
    /// Position on the continuous playback timeline, in seconds.
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
    epoch: u64,
}

/// A frame straight from a backend, timed in file seconds.
pub(crate) struct RawFrame {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
}

/// What a video backend provides. Runs entirely on the decoder thread.
pub(crate) trait Decoder: Sized {
    fn open(path: &Path) -> Result<Self, MediaError>;
    /// Positions the decoder so the next frames start at or before `seconds`
    /// (the driver drops frames before the target).
    fn seek(&mut self, seconds: f64) -> Result<(), MediaError>;
    /// The next frame in file order, or `None` at the end of the file.
    fn next_frame(&mut self) -> Result<Option<RawFrame>, MediaError>;
}

/// Which backend plays a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// macOS system decoders (VideoToolbox hardware decoding).
    AvFoundation,
    Ffmpeg,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::AvFoundation => "AVFoundation",
            Backend::Ffmpeg => "FFmpeg",
        }
    }
}

/// Reads a file's video properties without decoding, trying each backend in
/// order of preference. Returns the backend that can play it.
pub fn probe(path: &Path) -> Result<(VideoInfo, Backend), MediaError> {
    let mut errors: Vec<String> = Vec::new();
    #[cfg(has_avf)]
    match avf::probe(path) {
        Ok(info) => return Ok((info, Backend::AvFoundation)),
        Err(err) => errors.push(err.to_string()),
    }
    #[cfg(has_ffmpeg)]
    match ffmpeg::probe(path) {
        Ok(info) => return Ok((info, Backend::Ffmpeg)),
        Err(err) => errors.push(err.to_string()),
    }
    Err(MediaError::Video {
        path: path.to_owned(),
        reason: if errors.is_empty() {
            "no video backend in this build".into()
        } else {
            errors.join("; ")
        },
    })
}

struct Control {
    shutdown: AtomicBool,
    looping: AtomicBool,
    /// (target seconds, epoch) requested by the main thread.
    seek: Mutex<Option<(f64, u64)>>,
    error: Mutex<Option<String>>,
}

/// Plays one video file. Stops its decoder when dropped.
pub struct VideoPlayer {
    info: VideoInfo,
    backend: Backend,
    control: Arc<Control>,
    frames: Receiver<DecodedFrame>,
    /// Next frame not yet due.
    pending: Option<DecodedFrame>,
    /// Playback clock on the continuous timeline (grows past `duration` when looping).
    clock: f64,
    epoch: u64,
    /// After a seek, show the first frame even if the clock hasn't reached it
    /// (so scrubbing while paused updates the picture).
    show_next: bool,
    /// Frames handed out, for stats/tests.
    pub frames_shown: u64,
    loop_count: Arc<AtomicU64>,
}

impl VideoPlayer {
    pub fn open(path: &Path, looping: bool) -> Result<Self, MediaError> {
        let (info, backend) = probe(path)?;
        let control = Arc::new(Control {
            shutdown: AtomicBool::new(false),
            looping: AtomicBool::new(looping),
            seek: Mutex::new(None),
            error: Mutex::new(None),
        });
        let (tx, frames) = sync_channel(QUEUE);
        let loop_count = Arc::new(AtomicU64::new(0));
        let (ctl, path_owned, loops) = (control.clone(), path.to_owned(), loop_count.clone());
        std::thread::Builder::new()
            .name("video decoder".into())
            .spawn(move || {
                let result = match backend {
                    #[cfg(has_avf)]
                    Backend::AvFoundation => {
                        drive::<avf::AvfDecoder>(&path_owned, info, &ctl, &tx, &loops)
                    }
                    #[cfg(has_ffmpeg)]
                    Backend::Ffmpeg => {
                        drive::<ffmpeg::FfmpegDecoder>(&path_owned, info, &ctl, &tx, &loops)
                    }
                    #[allow(unreachable_patterns)]
                    _ => Ok(()),
                };
                if let Err(err) = result {
                    tracing::warn!("{err}");
                    if let Ok(mut e) = ctl.error.lock() {
                        *e = Some(err.to_string());
                    }
                }
            })
            .map_err(|e| MediaError::Video {
                path: path.to_owned(),
                reason: e.to_string(),
            })?;
        tracing::info!(path = %path.display(), backend = backend.label(), "playing video");
        Ok(Self {
            info,
            backend,
            control,
            frames,
            pending: None,
            clock: 0.0,
            epoch: 0,
            show_next: true,
            frames_shown: 0,
            loop_count,
        })
    }

    pub fn info(&self) -> VideoInfo {
        self.info
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Decoder failure, if any (e.g. corrupt file mid-way).
    pub fn error(&self) -> Option<String> {
        self.control.error.lock().ok()?.clone()
    }

    pub fn set_looping(&self, looping: bool) {
        self.control.looping.store(looping, Ordering::Relaxed);
    }

    /// Position within the file, in seconds.
    pub fn position(&self) -> f64 {
        let d = self.info.duration;
        if d > 0.0 {
            self.clock.rem_euclid(d)
        } else {
            self.clock
        }
    }

    pub fn seek(&mut self, seconds: f64) {
        let t = if self.info.duration > 0.0 {
            seconds.clamp(0.0, self.info.duration)
        } else {
            seconds.max(0.0)
        };
        self.epoch += 1;
        self.clock = t;
        self.pending = None;
        self.show_next = true;
        if let Ok(mut s) = self.control.seek.lock() {
            *s = Some((t, self.epoch));
        }
    }

    /// Advances the clock by `dt` seconds and returns the frame to show now,
    /// if it changed.
    pub fn update(&mut self, dt: f64, playing: bool, speed: f64) -> Option<DecodedFrame> {
        let looping = self.control.looping.load(Ordering::Relaxed);
        if playing {
            self.clock += dt * speed.clamp(0.0, 16.0);
            if !looping && self.info.duration > 0.0 {
                // Not looping: stop on the last frame.
                let loops = self.loop_count.load(Ordering::Relaxed) as f64;
                let end = self.info.duration * (loops + 1.0);
                self.clock = self.clock.min(end);
            }
        }

        let mut due = None;
        loop {
            let frame = match self.pending.take() {
                Some(f) => f,
                None => match self.frames.try_recv() {
                    Ok(f) => f,
                    Err(_) => break,
                },
            };
            if frame.epoch != self.epoch {
                continue; // from before a seek
            }
            if self.show_next {
                self.show_next = false;
                // Snap the clock to what is on screen after a seek.
                self.clock = self.clock.max(frame.time);
                due = Some(frame);
                break;
            }
            if frame.time <= self.clock + 1e-4 {
                due = Some(frame); // newer due frame replaces older one
            } else {
                self.pending = Some(frame);
                break;
            }
        }
        if due.is_some() {
            self.frames_shown += 1;
        }
        due
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.control.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Sends a frame, waking up regularly to notice a seek or shutdown.
/// Returns false if the frame should be dropped (seek pending / shutting down).
fn send(tx: &SyncSender<DecodedFrame>, mut frame: DecodedFrame, ctl: &Control) -> bool {
    loop {
        if ctl.shutdown.load(Ordering::Relaxed) || ctl.seek.lock().is_ok_and(|s| s.is_some()) {
            return false;
        }
        match tx.try_send(frame) {
            Ok(()) => return true,
            Err(TrySendError::Full(f)) => {
                frame = f;
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

/// The decoder thread: seeks, loops and timeline bookkeeping around any backend.
fn drive<D: Decoder>(
    path: &Path,
    info: VideoInfo,
    ctl: &Control,
    tx: &SyncSender<DecodedFrame>,
    loops: &AtomicU64,
) -> Result<(), MediaError> {
    let mut decoder = D::open(path)?;
    let frame_step = if info.frame_rate > 0.0 {
        1.0 / info.frame_rate
    } else {
        1.0 / 30.0
    };
    let mut epoch = 0u64;
    let mut offset = 0.0f64; // timeline offset added by loops
    let mut skip_until = f64::NEG_INFINITY; // after a seek: drop frames before the target
    let mut last_time = 0.0f64; // file time of the last frame
    let mut at_eof = false;

    while !ctl.shutdown.load(Ordering::Relaxed) {
        // Seek requested by the main thread.
        let request = ctl.seek.lock().ok().and_then(|mut s| s.take());
        if let Some((target, new_epoch)) = request {
            decoder.seek(target)?;
            epoch = new_epoch;
            offset = 0.0;
            loops.store(0, Ordering::Relaxed);
            skip_until = target - frame_step * 0.5;
            at_eof = false;
        }

        if at_eof {
            if ctl.looping.load(Ordering::Relaxed) {
                // Seamless loop: rewind and keep the timeline going.
                decoder.seek(0.0)?;
                let duration = if info.duration > 0.0 {
                    info.duration
                } else {
                    last_time + frame_step
                };
                offset += duration;
                loops.fetch_add(1, Ordering::Relaxed);
                skip_until = f64::NEG_INFINITY;
                at_eof = false;
            } else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        }

        match decoder.next_frame()? {
            None => at_eof = true,
            Some(raw) => {
                if raw.time < skip_until {
                    continue;
                }
                last_time = raw.time;
                let frame = DecodedFrame {
                    time: raw.time + offset,
                    width: raw.width,
                    height: raw.height,
                    stride: raw.stride,
                    data: raw.data,
                    epoch,
                };
                send(tx, frame, ctl);
            }
        }
    }
    Ok(())
}

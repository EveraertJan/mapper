//! RUSTMAP media decoding.
//!
//! Everything here runs on the CPU, usually on worker threads, and hands
//! finished pixel data to the renderer. Video plays through AVFoundation on
//! macOS and FFmpeg elsewhere (see `video`).

mod image_data;
mod loader;
#[cfg(any(has_avf, has_ffmpeg))]
mod video;

pub use image_data::{ImageData, MediaError, load_image};
pub use loader::Loader;
#[cfg(any(has_avf, has_ffmpeg))]
pub use video::{Backend, DecodedFrame, VideoInfo, VideoPlayer, probe};

/// File extensions treated as video.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mxf", "mpg", "mpeg",
];

/// True if `path` looks like a video file.
pub fn is_video_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// True if this build can play video (built with the `video` feature, i.e. FFmpeg).
pub const VIDEO_SUPPORTED: bool = cfg!(any(has_avf, has_ffmpeg));

/// File extensions the image loader accepts, for file dialogs and drag & drop.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "webp", "bmp", "gif"];

/// True if `path` looks like an image we can load.
pub fn is_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

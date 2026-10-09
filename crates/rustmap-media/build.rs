//! Picks the video backend for the target:
//! - `has_avf`: AVFoundation (macOS, with the `video` feature)
//! - `has_ffmpeg`: FFmpeg (other platforms with `video`, or macOS with `ffmpeg-macos`)

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_avf)");
    println!("cargo:rustc-check-cfg=cfg(has_ffmpeg)");
    let macos = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos");
    let video = std::env::var_os("CARGO_FEATURE_VIDEO").is_some();
    let ffmpeg_macos = std::env::var_os("CARGO_FEATURE_FFMPEG_MACOS").is_some();
    if video && macos {
        println!("cargo:rustc-cfg=has_avf");
    }
    if (video && !macos) || ffmpeg_macos {
        println!("cargo:rustc-cfg=has_ffmpeg");
    }
}

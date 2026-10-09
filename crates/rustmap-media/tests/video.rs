//! Playback tests on a clip generated with the `ffmpeg` command-line tool.
//! Skipped if `ffmpeg` is not installed.
#![cfg(feature = "video")]
// Runs whichever backend this platform uses (FFmpeg on Linux CI, AVFoundation on macOS).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustmap_media::{VideoPlayer, probe};

/// 2 s, 10 fps, 64x36 H.264: brightness ramps 100 levels per second, so
/// frames are distinguishable. Committed so both backends test the same file
/// (regenerate: see tests/data/README.md).
fn clip() -> Option<PathBuf> {
    Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/clip.mp4"))
}

/// Runs the player at 60 fps of simulated time, waiting for decoded frames.
fn play(p: &mut VideoPlayer, seconds: f64, speed: f64) -> Vec<f64> {
    let mut times = Vec::new();
    let steps = (seconds * 60.0) as usize;
    for _ in 0..steps {
        let deadline = Instant::now() + Duration::from_millis(50);
        // Give the decoder a moment so tests aren't timing-sensitive.
        loop {
            if let Some(f) = p.update(0.0, false, 1.0) {
                times.push(f.time);
            }
            if Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
            if !times.is_empty() {
                break;
            }
        }
        if let Some(f) = p.update(1.0 / 60.0, true, speed) {
            times.push(f.time);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    times
}

#[test]
fn probes_size_rate_and_duration() {
    let Some(path) = clip() else { return };
    let (info, _backend) = probe(&path).unwrap();
    assert_eq!((info.width, info.height), (64, 36));
    assert!((info.frame_rate - 10.0).abs() < 0.01);
    assert!((info.duration - 2.0).abs() < 0.15, "{}", info.duration);
}

#[test]
fn plays_in_order_and_loops_seamlessly() {
    let Some(path) = clip() else { return };
    let mut p = VideoPlayer::open(&path, true).unwrap();
    let times = play(&mut p, 3.0, 1.0);
    assert!(
        times.windows(2).all(|w| w[1] > w[0]),
        "monotonic: {times:?}"
    );
    // 3 s of 10 fps footage, looping: ~30 frames and the timeline passed 2 s.
    assert!(
        (25..=33).contains(&times.len()),
        "{} frames: {times:?}",
        times.len()
    );
    assert!(
        times.last().unwrap() > &2.5,
        "timeline continues through the loop"
    );
    let gaps = times.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
    assert!(gaps < 0.25, "no hitch at the loop point, worst gap {gaps}");
    assert!(p.position() < 2.0);
}

#[test]
fn stops_at_end_without_loop() {
    let Some(path) = clip() else { return };
    let mut p = VideoPlayer::open(&path, false).unwrap();
    let times = play(&mut p, 3.0, 1.0);
    assert!(times.last().unwrap() < &2.0, "{times:?}");
    assert!((17..=21).contains(&times.len()), "{} frames", times.len());
}

#[test]
fn seek_shows_target_frame_even_when_paused() {
    let Some(path) = clip() else { return };
    let mut p = VideoPlayer::open(&path, true).unwrap();
    play(&mut p, 0.3, 1.0);
    p.seek(1.5);
    let deadline = Instant::now() + Duration::from_secs(5);
    let frame = loop {
        if let Some(f) = p.update(0.0, false, 1.0) {
            break f;
        }
        assert!(Instant::now() < deadline, "no frame after seek");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!((frame.time - 1.5).abs() < 0.11, "{}", frame.time);
    // Luma ramps 100/s: at 1.5 s the picture is mid-gray, not black.
    let y = frame.data[(10 * frame.stride + 40) as usize];
    assert!((120..=180).contains(&y), "pixel {y}");
}

#[test]
fn double_speed_plays_twice_as_fast() {
    let Some(path) = clip() else { return };
    let mut p = VideoPlayer::open(&path, true).unwrap();
    let times = play(&mut p, 0.8, 2.0);
    let last = *times.last().unwrap();
    assert!((1.4..=1.7).contains(&last), "{last}");
}

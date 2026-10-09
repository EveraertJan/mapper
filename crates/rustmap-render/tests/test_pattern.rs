//! Renders test patterns headless and checks known pixels.
//!
//! Skipped (passes with a note) on machines without any GPU adapter.
//! Set `RUSTMAP_DUMP=dir` to also write the images as PPM for eyeballing.

use rustmap_core::TestPattern;
use rustmap_render::{Gpu, OutputTarget, PatternParams, TestPatternRenderer};

const W: u32 = 640;
const H: u32 = 360;

fn render(pattern: TestPattern) -> Option<Vec<u8>> {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping GPU test: {err}");
            return None;
        }
    };
    let renderer = TestPatternRenderer::new(&gpu.device);
    let state = renderer.create_state(&gpu.device);
    let target = OutputTarget::new(&gpu.device, [W, H], "test");
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render(
        &gpu.queue,
        &mut encoder,
        &state,
        &target,
        &PatternParams {
            pattern,
            time_seconds: 0.0,
            grid_cell_px: 60,
            output_index: 2,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let pixels = target.read_pixels(&gpu.device, &gpu.queue);
    assert_eq!(pixels.len(), (W * H * 4) as usize);

    if let Ok(dir) = std::env::var("RUSTMAP_DUMP") {
        let mut ppm = format!("P6\n{W} {H}\n255\n").into_bytes();
        ppm.extend(pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
        let name = format!(
            "{dir}/{}.ppm",
            pattern.label().replace(' ', "_").to_lowercase()
        );
        std::fs::write(name, ppm).unwrap();
    }
    Some(pixels)
}

fn px(img: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2]]
}

#[test]
fn grid_has_orientation_tags_border_and_center_cross() {
    let Some(img) = render(TestPattern::Grid) else {
        return;
    };

    // Corner tags: TL red, TR green, BL blue, BR magenta.
    let [r, g, b] = px(&img, 4, 4);
    assert!(
        r > 200 && g < 80 && b < 80,
        "top-left should be red, got {r},{g},{b}"
    );
    let [r, g, b] = px(&img, W - 5, 4);
    assert!(
        g > 200 && r < 80 && b < 100,
        "top-right should be green, got {r},{g},{b}"
    );
    let [r, g, b] = px(&img, 4, H - 5);
    assert!(
        b > 200 && r < 80,
        "bottom-left should be blue, got {r},{g},{b}"
    );
    let [r, g, b] = px(&img, W - 5, H - 5);
    assert!(
        r > 200 && b > 200 && g < 80,
        "bottom-right should be magenta, got {r},{g},{b}"
    );

    // Edge pixels are white so cropping is visible.
    assert_eq!(px(&img, 0, H / 3), [255, 255, 255]);
    assert_eq!(px(&img, W - 1, H / 3), [255, 255, 255]);

    // Center cross is yellow.
    let [r, g, b] = px(&img, W / 2, H / 2 + 37);
    assert!(
        r > 200 && g > 180 && b < 80,
        "center cross should be yellow, got {r},{g},{b}"
    );
}

#[test]
fn solid_fills_with_color() {
    let Some(img) = render(TestPattern::Solid([1.0, 0.5, 0.0])) else {
        return;
    };
    for (x, y) in [(0, 0), (W / 2, H / 2), (W - 1, H - 1)] {
        let [r, g, b] = px(&img, x, y);
        assert_eq!((r, b), (255, 0));
        assert!(
            (127..=128).contains(&g),
            "gamma must not be re-applied, got g={g}"
        );
    }
}

#[test]
fn color_bars_start_white_and_end_black() {
    let Some(img) = render(TestPattern::ColorBars) else {
        return;
    };
    assert_eq!(px(&img, 10, 10), [255, 255, 255]);
    assert_eq!(px(&img, W - 10, 10), [0, 0, 0]);
    // Smooth ramp along the bottom goes from dark to bright.
    assert!(px(&img, 10, H - 5)[0] < 20);
    assert!(px(&img, W - 10, H - 5)[0] > 235);
}

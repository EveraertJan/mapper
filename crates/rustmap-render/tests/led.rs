//! Headless GPU test for LED sampling. Skipped without a GPU adapter.

use rustmap_core::{Fixture, FixtureId, FixtureShape, OutputId, Wiring};
use rustmap_geom::Vec2;
use rustmap_media::ImageData;
use rustmap_render::{
    Gpu, GpuImage, LedSampler, LedSamples, OutputTarget, QuadDraw, QuadFill, QuadRenderer,
};

const SIZE: u32 = 256;

/// Output showing quadrants: red, green / blue, white.
fn quadrants(gpu: &Gpu, quads: &QuadRenderer) -> OutputTarget {
    // Big enough that filtering only blurs a sliver at the borders.
    let rgba = image::RgbaImage::from_fn(64, 64, |x, y| match (x < 32, y < 32) {
        (true, true) => image::Rgba([255, 0, 0, 255]),
        (false, true) => image::Rgba([0, 255, 0, 255]),
        (true, false) => image::Rgba([0, 0, 255, 255]),
        (false, false) => image::Rgba([255, 255, 255, 255]),
    });
    let data = ImageData::from_rgba(rgba, 4096);
    let img = GpuImage::upload(&gpu.device, &gpu.queue, &data, "test");
    let binding = quads.bind_image(&gpu.device, &img);
    let target = OutputTarget::new(&gpu.device, [SIZE, SIZE], "test");
    let state = quads.create_state(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let full = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];
    quads.render(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &target,
        &[QuadDraw::simple(
            &state,
            full,
            1.0,
            QuadFill::Image(&binding),
        )],
    );
    gpu.queue.submit(Some(encoder.finish()));
    target
}

fn sample(
    gpu: &Gpu,
    sampler: &LedSampler,
    source: &OutputTarget,
    points: &[[f32; 4]],
) -> Vec<[u8; 3]> {
    let mut state: Option<LedSamples> = None;
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    sampler.sample(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &mut state,
        source,
        points,
    );
    gpu.queue.submit(Some(encoder.finish()));
    let state = state.as_mut().unwrap();
    state.after_submit();
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    state.read_latest().expect("samples ready")
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

use rustmap_render::wgpu;

#[test]
fn samples_follow_the_image() {
    let gpu = match Gpu::headless() {
        Ok(g) => g,
        Err(err) => {
            eprintln!("skipping GPU test: {err}");
            return;
        }
    };
    let quads = QuadRenderer::new(&gpu.device, &gpu.queue);
    let sampler = LedSampler::new(&gpu.device);
    let target = quadrants(&gpu, &quads);

    // Points, an area sample inside one quadrant, and one off the canvas.
    let got = sample(
        &gpu,
        &sampler,
        &target,
        &[
            [0.25, 0.25, 0.0, 0.0],
            [0.75, 0.25, 0.0, 0.0],
            [0.25, 0.75, 0.0, 0.0],
            [0.75, 0.75, 0.1, 0.1],
            [1.5, 0.5, 0.0, 0.0],
        ],
    );
    assert_eq!(got.len(), 5);
    assert!(close(got[0], [255, 0, 0]), "{:?}", got[0]);
    assert!(close(got[1], [0, 255, 0]), "{:?}", got[1]);
    assert!(close(got[2], [0, 0, 255]), "{:?}", got[2]);
    assert!(close(got[3], [255, 255, 255]), "{:?}", got[3]);
    assert_eq!(got[4], [0, 0, 0]);

    // An area straddling the red/green border averages them.
    let mix = sample(&gpu, &sampler, &target, &[[0.5, 0.25, 0.2, 0.0]])[0];
    assert!(
        mix[0].abs_diff(128) < 12 && mix[1].abs_diff(128) < 12,
        "{mix:?}"
    );

    // More LEDs than one row of the sample texture, in wiring order.
    let matrix = Fixture::new(
        FixtureId(1),
        OutputId(1),
        FixtureShape::Matrix {
            corners: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            cols: 20,
            rows: 15,
            wiring: Wiring::default(),
        },
    );
    let points = matrix.sample_points();
    let got = sample(&gpu, &sampler, &target, &points);
    assert_eq!(got.len(), 300);
    assert!(close(got[0], [255, 0, 0]));
    assert!(close(got[19], [0, 255, 0]));
    assert!(close(got[280], [0, 0, 255]));
    assert!(close(got[299], [255, 255, 255]));
    assert!(close(got[256], [255, 255, 255])); // col 16, row 12
}

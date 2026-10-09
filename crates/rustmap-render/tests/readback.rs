//! The async readback ring returns the newest rendered frame.

use rustmap_core::TestPattern;
use rustmap_render::{FrameReadback, Gpu, OutputTarget, PatternParams, TestPatternRenderer};

#[test]
fn returns_newest_frame_with_padded_rows() {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping GPU test: {err}");
            return;
        }
    };
    // 100 px wide: 400 bytes per row, padded to 512 for the copy.
    let size = [100, 10];
    let target = OutputTarget::new(&gpu.device, size, "test");
    let patterns = TestPatternRenderer::new(&gpu.device);
    let state = patterns.create_state(&gpu.device);
    let mut readback = FrameReadback::new(&gpu.device, size);

    let mut seen = Vec::new();
    for frame in 0..6u8 {
        let level = frame as f32 * 0.1;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        patterns.render(
            &gpu.queue,
            &mut encoder,
            &state,
            &target,
            &PatternParams {
                pattern: TestPattern::Solid([level, 0.0, 0.0]),
                time_seconds: 0.0,
                grid_cell_px: 10,
                output_index: 1,
            },
        );
        readback.capture(&mut encoder, &target);
        gpu.queue.submit(Some(encoder.finish()));
        readback.after_submit();
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        if let Some((stride, red)) =
            readback.read_latest(|f| (f.stride, f.data[(f.stride * 9) as usize + 99 * 4]))
        {
            assert_eq!(stride, 512);
            seen.push(red);
        }
    }
    // Waiting for the GPU each frame means every frame comes back, in order.
    let expected: Vec<u8> = (0..6)
        .map(|i| (i as f32 * 0.1 * 255.0).round() as u8)
        .collect();
    assert_eq!(seen.len(), expected.len());
    for (got, want) in seen.iter().zip(&expected) {
        assert!(got.abs_diff(*want) <= 1, "{seen:?} vs {expected:?}");
    }
}

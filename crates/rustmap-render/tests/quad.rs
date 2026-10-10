//! Headless GPU tests for perspective-correct quad rendering.
//! Skipped (passes with a note) on machines without any GPU adapter.

use rustmap_core::{BlendMode, EdgeBlend, Mask, Shape, TestPattern};
use rustmap_geom::{MeshGrid, Quad, Vec2, project, square_to_quad};
use rustmap_media::ImageData;
use rustmap_render::{
    EdgeBlendRenderer, Gpu, GpuImage, OutputTarget, PatternParams, QuadDraw, QuadFill,
    QuadRenderer, TestPatternRenderer,
};

const SIZE: u32 = 512;

/// A built-in shader's source.
fn builtin(id: &str) -> (String, String) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/shaders")
        .join(format!("{id}.wgsl"));
    (id.to_owned(), std::fs::read_to_string(path).unwrap())
}

/// Chain entries with their declared parameter names.
fn chain(shaders: &[(String, String)]) -> Vec<rustmap_render::ChainShader<'_>> {
    shaders
        .iter()
        .map(|(id, src)| rustmap_render::ChainShader {
            id,
            source: src,
            params: rustmap_core::shaders::parse_header(id, src)
                .unwrap()
                .params
                .into_iter()
                .map(|p| p.id)
                .collect(),
            own_pass: false,
        })
        .collect()
}

#[derive(Default)]
struct ExtraM5 {
    mesh: Option<MeshGrid>,
    mask: Option<Mask>,
    edge_blend: Option<EdgeBlend>,
    /// Shader chain: (id, WGSL source).
    chain: Vec<(String, String)>,
    params: Vec<f32>,
    /// Chain entries that get their own pass.
    own_pass: Vec<usize>,
}

struct Extra {
    blend: BlendMode,
    shape: Shape,
    input: Quad,
    m5: ExtraM5,
}

impl Default for Extra {
    fn default() -> Self {
        Self {
            blend: BlendMode::Normal,
            shape: Shape::Quad,
            input: full(),
            m5: ExtraM5::default(),
        }
    }
}

struct Ctx {
    gpu: Gpu,
    quads: QuadRenderer,
    patterns: TestPatternRenderer,
    edge: EdgeBlendRenderer,
}

fn ctx() -> Option<Ctx> {
    match Gpu::headless() {
        Ok(gpu) => Some(Ctx {
            quads: QuadRenderer::new(&gpu.device, &gpu.queue),
            patterns: TestPatternRenderer::new(&gpu.device),
            edge: EdgeBlendRenderer::new(&gpu.device),
            gpu,
        }),
        Err(err) => {
            eprintln!("skipping GPU test: {err}");
            None
        }
    }
}

impl Ctx {
    /// Clears to `background`, draws one quad, returns RGBA pixels.
    fn render(
        &self,
        background: TestPattern,
        corners: Quad,
        opacity: f32,
        fill: QuadFill<'_>,
    ) -> Vec<u8> {
        self.render_with(background, corners, opacity, fill, Extra::default())
    }

    fn render_with(
        &self,
        background: TestPattern,
        corners: Quad,
        opacity: f32,
        fill: QuadFill<'_>,
        extra: Extra,
    ) -> Vec<u8> {
        let device = &self.gpu.device;
        let target = OutputTarget::new(device, [SIZE, SIZE], "test");
        let pattern = self.patterns.create_state(device);
        let state = self.quads.create_state(device);
        let filter = (!extra.m5.chain.is_empty()).then(|| {
            let mut c = chain(&extra.m5.chain);
            for i in &extra.m5.own_pass {
                c[*i].own_pass = true;
            }
            self.quads
                .compile_chain(device, &c)
                .expect("test chain compiles")
        });
        let passes = match (&filter, fill) {
            (Some(f), QuadFill::Image(b)) if f.pass_count() > 0 => {
                Some(self.quads.pass_targets(device, b.size))
            }
            _ => None,
        };
        let mut params = [0.0; rustmap_core::shaders::MAX_CHAIN_PARAMS];
        params[..extra.m5.params.len()].copy_from_slice(&extra.m5.params);
        let mut encoder = device.create_command_encoder(&Default::default());
        self.patterns.render(
            &self.gpu.queue,
            &mut encoder,
            &pattern,
            &target,
            &PatternParams {
                pattern: background,
                time_seconds: 0.0,
                grid_cell_px: 64,
                output_index: 1,
            },
        );
        self.quads.render(
            device,
            &self.gpu.queue,
            &mut encoder,
            &target,
            &[QuadDraw {
                state: &state,
                corners,
                mesh: extra.m5.mesh.as_ref(),
                opacity,
                fill,
                blend: extra.blend,
                shape: extra.shape,
                input: extra.input,
                mask: extra.m5.mask.as_ref(),
                filter: filter.as_ref(),
                params,
                passes: passes.as_ref(),
                time: 0.0,
            }],
        );
        if let Some(blend) = &extra.m5.edge_blend {
            let state = self.edge.create_state(device);
            self.edge
                .render(&self.gpu.queue, &mut encoder, &state, &target, blend);
        }
        self.gpu.queue.submit(Some(encoder.finish()));
        target.read_pixels(device, &self.gpu.queue)
    }
}

fn px(img: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

fn keystone() -> Quad {
    [
        Vec2::new(0.10, 0.05),
        Vec2::new(0.92, 0.18),
        Vec2::new(0.80, 0.95),
        Vec2::new(0.15, 0.70),
    ]
}

fn full() -> Quad {
    [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ]
}

#[test]
fn texture_coordinates_follow_the_homography() {
    let Some(ctx) = ctx() else { return };
    let quad = keystone();
    let img = ctx.render(TestPattern::Off, quad, 1.0, QuadFill::UvDebug);
    let inverse = square_to_quad(&quad).inverse();

    let mut checked = 0;
    let mut worst = 0.0_f32;
    for gy in 1..10 {
        for gx in 1..10 {
            // Pick the pixel under a texture point, then compare its color to
            // the exact texture coordinate of that pixel's center.
            let uv = Vec2::new(gx as f32 / 10.0, gy as f32 / 10.0);
            let p = project(&square_to_quad(&quad), uv).unwrap() * SIZE as f32;
            let (x, y) = (p.x.floor() as u32, p.y.floor() as u32);
            let center = (Vec2::new(x as f32, y as f32) + 0.5) / SIZE as f32;
            let expected = project(&inverse, center).unwrap();
            let [r, g, _, _] = px(&img, x, y);
            let got = Vec2::new(r as f32, g as f32) / 255.0;
            worst = worst.max((got - expected).abs().max_element());
            checked += 1;
        }
    }
    assert_eq!(checked, 81);
    // 8-bit output: allow 1.5 steps of quantization.
    assert!(worst < 1.5 / 255.0, "uv error up to {worst}");
}

#[test]
fn diagonal_split_does_not_bend() {
    // An affine two-triangle split would put the texture center somewhere
    // else on a keystoned quad; a perspective map puts it on the diagonals'
    // intersection.
    let Some(ctx) = ctx() else { return };
    let q = keystone();
    let img = ctx.render(TestPattern::Off, q, 1.0, QuadFill::UvDebug);
    let (a, b) = (q[0], q[2] - q[0]);
    let (c, d) = (q[1], q[3] - q[1]);
    let t = (c - a).perp_dot(d) / b.perp_dot(d);
    let hit = (a + b * t) * SIZE as f32;
    let [r, g, _, _] = px(&img, hit.x as u32, hit.y as u32);
    assert!(
        (r as i32 - 128).abs() <= 2 && (g as i32 - 128).abs() <= 2,
        "got {r},{g}"
    );
}

#[test]
fn image_fill_and_opacity() {
    let Some(ctx) = ctx() else { return };
    // 2x2 image: red, green / blue, white. No mips beyond what from_rgba makes.
    let mut rgba = image::RgbaImage::new(2, 2);
    rgba.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    rgba.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
    rgba.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
    rgba.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
    let data = ImageData::from_rgba(rgba, 4096);
    let gpu_image = GpuImage::upload(&ctx.gpu.device, &ctx.gpu.queue, &data, "test");
    let binding = ctx.quads.bind_image(&ctx.gpu.device, &gpu_image);

    let img = ctx.render(TestPattern::Off, full(), 1.0, QuadFill::Image(&binding));
    // Quadrant centers sit (almost) on texel centers; allow a step of filtering.
    let close =
        |got: [u8; 4], want: [u8; 3]| got[..3].iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 2);
    assert!(close(px(&img, 128, 128), [255, 0, 0]));
    assert!(close(px(&img, 384, 128), [0, 255, 0]));
    assert!(close(px(&img, 128, 384), [0, 0, 255]));
    assert!(close(px(&img, 384, 384), [255, 255, 255]));

    let half = ctx.render(TestPattern::Off, full(), 0.5, QuadFill::Image(&binding));
    let [r, g, b, _] = px(&half, 384, 384);
    for c in [r, g, b] {
        assert!((126..=128).contains(&c), "50% white over black, got {c}");
    }
}

#[test]
fn transparent_pixels_show_what_is_behind() {
    let Some(ctx) = ctx() else { return };
    // Left half opaque red, right half fully transparent (with garbage color,
    // which premultiplication must remove).
    let mut rgba = image::RgbaImage::new(2, 1);
    rgba.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    rgba.put_pixel(1, 0, image::Rgba([0, 255, 0, 0]));
    let data = ImageData::from_rgba(rgba, 4096);
    let gpu_image = GpuImage::upload(&ctx.gpu.device, &ctx.gpu.queue, &data, "test");
    let binding = ctx.quads.bind_image(&ctx.gpu.device, &gpu_image);

    let img = ctx.render(
        TestPattern::Solid([1.0, 1.0, 1.0]),
        full(),
        1.0,
        QuadFill::Image(&binding),
    );
    assert_eq!(px(&img, 60, 256)[..3], [255, 0, 0]);
    assert_eq!(px(&img, 450, 256)[..3], [255, 255, 255], "no green fringe");
    // Across the edge the mix must go red -> white without any green.
    for x in 200..312 {
        let [_, g, b, _] = px(&img, x, 256);
        assert_eq!(g, b, "pixel {x}: green {g} != blue {b} means a fringe");
    }
}

#[test]
fn non_convex_quads_are_not_drawn() {
    let Some(ctx) = ctx() else { return };
    let mut bowtie = full();
    bowtie.swap(2, 3);
    let img = ctx.render(TestPattern::Off, bowtie, 1.0, QuadFill::Checker);
    assert!(img.chunks(4).all(|p| p[..3] == [0, 0, 0]));
}

#[test]
fn checker_placeholder_draws_inside_quad_only() {
    let Some(ctx) = ctx() else { return };
    let q = keystone();
    let img = ctx.render(TestPattern::Off, q, 1.0, QuadFill::Checker);
    // Inside: something visible. Outside (top-right corner of the output): black.
    let inside = (q[0] + q[2]) * 0.5 * SIZE as f32;
    assert_ne!(
        px(&img, inside.x as u32 + 20, inside.y as u32 + 20)[..3],
        [0, 0, 0]
    );
    assert_eq!(px(&img, SIZE - 2, 2)[..3], [0, 0, 0]);

    if let Ok(dir) = std::env::var("RUSTMAP_DUMP") {
        let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        ppm.extend(img.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write(format!("{dir}/checker_quad.ppm"), ppm).unwrap();
    }
}

#[test]
fn input_region_crops_the_media() {
    let Some(ctx) = ctx() else { return };
    // 64x64 image with solid quadrants (red, green / blue, white).
    let rgba = image::RgbaImage::from_fn(64, 64, |x, y| match (x < 32, y < 32) {
        (true, true) => image::Rgba([255, 0, 0, 255]),
        (false, true) => image::Rgba([0, 255, 0, 255]),
        (true, false) => image::Rgba([0, 0, 255, 255]),
        (false, false) => image::Rgba([255, 255, 255, 255]),
    });
    let data = ImageData::from_rgba(rgba, 4096);
    let gpu_img = GpuImage::upload(&ctx.gpu.device, &ctx.gpu.queue, &data, "quads");
    let binding = ctx.quads.bind_image(&ctx.gpu.device, &gpu_img);
    // Show only the top-left quarter of the image (the red texel) on the whole quad.
    let input = [
        Vec2::new(0.0, 0.0),
        Vec2::new(0.5, 0.0),
        Vec2::new(0.5, 0.5),
        Vec2::new(0.0, 0.5),
    ];
    let img = ctx.render_with(
        TestPattern::Off,
        full(),
        1.0,
        QuadFill::Image(&binding),
        Extra {
            input,
            ..Extra::default()
        },
    );
    for (x, y) in [(20, 20), (480, 30), (256, 256), (30, 480)] {
        let [r, g, b, _] = px(&img, x, y);
        assert!(r > 200 && g < 60 && b < 60, "({x},{y}) = {r},{g},{b}");
    }
}

#[test]
fn shape_masks() {
    let Some(ctx) = ctx() else { return };
    let tri = ctx.render_with(
        TestPattern::Off,
        full(),
        1.0,
        QuadFill::UvDebug,
        Extra {
            shape: Shape::Triangle,
            ..Extra::default()
        },
    );
    assert_ne!(px(&tri, 256, 400)[..2], [0, 0], "inside triangle");
    assert_eq!(
        px(&tri, 40, 60)[..3],
        [0, 0, 0],
        "outside triangle (top-left)"
    );

    let circle = ctx.render_with(
        TestPattern::Off,
        full(),
        1.0,
        QuadFill::UvDebug,
        Extra {
            shape: Shape::Circle,
            ..Extra::default()
        },
    );
    assert_ne!(px(&circle, 256, 256)[..2], [0, 0], "inside circle");
    assert_eq!(px(&circle, 10, 10)[..3], [0, 0, 0], "corner outside circle");
    assert_ne!(
        px(&circle, 256, 5)[..2],
        [0, 0],
        "circle touches the top edge"
    );
}

#[test]
fn blend_modes() {
    let Some(ctx) = ctx() else { return };
    // 50% gray solid behind; checker-free test via a 1x1 image of a known color.
    let color = |c: [u8; 4]| {
        let rgba = image::RgbaImage::from_pixel(1, 1, image::Rgba(c));
        let data = ImageData::from_rgba(rgba, 16);
        let img = GpuImage::upload(&ctx.gpu.device, &ctx.gpu.queue, &data, "c");
        let b = ctx.quads.bind_image(&ctx.gpu.device, &img);
        (img, b)
    };
    let (_i, gray) = color([128, 128, 128, 255]);
    let bg = TestPattern::Solid([0.5, 0.5, 0.5]);
    let run = |blend| {
        let img = ctx.render_with(
            bg,
            full(),
            1.0,
            QuadFill::Image(&gray),
            Extra {
                blend,
                ..Extra::default()
            },
        );
        px(&img, 256, 256)[0] as i32
    };
    let close = |got: i32, want: i32| (got - want).abs() <= 2;
    assert!(close(run(BlendMode::Normal), 128));
    assert!(close(run(BlendMode::Add), 255), "128 + 128 clips");
    assert!(close(run(BlendMode::Multiply), 64), "0.5 * 0.5");
    assert!(close(run(BlendMode::Screen), 192), "1 - 0.5*0.5");
}

fn extra(m5: ExtraM5) -> Extra {
    Extra {
        m5,
        ..Extra::default()
    }
}

#[test]
fn flat_mesh_renders_like_the_quad() {
    let Some(ctx) = ctx() else { return };
    let q = keystone();
    let quad = ctx.render(TestPattern::Off, q, 1.0, QuadFill::UvDebug);
    let mesh = MeshGrid::from_quad(&q, 5, 5, true);
    let warped = ctx.render_with(
        TestPattern::Off,
        q,
        1.0,
        QuadFill::UvDebug,
        extra(ExtraM5 {
            mesh: Some(mesh),
            ..Default::default()
        }),
    );
    // A mesh sampled from the quad approximates its perspective closely.
    let mut worst = 0u8;
    for y in (40..480).step_by(37) {
        for x in (60..460).step_by(41) {
            let (a, b) = (px(&quad, x, y), px(&warped, x, y));
            if a[3] == 255 && b[3] == 255 && a[..2] != [0, 0] && b[..2] != [0, 0] {
                worst = worst.max(a[0].abs_diff(b[0])).max(a[1].abs_diff(b[1]));
            }
        }
    }
    assert!(worst <= 3, "uv differs by up to {worst}/255");
}

#[test]
fn bent_mesh_moves_the_image() {
    let Some(ctx) = ctx() else { return };
    let mut mesh = MeshGrid::from_quad(&full(), 3, 3, true);
    mesh.points[4] = Vec2::new(0.75, 0.5); // pull the center to the right
    let img = ctx.render_with(
        TestPattern::Off,
        full(),
        1.0,
        QuadFill::UvDebug,
        extra(ExtraM5 {
            mesh: Some(mesh),
            ..Default::default()
        }),
    );
    // Texture u = 0.5 now sits at x ≈ 0.75 on the center row.
    let [r, _, _, _] = px(&img, (0.75 * SIZE as f32) as u32, SIZE / 2);
    assert!((120..=136).contains(&r), "u at the moved center = {r}");
}

#[test]
fn mask_cuts_and_inverts() {
    let Some(ctx) = ctx() else { return };
    let square = |invert| Mask {
        points: vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]],
        feather: 0.0,
        invert,
    };
    let run = |mask| {
        ctx.render_with(
            TestPattern::Off,
            full(),
            1.0,
            QuadFill::Checker,
            extra(ExtraM5 {
                mask: Some(mask),
                ..Default::default()
            }),
        )
    };
    let normal = run(square(false));
    assert_ne!(px(&normal, 256, 256)[..3], [0, 0, 0], "inside shows");
    assert_eq!(px(&normal, 40, 40)[..3], [0, 0, 0], "outside is cut");
    let inverted = run(square(true));
    assert_eq!(
        px(&inverted, 256, 256)[..3],
        [0, 0, 0],
        "inverted: inside is cut"
    );
    assert_ne!(px(&inverted, 40, 40)[..3], [0, 0, 0]);

    let mut soft = square(false);
    soft.feather = 0.2;
    let feathered = run(soft);
    // Just inside the edge: partly faded; deep inside: full.
    let edge = px(&feathered, (0.27 * SIZE as f32) as u32, 256);
    let deep = px(&normal, (0.27 * SIZE as f32) as u32, 256);
    assert!(
        edge[0] < deep[0] / 2,
        "feathered edge {} vs hard {}",
        edge[0],
        deep[0]
    );
}

#[test]
fn color_shader() {
    let Some(ctx) = ctx() else { return };
    let rgba = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 255]));
    let data = ImageData::from_rgba(rgba, 16);
    let img = GpuImage::upload(&ctx.gpu.device, &ctx.gpu.queue, &data, "c");
    let b = ctx.quads.bind_image(&ctx.gpu.device, &img);
    // brightness, contrast, saturation, hue, red, green, blue
    let run = |params: [f32; 7]| {
        let out = ctx.render_with(
            TestPattern::Off,
            full(),
            1.0,
            QuadFill::Image(&b),
            extra(ExtraM5 {
                chain: vec![builtin("color")],
                params: params.to_vec(),
                ..Default::default()
            }),
        );
        px(&out, 256, 256)
    };
    let gray = run([0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    assert!(
        gray[0].abs_diff(gray[1]) <= 1 && gray[1].abs_diff(gray[2]) <= 1,
        "{gray:?}"
    );
    let bright = run([0.2, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0]);
    assert!(bright[1].abs_diff(151) <= 2, "{bright:?}");
    let red_off = run([0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
    assert_eq!(red_off[0], 0);
    let hue = run([0.0, 1.0, 1.0, 120.0, 1.0, 1.0, 1.0]);
    // A 120° hue turn moves the dominant channel from red to green.
    assert!(hue[1] > hue[0] && hue[1] > hue[2], "{hue:?}");
}

#[test]
fn edge_blend_ramps_match_the_cpu_formula() {
    let Some(ctx) = ctx() else { return };
    let blend = EdgeBlend {
        right: 0.25,
        ..Default::default()
    };
    let img = ctx.render_with(
        TestPattern::Solid([1.0, 1.0, 1.0]),
        full(),
        0.0,
        QuadFill::Checker,
        extra(ExtraM5 {
            edge_blend: Some(blend),
            ..Default::default()
        }),
    );
    assert_eq!(px(&img, 100, 256)[0], 255, "outside the ramp: untouched");
    assert!(px(&img, SIZE - 1, 256)[0] <= 3, "outer edge: black");
    // Middle of the ramp (t = 0.5 from the edge).
    let x = SIZE - (0.125 * SIZE as f32) as u32;
    let want = (blend.ramp(0.5) * 255.0).round() as i32;
    let got = px(&img, x, 256)[0] as i32;
    assert!((got - want).abs() <= 3, "got {got}, want {want}");
}

fn one_pixel(ctx: &Ctx, rgba: [u8; 4]) -> rustmap_render::ImageBinding {
    let data = ImageData::from_rgba(image::RgbaImage::from_pixel(1, 1, image::Rgba(rgba)), 16);
    let img = GpuImage::upload(&ctx.gpu.device, &ctx.gpu.queue, &data, "px");
    ctx.quads.bind_image(&ctx.gpu.device, &img)
}

#[test]
fn posterize_quantizes_each_channel() {
    let Some(ctx) = ctx() else { return };
    let b = one_pixel(&ctx, [200, 100, 50, 255]);
    let run = |bits: u8| {
        let out = ctx.render_with(
            TestPattern::Off,
            full(),
            1.0,
            QuadFill::Image(&b),
            extra(ExtraM5 {
                chain: vec![builtin("posterize")],
                params: vec![f32::from(bits)],
                ..Default::default()
            }),
        );
        px(&out, 256, 256)
    };
    // 1 bit: each channel is either off or full.
    assert_eq!(&run(1)[..3], &[255, 0, 0]);
    // 2 bits: 4 levels (0, 85, 170, 255).
    assert_eq!(&run(2)[..3], &[170, 85, 85]);
    // 8 bits: unchanged.
    let same = run(8);
    assert!(
        same[0].abs_diff(200) <= 1 && same[1].abs_diff(100) <= 1,
        "{same:?}"
    );
}

#[test]
fn shader_filters_run_and_read_their_knobs() {
    let Some(ctx) = ctx() else { return };
    let b = one_pixel(&ctx, [200, 100, 50, 255]);
    let filter = "// @param amount 0 1 0\nfn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {\n    \
                  let inv = vec4<f32>(vec3<f32>(1.0) - color.rgb, color.a);\n    \
                  return mix(color, inv, rm_param(0u)) * vec4<f32>(1.0, 1.0, rm_sample(src_uv).r, p_amount());\n}";
    let out = ctx.render_with(
        TestPattern::Off,
        full(),
        1.0,
        QuadFill::Image(&b),
        extra(ExtraM5 {
            // Posterize first (8 bit: no change), then this one: its
            // parameter is the second value of the chain.
            chain: vec![builtin("posterize"), ("mine".into(), filter.into())],
            params: vec![8.0, 1.0],
            ..Default::default()
        }),
    );
    let p = px(&out, 256, 256);
    // Inverted: 55, 155, 205; blue then scaled by the sampled red (200/255).
    assert!(p[0].abs_diff(55) <= 2 && p[1].abs_diff(155) <= 2, "{p:?}");
    assert!(p[2].abs_diff(161) <= 3, "{p:?}");
}

#[test]
fn broken_shader_filters_report_their_own_line() {
    let Some(ctx) = ctx() else { return };
    let broken = (
        "mine".to_owned(),
        "// my filter\nfn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {\n    return nope(color);\n}".to_owned(),
    );
    let err = ctx
        .quads
        .compile_chain(&ctx.gpu.device, &chain(&[builtin("invert"), broken]))
        .err()
        .expect("must fail");
    assert!(err.starts_with("mine line 3"), "{err}");
    let err = ctx
        .quads
        .compile_chain(
            &ctx.gpu.device,
            &chain(&[("x".into(), "fn other() {}".into())]),
        )
        .err()
        .expect("needs fn effect");
    assert!(err.contains("fn effect"), "{err}");
}

#[test]
fn built_in_shaders_compile_alone_and_chained() {
    let Some(ctx) = ctx() else { return };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/shaders");
    let mut all = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("assets/shaders") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "wgsl") {
            let id = rustmap_core::shaders::shader_id(&path);
            let shader = builtin(&id);
            if let Err(err) = ctx
                .quads
                .compile_chain(&ctx.gpu.device, &chain(std::slice::from_ref(&shader)))
            {
                panic!("{id}: {err}");
            }
            all.push(shader);
        }
    }
    assert!(all.len() >= 8);
    // All of them in one chain: names don't clash, parameters fit.
    ctx.quads
        .compile_chain(&ctx.gpu.device, &chain(&all))
        .expect("whole chain");
}

#[test]
fn own_pass_lets_a_shader_sample_the_chain_above() {
    let Some(ctx) = ctx() else { return };
    let b = one_pixel(&ctx, [200, 100, 50, 255]);
    // Ignores its color and samples instead: sees the input, or with its
    // own pass, the inverted image from the shader above.
    let resample = (
        "resample".to_owned(),
        "fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {\n    return rm_sample(src_uv);\n}".to_owned(),
    );
    let run = |own: Vec<usize>| {
        let out = ctx.render_with(
            TestPattern::Off,
            full(),
            1.0,
            QuadFill::Image(&b),
            extra(ExtraM5 {
                chain: vec![builtin("invert"), resample.clone()],
                params: vec![1.0],
                own_pass: own,
                ..Default::default()
            }),
        );
        px(&out, 256, 256)
    };
    let single = run(vec![]);
    assert!(
        single[0].abs_diff(200) <= 2,
        "one pass samples the input: {single:?}"
    );
    let two = run(vec![1]);
    assert!(
        two[0].abs_diff(55) <= 2 && two[1].abs_diff(155) <= 2,
        "own pass: {two:?}"
    );
    // Three parts (two passes) still work and parameters keep their slots.
    let three = ctx.render_with(
        TestPattern::Off,
        full(),
        1.0,
        QuadFill::Image(&b),
        extra(ExtraM5 {
            chain: vec![builtin("invert"), builtin("invert"), resample.clone()],
            params: vec![1.0, 0.0],
            own_pass: vec![1, 2],
            ..Default::default()
        }),
    );
    let p = px(&three, 256, 256);
    assert!(p[0].abs_diff(55) <= 2, "{p:?}");
}

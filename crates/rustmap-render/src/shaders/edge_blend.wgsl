// Edge blending: multiplies the finished output by a soft ramp on each side
// (blend state dst * src), so overlapping projectors add up evenly.

struct Blend {
    resolution: vec2<f32>,
    gamma: f32,
    curve: f32,
    // left, right, top, bottom ramp widths as fractions of the output.
    widths: vec4<f32>,
};

@group(0) @binding(0) var<uniform> b: Blend;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

// Must match EdgeBlend::ramp in rustmap-core.
fn ramp(t_in: f32) -> f32 {
    let t = clamp(t_in, 0.0, 1.0);
    let p = max(b.curve, 0.1);
    var s: f32;
    if (t < 0.5) {
        s = 0.5 * pow(2.0 * t, p);
    } else {
        s = 1.0 - 0.5 * pow(2.0 * (1.0 - t), p);
    }
    return pow(s, 1.0 / max(b.gamma, 0.1));
}

fn side(dist: f32, width: f32) -> f32 {
    if (width <= 0.0) {
        return 1.0;
    }
    return ramp(dist / width);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / b.resolution;
    let f = side(uv.x, b.widths.x) * side(1.0 - uv.x, b.widths.y)
          * side(uv.y, b.widths.z) * side(1.0 - uv.y, b.widths.w);
    return vec4<f32>(f, f, f, 1.0);
}

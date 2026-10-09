// RUSTMAP projector test patterns.
//
// Everything is computed in output pixels (@builtin(position)), so line widths
// are exact on the projector regardless of resolution.

struct Params {
    resolution: vec2<f32>,
    time: f32,
    pattern: u32,       // 0 off, 1 grid, 2 color bars, 3 solid
    color: vec4<f32>,
    cell: f32,          // grid cell size in px
    index: f32,         // output number, shown as dots
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

// One oversized triangle covering the whole target; no vertex buffer needed.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

// 1 on a line of half-width `w` at distance `d` (px), with 1px anti-aliasing.
fn line(d: f32, w: f32) -> f32 {
    return 1.0 - smoothstep(w - 0.5, w + 0.5, d);
}

// Distance (px) to the nearest grid line of spacing `s`, grid centered on `c`.
fn grid_dist(x: f32, c: f32, s: f32) -> f32 {
    let t = (x - c) / s;
    return abs(t - round(t)) * s;
}

// Distance (px) from point q to segment a-b.
fn seg_dist(q: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = q - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn grid(px: vec2<f32>) -> vec3<f32> {
    let res = p.resolution;
    let c = res * 0.5;
    let s = max(p.cell, 8.0);
    var col = vec3<f32>(0.06);

    // Minor lines at half cells, major lines at full cells, aligned to the center.
    let minor = max(line(grid_dist(px.x, c.x, s * 0.5), 0.5), line(grid_dist(px.y, c.y, s * 0.5), 0.5));
    col = mix(col, vec3<f32>(0.22), minor);
    let major = max(line(grid_dist(px.x, c.x, s), 1.0), line(grid_dist(px.y, c.y, s), 1.0));
    col = mix(col, vec3<f32>(0.7), major);

    // Diagonals: keystone shows up as bent or off-center crossings.
    let diag = min(seg_dist(px, vec2<f32>(0.0), res), seg_dist(px, vec2<f32>(res.x, 0.0), vec2<f32>(0.0, res.y)));
    col = mix(col, vec3<f32>(0.45, 0.45, 0.55), line(diag, 0.75));

    // Circles: should look round on the wall when aspect is right.
    let r_big = 0.45 * min(res.x, res.y);
    col = mix(col, vec3<f32>(1.0), line(abs(length(px - c) - r_big), 1.5));
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(s, s), vec2<f32>(res.x - s, s), vec2<f32>(s, res.y - s), vec2<f32>(res.x - s, res.y - s)
    );
    for (var k = 0; k < 4; k++) {
        col = mix(col, vec3<f32>(0.9), line(abs(length(px - corners[k]) - s * 0.5), 1.0));
    }

    // Center cross.
    let cross = min(abs(px.x - c.x), abs(px.y - c.y));
    col = mix(col, vec3<f32>(1.0, 0.85, 0.1), line(cross, 1.5));

    // Corner tags for orientation: TL red, TR green, BL blue, BR magenta.
    let tag = s * 0.6;
    if (px.x + px.y < tag) { col = vec3<f32>(1.0, 0.15, 0.15); }
    if ((res.x - px.x) + px.y < tag) { col = vec3<f32>(0.15, 1.0, 0.25); }
    if (px.x + (res.y - px.y) < tag) { col = vec3<f32>(0.2, 0.4, 1.0); }
    if ((res.x - px.x) + (res.y - px.y) < tag) { col = vec3<f32>(1.0, 0.2, 1.0); }

    // Output number as a row of dots, top-left, next to the red tag.
    let dot_r = s * 0.12;
    for (var k = 0; k < 16; k++) {
        if (f32(k) >= p.index) { break; }
        let center = vec2<f32>(s * 0.9 + f32(k) * dot_r * 3.0, s * 0.35);
        if (length(px - center) < dot_r) { col = vec3<f32>(1.0, 0.85, 0.1); }
    }

    // Sweeping bar: stutter, dropped frames and tearing are obvious on it.
    let bar_x = fract(p.time * 0.25) * (res.x + s) - s * 0.5;
    let in_bar = 1.0 - smoothstep(s * 0.25 - 0.5, s * 0.25 + 0.5, abs(px.x - bar_x));
    col = mix(col, vec3<f32>(0.1, 0.8, 0.9), in_bar * 0.5);

    // Border: the very edge pixels, so you can see if anything is cropped.
    let edge = min(min(px.x, res.x - px.x), min(px.y, res.y - px.y));
    col = mix(col, vec3<f32>(1.0), line(edge, 2.0));

    return col;
}

fn color_bars(px: vec2<f32>) -> vec3<f32> {
    let res = p.resolution;
    let u = px.x / res.x;
    let v = px.y / res.y;
    if (v < 0.66) {
        let bars = array<vec3<f32>, 8>(
            vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(1.0, 1.0, 0.0), vec3<f32>(0.0, 1.0, 1.0), vec3<f32>(0.0, 1.0, 0.0),
            vec3<f32>(1.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, 0.0)
        );
        return bars[min(u32(u * 8.0), 7u)];
    }
    if (v < 0.83) {
        // 16-step gray ramp: count the visible steps to judge black/white clipping.
        return vec3<f32>(floor(u * 16.0) / 15.0);
    }
    return vec3<f32>(u);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = in.pos.xy;
    var col = vec3<f32>(0.0);
    switch p.pattern {
        case 1u: { col = grid(px); }
        case 2u: { col = color_bars(px); }
        case 3u: { col = p.color.rgb; }
        default: { col = vec3<f32>(0.0); }
    }
    return vec4<f32>(col, 1.0);
}

// LED sampling: one output texel per LED, holding the color of the output
// image at that LED's position (averaged over a small area when asked).

struct Params {
    width: u32,
    count: u32,
    _pad: vec2<u32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
// x, y, radius_x, radius_y in normalized output space.
@group(0) @binding(2) var<storage, read> points: array<vec4<f32>>;
@group(0) @binding(3) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

fn tap(p: vec2<f32>) -> vec3<f32> {
    // Off the canvas is dark, like the projector.
    if (any(p < vec2<f32>(0.0)) || any(p > vec2<f32>(1.0))) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(src, samp, p, 0.0).rgb;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let idx = u32(pos.y) * params.width + u32(pos.x);
    if (idx >= params.count) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let pt = points[idx];
    let r = pt.zw;
    if (r.x <= 0.0 && r.y <= 0.0) {
        return vec4<f32>(tap(pt.xy), 1.0);
    }
    // 4 x 4 taps spread over the area; with bilinear filtering that covers it well.
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let o = (vec2<f32>(f32(i), f32(j)) + 0.5) / 4.0 * 2.0 - 1.0;
            sum += tap(pt.xy + o * r);
        }
    }
    return vec4<f32>(sum / 16.0, 1.0);
}

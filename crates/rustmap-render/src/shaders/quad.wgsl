// A textured surface: a perspective quad or a tessellated mesh warp.
//
// Perspective quads: the CPU gives each corner its homogeneous W from the
// unit-square homography and writes clip position = (ndc * W, 0, W), so the
// GPU's perspective-correct interpolation does the projective mapping and
// both triangles agree exactly. Meshes arrive pre-tessellated (W = 1).

struct Surface {
    opacity: f32,
    fill: u32,          // 0 image, 1 checker, 2 uv debug
    shape: u32,         // 0 quad, 1 triangle, 2 circle
    mask_count: u32,    // 0 = no mask, else polygon points used
    mask_feather: f32,  // soft edge width in surface uv units
    mask_invert: u32,
    time: f32,          // seconds, for shader filters
    _pad: f32,
    // Shader filter parameters of the whole chain, four per vec4.
    params: array<vec4<f32>, 8>,
    // Homography from surface uv to media uv (input region), as matrix columns.
    input: array<vec4<f32>, 3>,
    // Mask polygon in surface uv, two points per vec4.
    mask: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> q: Surface;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = in.pos;
    out.uv = in.uv;
    return out;
}

// Placeholder for surfaces without media: makes perspective obvious.
fn checker(uv: vec2<f32>) -> vec3<f32> {
    let cells = floor(uv * 8.0);
    let odd = (i32(cells.x) + i32(cells.y)) % 2 == 1;
    var col = select(vec3<f32>(0.32), vec3<f32>(0.62), odd);
    // Quadrant tints, matching the output's corner colors.
    let right = uv.x >= 0.5;
    let bottom = uv.y >= 0.5;
    var tint = vec3<f32>(1.0, 0.45, 0.45);                       // TL red
    if (right && !bottom) { tint = vec3<f32>(0.45, 1.0, 0.5); }    // TR green
    if (!right && bottom) { tint = vec3<f32>(0.5, 0.6, 1.0); }     // BL blue
    if (right && bottom) { tint = vec3<f32>(1.0, 0.5, 1.0); }      // BR magenta
    col = col * tint;

    // Border and center cross, ~2 px wide whatever the warp.
    let px = fwidth(uv);
    let edge = min(min(uv.x, 1.0 - uv.x) / px.x, min(uv.y, 1.0 - uv.y) / px.y);
    let cross = min(abs(uv.x - 0.5) / px.x, abs(uv.y - 0.5) / px.y);
    let line = 1.0 - smoothstep(1.5, 2.5, min(edge, cross));
    return mix(col, vec3<f32>(1.0), line);
}

// Coverage of the shape mask at `uv`, anti-aliased over about one pixel.
fn shape_mask(uv: vec2<f32>) -> f32 {
    var d = 1.0;
    switch q.shape {
        // Triangle: apex at top middle, base along the bottom edge.
        case 1u: { d = 0.5 * uv.y - abs(uv.x - 0.5); }
        // Ellipse touching all four edges.
        case 2u: { d = 0.5 - length(uv - vec2<f32>(0.5)); }
        default: { return 1.0; }
    }
    return clamp(d / max(fwidth(d), 1e-6) + 0.5, 0.0, 1.0);
}

fn mask_point(i: u32) -> vec2<f32> {
    let v = q.mask[i / 2u];
    return select(v.xy, v.zw, (i % 2u) == 1u);
}

// Coverage of the polygon mask at `uv`: 1 inside, 0 outside, with a soft
// edge `mask_feather` wide fading inwards from the outline.
fn mask_coverage(uv: vec2<f32>) -> f32 {
    let n = q.mask_count;
    if (n < 3u) {
        return 1.0;
    }
    var inside = false;
    var dist = 1e9;
    var j = n - 1u;
    for (var i = 0u; i < n; i++) {
        let a = mask_point(i);
        let b = mask_point(j);
        if ((a.y > uv.y) != (b.y > uv.y)) && (uv.x < (b.x - a.x) * (uv.y - a.y) / (b.y - a.y) + a.x) {
            inside = !inside;
        }
        let ab = b - a;
        let t = clamp(dot(uv - a, ab) / max(dot(ab, ab), 1e-12), 0.0, 1.0);
        dist = min(dist, length(uv - (a + ab * t)));
        j = i;
    }
    let d = select(-dist, dist, inside);
    var cov: f32;
    if (q.mask_feather > 0.0) {
        cov = smoothstep(0.0, q.mask_feather, d);
    } else {
        // Hard edge, anti-aliased over about a pixel.
        cov = clamp(d / max(length(fwidth(uv)), 1e-6) + 0.5, 0.0, 1.0);
    }
    return select(cov, 1.0 - cov, q.mask_invert != 0u);
}

// ---- for shader filters ----

// Seconds since RUSTMAP started.
fn rm_time() -> f32 {
    return q.time;
}

// Parameter `n` of the whole chain (each shader gets its own `rm_param`
// and `p_<name>` accessors that add their offset).
fn rm_p(n: u32) -> f32 {
    let i = min(n, 31u);
    return q.params[i / 4u][i % 4u];
}

// The input at media uv `uv` (straight alpha). Safe anywhere in a filter.
fn rm_sample(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(tex, samp, uv, 0.0);
    if (c.a > 0.0) {
        return vec4<f32>(c.rgb / c.a, c.a);
    }
    return vec4<f32>(0.0);
}

// Size of the input in pixels.
fn rm_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(tex, 0));
}

// RUSTMAP_FILTER_BEGIN
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    return color;
}
// RUSTMAP_FILTER_END

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Surface uv -> media uv through the input region's homography.
    let m = mat3x3<f32>(q.input[0].xyz, q.input[1].xyz, q.input[2].xyz);
    let h = m * vec3<f32>(in.uv, 1.0);
    let src_uv = h.xy / h.z;
    // Sample unconditionally: derivatives must be in uniform control flow.
    let image = textureSample(tex, samp, src_uv);
    let mask = shape_mask(in.uv) * mask_coverage(in.uv);
    var color: vec4<f32>;
    switch q.fill {
        case 1u: { color = vec4<f32>(checker(in.uv), 1.0); }
        case 2u: { color = vec4<f32>(in.uv, 0.0, 1.0); }
        default: { color = image; }  // premultiplied
    }
    // Shader filters work on straight alpha; colors here are premultiplied.
    var straight = vec4<f32>(0.0);
    if (color.a > 0.0) {
        straight = vec4<f32>(color.rgb / color.a, color.a);
    }
    if (q.fill == 0u) {
        straight = effect(straight, in.uv, src_uv);
    }
    let a = clamp(straight.a, 0.0, 1.0);
    color = vec4<f32>(clamp(straight.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * a, a);
    return color * (q.opacity * mask);
}

// ---- own-pass shaders ----
// A shader marked "own pass" sees everything above it baked into an image:
// those shaders run here, as one full-frame triangle in input space
// (uv = src_uv), into an image the size of the input.

struct PassOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_pass(@builtin(vertex_index) i: u32) -> PassOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PassOut;
    out.pos = vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    out.uv = p;
    return out;
}

@fragment
fn fs_pass(in: PassOut) -> @location(0) vec4<f32> {
    let r = effect(rm_sample(in.uv), in.uv, in.uv);
    let a = clamp(r.a, 0.0, 1.0);
    return vec4<f32>(clamp(r.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * a, a);
}

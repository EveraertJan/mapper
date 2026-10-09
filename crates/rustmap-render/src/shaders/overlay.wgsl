// Flat-colored triangles in window pixels: guides and handles on top of a
// projector window. Never part of the output image itself.

struct Screen {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let ndc = in.pos / screen.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}

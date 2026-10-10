// Big pixels. A: pixel size (1 to 64 input pixels).
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let size = mix(1.0, 64.0, rm_param(0u));
    let cells = rm_size() / size;
    let center = (floor(src_uv * cells) + vec2<f32>(0.5)) / cells;
    return rm_sample(center);
}

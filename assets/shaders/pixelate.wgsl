// @name Pixelate
// @description Big pixels.
// @param size "Pixel size" 1 64 8 step=1

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let cells = rm_size() / max(p_size(), 1.0);
    let center = (floor(src_uv * cells) + vec2<f32>(0.5)) / cells;
    return rm_sample(center);
}

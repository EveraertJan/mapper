// @name RGB split
// @description Chromatic aberration: red and blue pulled apart.
// @param distance "Distance" 0 1 0.3

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let d = vec2<f32>(p_distance() * 0.02, 0.0);
    return vec4<f32>(rm_sample(src_uv + d).r, color.g, rm_sample(src_uv - d).b, color.a);
}

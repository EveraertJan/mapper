// Chromatic aberration: red and blue pulled apart. A: distance.
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let d = vec2<f32>(rm_param(0u) * 0.02, 0.0);
    let r = rm_sample(src_uv + d).r;
    let b = rm_sample(src_uv - d).b;
    return vec4<f32>(r, color.g, b, color.a);
}

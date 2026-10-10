// Negative image. A: amount (0 = original, 1 = fully inverted).
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let inverted = vec4<f32>(vec3<f32>(1.0) - color.rgb, color.a);
    return mix(color, inverted, rm_param(0u));
}

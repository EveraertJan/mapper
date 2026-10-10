// Animated wobble. A: strength. B: speed.
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let t = rm_time() * mix(0.2, 6.0, rm_param(1u));
    let strength = rm_param(0u) * 0.05;
    let offset = vec2<f32>(sin(src_uv.y * 12.0 + t), cos(src_uv.x * 9.0 + t * 1.3)) * strength;
    return rm_sample(src_uv + offset);
}

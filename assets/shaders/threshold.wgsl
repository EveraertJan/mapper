// Black and white. A: threshold (dark to bright).
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let luma = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let v = step(rm_param(0u), luma);
    return vec4<f32>(vec3<f32>(v), color.a);
}

// @name Threshold
// @description Black and white.
// @param level "Level" 0 1 0.5

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let luma = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(vec3<f32>(step(p_level(), luma)), color.a);
}

// @name Scanlines
// @description CRT-style dark lines across the surface.
// @param darkness "Darkness" 0 1 0.5
// @param lines "Lines" 20 800 240 step=1

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let wave = 0.5 + 0.5 * sin(uv.y * p_lines() * 6.2831853);
    return vec4<f32>(color.rgb * (1.0 - p_darkness() * wave), color.a);
}

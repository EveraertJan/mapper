// @name Wave
// @description Animated wobble.
// @param strength "Strength" 0 1 0.3
// @param speed "Speed" 0 6 1

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let t = rm_time() * p_speed();
    let s = p_strength() * 0.05;
    let offset = vec2<f32>(sin(src_uv.y * 12.0 + t), cos(src_uv.x * 9.0 + t * 1.3)) * s;
    return rm_sample(src_uv + offset);
}

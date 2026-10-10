// @name Posterize
// @description Fewer levels per color channel: 1 bit = 2 levels, 2 bit = 4, 6 bit = 64.
// @param bits "Bits per channel" 1 8 4 step=1

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let steps = exp2(round(p_bits())) - 1.0;
    let rgb = floor(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * steps + vec3<f32>(0.5)) / steps;
    return vec4<f32>(rgb, color.a);
}

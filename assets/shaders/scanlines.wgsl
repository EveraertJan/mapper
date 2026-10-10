// CRT-style scanlines. A: how dark the lines are. B: number of lines.
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let lines = mix(40.0, 600.0, rm_param(1u));
    let wave = 0.5 + 0.5 * sin(uv.y * lines * 6.2831853);
    let dark = 1.0 - rm_param(0u) * wave;
    return vec4<f32>(color.rgb * dark, color.a);
}

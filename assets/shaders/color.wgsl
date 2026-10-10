// @name Color
// @description Brightness, contrast, saturation, hue and per-channel gain.
// @param brightness "Brightness" -1 1 0
// @param contrast "Contrast" 0 2 1
// @param saturation "Saturation" 0 2 1
// @param hue "Hue" -180 180 0 step=1
// @param red "Red" 0 2 1
// @param green "Green" 0 2 1
// @param blue "Blue" 0 2 1

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    var rgb = color.rgb * vec3<f32>(p_red(), p_green(), p_blue());
    rgb = rgb + vec3<f32>(p_brightness());
    rgb = (rgb - vec3<f32>(0.5)) * p_contrast() + vec3<f32>(0.5);
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3<f32>(luma), rgb, p_saturation());
    let angle = radians(p_hue());
    if (angle != 0.0) {
        // Rotate around the gray axis.
        let k = vec3<f32>(0.57735);
        rgb = rgb * cos(angle) + cross(k, rgb) * sin(angle) + k * dot(k, rgb) * (1.0 - cos(angle));
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
}

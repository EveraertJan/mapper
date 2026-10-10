# Shaders

Everything in the **Filters** column is a shader: each surface has a chain of them, run top to bottom, each with its own sliders. The ones in this folder are built in (Color, Posterize, Invert, Pixelate, Scanlines, Wave, RGB split, Threshold). Shaders you add go into your shader library and are available in every show until you remove them.

- **+ Add shader** on a surface picks from the library.
- **Library → Add file…** copies a `.wgsl` file into your library (on macOS `~/Library/Application Support/RUSTMAP/Shaders`); **New shader…** creates one from a template there; **Open folder** shows it. Edit a library file and every surface using it updates when you save. Errors show in the Filters column with the shader's name and its own line number; until the file is fixed the surface draws without that chain.
- A file in your library with the same name as a built-in one replaces it.
- A show remembers the source of the custom shaders it uses, so it opens the same on another machine; *Add to library* there keeps them.

## Writing a shader

```wgsl
// @name Tint
// @description Pushes the picture towards a color.
// @param amount "Amount" 0 1 0.5
// @param hue "Hue" 0 360 200 step=1

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    let tint = vec3<f32>(0.5 + 0.5 * cos(radians(p_hue()) + vec3<f32>(0.0, 2.09, 4.19)));
    return vec4<f32>(mix(color.rgb, color.rgb * tint, p_amount()), color.a);
}
```

- `effect` gets the color at this pixel (straight alpha, 0..1), `uv` (position on the surface, 0..1 from the top-left) and `src_uv` (position in the input after the input region), and returns the new color.
- `// @param id "Label" min max default [step=s]` makes a slider; read it with `p_id()`, or `rm_param(n)` by position. Up to 8 per shader, 32 per surface. `step=1` gives whole numbers.
- `// @name` and `// @description` are what the Filters column shows.
- Helpers: `rm_time()` (seconds, for animation), `rm_sample(uv)` (the input at any position, straight alpha), `rm_size()` (input size in pixels).
- The file name (without `.wgsl`) is the shader's id. Your own helper functions and constants are fine; each shader in a chain gets its own copy of them.
- `rm_sample` reads the input, not the result of shaders earlier in the chain (the whole chain runs in one pass).
- `filter` is a reserved word in WGSL, so the function is called `effect`.

# Shader filters

A shader filter is a WGSL file that changes the picture of a surface. Pick one in the **Filters** column (*Shader… → Choose file*). RUSTMAP recompiles the file as soon as you save it, so you can edit it while watching the output; errors show in the Filters column with the line number in your file.

## The one function you write

```wgsl
fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    return color;
}
```

- `color`: the input's color at this pixel, straight alpha (not premultiplied), 0..1.
- `uv`: position on the surface, 0..1 from the top-left corner.
- `src_uv`: position in the input (the media), 0..1, after the input region.
- Return the new color, straight alpha.

The filter runs before the color adjustments and posterize in the Filters column.

## Helpers you can call

| Function | Gives |
|---|---|
| `rm_time()` | seconds since RUSTMAP started (for animation) |
| `rm_param(0u)` … `rm_param(3u)` | the four knobs A–D in the Filters column, 0..1 |
| `rm_sample(src_uv)` | the input at any position (straight alpha); use it for blurs, offsets, pixelation |
| `rm_size()` | the input's size in pixels |

You can add your own functions and constants in the same file. Names starting with `rm_`, `vs_main`, `fs_main` and the shader's own `q`, `tex`, `samp` are taken. `filter` is a reserved word in WGSL, which is why the function is called `effect`.

## Examples here

- `invert.wgsl`: negative image; A = amount.
- `pixelate.wgsl`: big pixels; A = pixel size.
- `scanlines.wgsl`: CRT-style lines; A = darkness, B = line count.
- `wave.wgsl`: animated wobble; A = strength, B = speed.
- `rgb_split.wgsl`: chromatic aberration; A = distance.
- `threshold.wgsl`: black and white; A = threshold.

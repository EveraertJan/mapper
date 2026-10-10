//! Shader filters: every surface has a chain of shaders, each with its own
//! parameters (sliders). A shader is a WGSL file defining
//! `fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32>`;
//! comments at its top name it and declare its parameters:
//!
//! ```text
//! // @name Posterize
//! // @description Fewer levels per color channel.
//! // @param bits "Bits per channel" 1 8 4 step=1
//! ```
//!
//! `@param id "Label" min max default [step=s]`: the shader reads the value
//! with `p_id()` (or `rm_param(n)` by position). Up to [`MAX_PARAMS`] per
//! shader and [`MAX_CHAIN_PARAMS`] per surface.

use serde::{Deserialize, Serialize};

use crate::surface::ColorAdjust;

/// Parameters one shader may declare.
pub const MAX_PARAMS: usize = 8;
/// Parameters a surface's whole chain may use (GPU uniform space).
pub const MAX_CHAIN_PARAMS: usize = 32;

/// One shader in a surface's chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterUse {
    /// Library id: the file name without `.wgsl` (e.g. `"posterize"`).
    pub shader: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Parameter values, in the order the shader declares them.
    #[serde(default)]
    pub params: Vec<f32>,
    /// Render the shaders above into an image first, so this one's
    /// `rm_sample` reads their result (costs a render pass).
    #[serde(default)]
    pub own_pass: bool,
}

fn yes() -> bool {
    true
}

/// A declared parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
    pub id: String,
    pub label: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// 0 = continuous.
    pub step: f32,
}

/// What a shader file's header says about it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShaderInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<ParamDef>,
}

impl FilterUse {
    /// A new chain entry with the shader's default values.
    pub fn new(info: &ShaderInfo) -> Self {
        Self {
            shader: info.id.clone(),
            enabled: true,
            params: info.params.iter().map(|p| p.default).collect(),
            own_pass: false,
        }
    }

    /// Keeps `params` matching the shader's declaration (after the file
    /// changed): missing values get defaults, extra ones are dropped.
    pub fn fit(&mut self, info: &ShaderInfo) {
        self.params.truncate(info.params.len());
        for p in &info.params[self.params.len()..] {
            self.params.push(p.default);
        }
        for (v, p) in self.params.iter_mut().zip(&info.params) {
            *v = v.clamp(p.min.min(p.max), p.max.max(p.min));
        }
    }

    /// Value of parameter `i`, falling back to the default.
    pub fn value(&self, info: &ShaderInfo, i: usize) -> f32 {
        self.params
            .get(i)
            .copied()
            .or_else(|| info.params.get(i).map(|p| p.default))
            .unwrap_or_default()
    }
}

/// Library id for a shader file: its file name without extension, lower
/// case, anything but letters, digits and `_` turned into `_`.
pub fn shader_id(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Reads the `@name`, `@description` and `@param` comments of a shader.
pub fn parse_header(id: &str, source: &str) -> Result<ShaderInfo, String> {
    let mut info = ShaderInfo {
        id: id.to_owned(),
        name: id.replace('_', " "),
        ..Default::default()
    };
    for (n, line) in source.lines().enumerate() {
        let Some(rest) = line.trim().strip_prefix("//") else {
            continue;
        };
        let rest = rest.trim();
        let err = |msg: &str| format!("line {}: {msg}", n + 1);
        if let Some(v) = rest.strip_prefix("@name") {
            info.name = v.trim().to_owned();
        } else if let Some(v) = rest.strip_prefix("@description") {
            if !info.description.is_empty() {
                info.description.push(' ');
            }
            info.description.push_str(v.trim());
        } else if let Some(v) = rest.strip_prefix("@param") {
            if info.params.len() == MAX_PARAMS {
                return Err(err(&format!("at most {MAX_PARAMS} parameters")));
            }
            info.params.push(parse_param(v).map_err(|e| err(&e))?);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for p in &info.params {
        if !seen.insert(p.id.as_str()) {
            return Err(format!("parameter `{}` is declared twice", p.id));
        }
    }
    Ok(info)
}

/// `id "Label" min max default [step=s]` (the label is optional).
fn parse_param(text: &str) -> Result<ParamDef, String> {
    let usage = "expected: @param id \"Label\" min max default [step=s]";
    let text = text.trim();
    let (id, rest) = text.split_once(char::is_whitespace).ok_or(usage)?;
    if id.is_empty()
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || id.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(format!("`{id}` is not a valid parameter name"));
    }
    let rest = rest.trim();
    let (label, rest) = match rest.strip_prefix('"') {
        Some(r) => {
            let (label, after) = r.split_once('"').ok_or("unclosed quote in the label")?;
            (label.to_owned(), after)
        }
        None => (id.replace('_', " "), rest),
    };
    let mut nums = Vec::new();
    let mut step = 0.0;
    for word in rest.split_whitespace() {
        if let Some(s) = word.strip_prefix("step=") {
            step = s.parse().map_err(|_| format!("bad step `{s}`"))?;
        } else {
            nums.push(
                word.parse::<f32>()
                    .map_err(|_| format!("bad number `{word}`"))?,
            );
        }
    }
    let [min, max, default] = nums[..] else {
        return Err(usage.into());
    };
    Ok(ParamDef {
        id: id.to_owned(),
        label,
        min,
        max,
        default: default.clamp(min.min(max), max.max(min)),
        step,
    })
}

/// The chain that reproduces an old per-surface color adjustment (shows
/// saved before color became a shader).
pub fn chain_from_color(c: &ColorAdjust) -> Vec<FilterUse> {
    let mut out = Vec::new();
    let plain = ColorAdjust { posterize: 0, ..*c };
    if !plain.is_identity() {
        out.push(FilterUse {
            shader: "color".into(),
            enabled: true,
            own_pass: false,
            params: vec![
                c.brightness,
                c.contrast,
                c.saturation,
                c.hue,
                c.gain[0],
                c.gain[1],
                c.gain[2],
            ],
        });
    }
    if c.posterize > 0 {
        out.push(FilterUse {
            shader: "posterize".into(),
            enabled: true,
            params: vec![f32::from(c.posterize)],
            own_pass: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_declares_name_and_params() {
        let src = "// @name Posterize\n// @description Fewer levels.\n// @param bits \"Bits per channel\" 1 8 4 step=1\n// @param mix 0 1 1\nfn effect() {}";
        let info = parse_header("posterize", src).unwrap();
        assert_eq!(info.name, "Posterize");
        assert_eq!(info.description, "Fewer levels.");
        assert_eq!(info.params.len(), 2);
        assert_eq!(info.params[0].label, "Bits per channel");
        assert_eq!(info.params[0].step, 1.0);
        assert_eq!(info.params[1].label, "mix");
        assert_eq!(info.params[1].default, 1.0);
        let f = FilterUse::new(&info);
        assert_eq!(f.params, vec![4.0, 1.0]);
    }

    #[test]
    fn bad_headers_say_where() {
        assert!(
            parse_header("x", "// @param a 0 1")
                .unwrap_err()
                .starts_with("line 1")
        );
        assert!(
            parse_header("x", "\n// @param 9a 0 1 0")
                .unwrap_err()
                .starts_with("line 2")
        );
        assert!(parse_header("x", "// @param a 0 1 0\n// @param a 0 1 0").is_err());
        // No header at all is fine: a shader without parameters.
        assert_eq!(
            parse_header("my_fx", "fn effect() {}").unwrap().name,
            "my fx"
        );
    }

    #[test]
    fn fit_follows_edited_declarations() {
        let info = parse_header("x", "// @param a 0 1 0.5\n// @param b 0 10 2").unwrap();
        let mut f = FilterUse {
            shader: "x".into(),
            enabled: true,
            params: vec![7.0],
            own_pass: false,
        };
        f.fit(&info);
        assert_eq!(f.params, vec![1.0, 2.0]);
    }

    #[test]
    fn ids_come_from_file_names() {
        assert_eq!(
            shader_id(std::path::Path::new("/a/RGB Split.wgsl")),
            "rgb_split"
        );
    }

    #[test]
    fn old_color_adjustments_become_a_chain() {
        let c = ColorAdjust {
            saturation: 0.5,
            posterize: 3,
            ..Default::default()
        };
        let chain = chain_from_color(&c);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].shader, "color");
        assert_eq!(chain[0].params[2], 0.5);
        assert_eq!(chain[1].params, vec![3.0]);
        assert!(chain_from_color(&ColorAdjust::default()).is_empty());
    }
}

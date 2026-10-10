//! Turns a surface's shader chain into one WGSL module.
//!
//! Each shader is pasted in with its own names suffixed (`effect` becomes
//! `effect__rm0`, a helper `luma` becomes `luma__rm0`, …) so two shaders can
//! use the same names, gets `rm_param(k)` / `p_<name>()` accessors that read
//! its own slice of the parameter block, and a final `effect` runs them in
//! order. Without "own pass" shaders that is one draw with no intermediate
//! images, and `rm_sample` sees the input; a shader marked own pass starts a
//! new part of the chain that reads the earlier parts' result.

use rustmap_core::shaders::MAX_CHAIN_PARAMS;

/// One shader of a chain.
pub struct ChainShader<'a> {
    /// Library id, for messages.
    pub id: &'a str,
    pub source: &'a str,
    /// Declared parameter names, in order.
    pub params: Vec<String>,
    /// Bake everything above into an image first, so this shader's
    /// `rm_sample` reads it (an extra render pass).
    pub own_pass: bool,
}

/// Where each shader's own lines sit in the composed module.
pub(crate) struct LineMap {
    /// (first line, line count, shader id)
    blocks: Vec<(usize, usize, String)>,
}

impl LineMap {
    /// "posterize line 4", or "built-in code".
    pub(crate) fn describe(&self, line: usize) -> String {
        for (first, count, id) in &self.blocks {
            if line >= *first && line < first + count {
                return format!("{id} line {}", line + 1 - first);
            }
        }
        "built-in code".into()
    }
}

/// Words that introduce a name the shader declares.
const DECLARERS: [&str; 6] = ["fn", "const", "var", "let", "override", "struct"];

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Identifiers of `source`, with their byte ranges (comments skipped).
fn idents(source: &str) -> Vec<(usize, usize)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c == '/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if is_ident_start(c) {
            let start = i;
            while i < b.len() && is_ident(b[i] as char) {
                i += 1;
            }
            out.push((start, i));
        } else if c.is_ascii_digit() {
            // Numbers like 1e5 or 0x1fu aren't names.
            while i < b.len() && (is_ident(b[i] as char) || b[i] == b'.') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Names `source` declares: whatever follows `fn`, `const`, `var`
/// (also `var<private>`), `let`, `override` or `struct`.
fn declared(source: &str) -> Vec<String> {
    let ids = idents(source);
    let mut out = Vec::new();
    for (k, &(s, e)) in ids.iter().enumerate() {
        if !DECLARERS.contains(&&source[s..e]) {
            continue;
        }
        // `var<private> x`: skip the address space inside <...>.
        let mut next = k + 1;
        if &source[s..e] == "var" && source[e..].trim_start().starts_with('<') {
            let close = source[e..].find('>').map(|p| e + p);
            while let Some(&(ns, _)) = ids.get(next) {
                if close.is_some_and(|c| ns < c) {
                    next += 1;
                } else {
                    break;
                }
            }
        }
        if let Some(&(ns, ne)) = ids.get(next) {
            out.push(source[ns..ne].to_owned());
        }
    }
    out
}

/// `source` with every name in `names` suffixed by `suffix`.
fn rename(source: &str, names: &[String], suffix: &str) -> String {
    let mut out = String::with_capacity(source.len() + 64);
    let mut last = 0;
    for (s, e) in idents(source) {
        // `color.rgb`: a member or swizzle, never one of the shader's names.
        let member = source[..s].trim_end().ends_with('.');
        if !member && names.iter().any(|n| n == &source[s..e]) {
            out.push_str(&source[last..e]);
            out.push_str(suffix);
            last = e;
        }
    }
    out.push_str(&source[last..]);
    out
}

/// The body that replaces the default `effect` in `quad.wgsl`.
/// `offset`: where this part of the chain's parameters start in the block.
pub(crate) fn compose(
    chain: &[ChainShader<'_>],
    first_line: usize,
    offset: usize,
) -> Result<(String, LineMap), String> {
    let mut out = String::new();
    let mut blocks = Vec::new();
    let mut line = first_line;
    let mut offset = offset;
    let mut calls = String::new();
    for (i, shader) in chain.iter().enumerate() {
        if !shader.source.contains("fn effect(") {
            return Err(format!(
                "{}: the file must define fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32>",
                shader.id
            ));
        }
        let n = shader.params.len();
        if offset + n > MAX_CHAIN_PARAMS {
            return Err(format!(
                "this chain needs more than {MAX_CHAIN_PARAMS} parameters; remove a shader"
            ));
        }
        let suffix = format!("__rm{i}");
        let mut names = declared(shader.source);
        names.push("rm_param".into());
        names.extend(shader.params.iter().map(|p| format!("p_{p}")));
        names.sort();
        names.dedup();
        let body = rename(shader.source, &names, &suffix);
        blocks.push((line, body.lines().count(), shader.id.to_owned()));
        line += body.lines().count() + 1;
        out.push_str(&body);
        out.push('\n');

        // Accessors for this shader's slice of the parameter block.
        let last = n.saturating_sub(1);
        let mut acc = if n == 0 {
            format!("fn rm_param{suffix}(k: u32) -> f32 {{ return 0.0; }}\n")
        } else {
            format!(
                "fn rm_param{suffix}(k: u32) -> f32 {{ return rm_p({offset}u + min(k, {last}u)); }}\n"
            )
        };
        for (j, p) in shader.params.iter().enumerate() {
            acc.push_str(&format!(
                "fn p_{p}{suffix}() -> f32 {{ return rm_p({}u); }}\n",
                offset + j
            ));
        }
        line += acc.lines().count();
        out.push_str(&acc);
        calls.push_str(&format!("    c = effect{suffix}(c, uv, src_uv);\n"));
        offset += n;
    }
    out.push_str(&format!(
        "fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {{\n    var c = color;\n{calls}    return c;\n}}\n"
    ));
    Ok((out, LineMap { blocks }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_names_are_found() {
        let src = "// fn not_this()\nconst K = 2.0;\nvar<private> v: f32;\nfn helper(x: f32) -> f32 { let y = x; return y; }\nfn effect(c: vec4<f32>) -> vec4<f32> { return c; }";
        let mut d = declared(src);
        d.sort();
        assert_eq!(d, ["K", "effect", "helper", "v", "y"]);
    }

    #[test]
    fn renaming_keeps_shaders_apart() {
        let src = "fn luma(c: vec3<f32>) -> f32 { return c.x; }\nfn effect(c: vec4<f32>) -> vec4<f32> { return c * luma(c.rgb) * p_amount() * rm_param(0u); }";
        let chain = [
            ChainShader {
                id: "a",
                source: src,
                params: vec!["amount".into()],
                own_pass: false,
            },
            ChainShader {
                id: "b",
                source: src,
                params: vec!["amount".into()],
                own_pass: false,
            },
        ];
        let (out, map) = compose(&chain, 100, 0).unwrap();
        assert!(out.contains("fn luma__rm0(") && out.contains("fn luma__rm1("));
        assert!(out.contains("luma__rm1(c.rgb) * p_amount__rm1() * rm_param__rm1(0u)"));
        // Each shader reads its own slot.
        assert!(out.contains("fn p_amount__rm0() -> f32 { return rm_p(0u); }"));
        assert!(out.contains("fn p_amount__rm1() -> f32 { return rm_p(1u); }"));
        assert!(
            out.contains("c = effect__rm0(c, uv, src_uv);\n    c = effect__rm1(c, uv, src_uv);")
        );
        assert_eq!(map.describe(101), "a line 2");
        assert_eq!(map.describe(99), "built-in code");
    }

    #[test]
    fn members_and_swizzles_are_not_renamed() {
        let out = rename(
            "let rgb = c.rgb; let x = s . rgb;",
            &["rgb".into()],
            "__rm0",
        );
        assert_eq!(out, "let rgb__rm0 = c.rgb; let x = s . rgb;");
    }

    #[test]
    fn too_many_parameters_is_an_error() {
        let src = "fn effect(c: vec4<f32>) -> vec4<f32> { return c; }";
        let params: Vec<String> = (0..8).map(|i| format!("p{i}")).collect();
        let chain: Vec<_> = (0..5)
            .map(|_| ChainShader {
                id: "x",
                source: src,
                params: params.clone(),
                own_pass: false,
            })
            .collect();
        assert!(compose(&chain, 0, 0).is_err());
    }
}

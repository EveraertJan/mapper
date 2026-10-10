//! The shader library and the compiled shader chains of every surface.
//!
//! Shaders come from three places, later ones winning for the same id:
//! the built-ins (compiled into the app), the source copies a show carries
//! for custom shaders it uses, and the user's library folder (watched, so
//! editing a file updates every surface using it).

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rustmap_core::shaders::{MAX_CHAIN_PARAMS, ShaderInfo, parse_header, shader_id};
use rustmap_core::{Project, SurfaceId};
use rustmap_render::{ChainShader, FilterPipelines, QuadRenderer, wgpu};
use rustmap_ui::{ShaderEntry, ShaderOrigin};

/// How often the library folder is checked for changes.
const RESCAN: Duration = Duration::from_millis(500);

/// Shaders that come with RUSTMAP.
const BUILT_IN: [(&str, &str); 8] = [
    ("color", include_str!("../../assets/shaders/color.wgsl")),
    (
        "posterize",
        include_str!("../../assets/shaders/posterize.wgsl"),
    ),
    ("invert", include_str!("../../assets/shaders/invert.wgsl")),
    (
        "pixelate",
        include_str!("../../assets/shaders/pixelate.wgsl"),
    ),
    (
        "scanlines",
        include_str!("../../assets/shaders/scanlines.wgsl"),
    ),
    ("wave", include_str!("../../assets/shaders/wave.wgsl")),
    (
        "rgb_split",
        include_str!("../../assets/shaders/rgb_split.wgsl"),
    ),
    (
        "threshold",
        include_str!("../../assets/shaders/threshold.wgsl"),
    ),
];

/// Template for *New shader…*.
const TEMPLATE: &str = "// @name My shader
// @description What it does, shown in the Filters column.
// @param amount \"Amount\" 0 1 0.5

fn effect(color: vec4<f32>, uv: vec2<f32>, src_uv: vec2<f32>) -> vec4<f32> {
    // color: this pixel (straight alpha) · uv: on the surface · src_uv: in the input
    let gray = vec3<f32>(dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
    return vec4<f32>(mix(color.rgb, gray, p_amount()), color.a);
}
";

/// The user's shader library folder.
pub fn library_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("RUSTMAP")
        .join("Shaders")
}

struct Shader {
    source: String,
    info: Result<ShaderInfo, String>,
    origin: ShaderOrigin,
    path: Option<PathBuf>,
}

/// A surface's prepared chain: which compiled program, and its values.
struct SurfaceChain {
    key: u64,
    params: [f32; MAX_CHAIN_PARAMS],
}

pub struct ShaderLibrary {
    dir: PathBuf,
    shaders: BTreeMap<String, Shader>,
    /// Library files and their modification times at the last scan.
    files: BTreeMap<PathBuf, Option<SystemTime>>,
    scanned: Option<Instant>,
    /// Embedded sources seen at the last rebuild.
    embedded: BTreeMap<String, String>,
    /// Compiled chains by the hash of their shaders' sources.
    compiled: HashMap<u64, Result<FilterPipelines, String>>,
    /// Errors of each shader compiled on its own (for the library list).
    alone: HashMap<u64, Option<String>>,
    surfaces: HashMap<SurfaceId, SurfaceChain>,
    /// Why a surface's chain isn't running.
    pub surface_errors: HashMap<SurfaceId, String>,
    /// What the UI shows.
    pub entries: Vec<ShaderEntry>,
}

impl Default for ShaderLibrary {
    fn default() -> Self {
        Self {
            dir: library_dir(),
            shaders: BTreeMap::new(),
            files: BTreeMap::new(),
            scanned: None,
            embedded: BTreeMap::new(),
            compiled: HashMap::new(),
            alone: HashMap::new(),
            surfaces: HashMap::new(),
            surface_errors: HashMap::new(),
            entries: Vec::new(),
        }
    }
}

fn hash_of(parts: &[(&str, &str, bool)]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

impl ShaderLibrary {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Re-reads the library folder when something in it changed (or the
    /// show's embedded shaders did).
    fn rescan(&mut self, project: &Project) {
        let due = self.scanned.is_none_or(|t| t.elapsed() >= RESCAN);
        if !due && self.embedded == project.shader_sources {
            return;
        }
        self.scanned = Some(Instant::now());
        let mut files = BTreeMap::new();
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "wgsl") {
                    files.insert(path, entry.metadata().and_then(|m| m.modified()).ok());
                }
            }
        }
        if files == self.files
            && self.embedded == project.shader_sources
            && !self.shaders.is_empty()
        {
            return;
        }
        self.files = files;
        self.embedded = project.shader_sources.clone();

        let mut shaders = BTreeMap::new();
        for (id, src) in BUILT_IN {
            shaders.insert(
                id.to_owned(),
                Shader {
                    source: src.to_owned(),
                    info: parse_header(id, src),
                    origin: ShaderOrigin::BuiltIn,
                    path: None,
                },
            );
        }
        for (id, src) in &self.embedded {
            shaders.entry(id.clone()).or_insert_with(|| Shader {
                source: src.clone(),
                info: parse_header(id, src),
                origin: ShaderOrigin::Show,
                path: None,
            });
        }
        for path in self.files.keys() {
            let id = shader_id(path);
            let (source, info) = match std::fs::read_to_string(path) {
                Ok(src) => {
                    let info = parse_header(&id, &src);
                    (src, info)
                }
                Err(err) => (String::new(), Err(format!("can't read the file: {err}"))),
            };
            shaders.insert(
                id,
                Shader {
                    source,
                    info,
                    origin: ShaderOrigin::Library,
                    path: Some(path.clone()),
                },
            );
        }
        self.shaders = shaders;
        tracing::debug!("shader library: {} shaders", self.shaders.len());
    }

    /// Updates the library, compiles every chain the show uses, and keeps
    /// the show's copies of its custom shaders current. Call once a frame,
    /// before drawing.
    pub fn prepare(&mut self, quads: &QuadRenderer, device: &wgpu::Device, project: &mut Project) {
        self.rescan(project);

        // Each library shader compiled on its own, for the library list.
        let mut entries = Vec::new();
        for (id, s) in &self.shaders {
            let error = match &s.info {
                Err(e) => Some(e.clone()),
                Ok(info) => {
                    let key = hash_of(&[(id.as_str(), s.source.as_str(), false)]);
                    self.alone
                        .entry(key)
                        .or_insert_with(|| {
                            let chain = [ChainShader {
                                id,
                                source: &s.source,
                                params: info.params.iter().map(|p| p.id.clone()).collect(),
                                own_pass: false,
                            }];
                            quads.compile_chain(device, &chain).err()
                        })
                        .clone()
                }
            };
            let info = s.info.clone().unwrap_or_else(|_| ShaderInfo {
                id: id.clone(),
                name: id.replace('_', " "),
                ..Default::default()
            });
            entries.push(ShaderEntry {
                info,
                origin: s.origin,
                path: s.path.clone(),
                error,
            });
        }
        // Built-ins first, then the rest by name.
        entries.sort_by(|a, b| {
            (
                a.origin != ShaderOrigin::BuiltIn,
                a.info.name.to_lowercase(),
            )
                .cmp(&(
                    b.origin != ShaderOrigin::BuiltIn,
                    b.info.name.to_lowercase(),
                ))
        });
        self.entries = entries;

        // Every surface's chain.
        self.surfaces.clear();
        self.surface_errors.clear();
        let mut used = Vec::new();
        for surface in &project.surfaces {
            let active: Vec<_> = surface.filters.iter().filter(|f| f.enabled).collect();
            if active.is_empty() {
                continue;
            }
            let mut parts = Vec::new();
            let mut params = [0.0; MAX_CHAIN_PARAMS];
            let mut at = 0;
            let mut problem = None;
            for f in &active {
                match self.shaders.get(&f.shader) {
                    None => {
                        problem = Some(format!("“{}” isn't in the shader library", f.shader));
                        break;
                    }
                    Some(Shader { info: Err(e), .. }) => {
                        problem = Some(format!("{}: {e}", f.shader));
                        break;
                    }
                    Some(s @ Shader { info: Ok(info), .. }) => {
                        for i in 0..info.params.len() {
                            if at < MAX_CHAIN_PARAMS {
                                params[at] = f.value(info, i);
                            }
                            at += 1;
                        }
                        parts.push((f.shader.as_str(), s.source.as_str(), info, f.own_pass));
                    }
                }
            }
            if let Some(p) = problem {
                self.surface_errors.insert(surface.id, p);
                continue;
            }
            let key = hash_of(
                &parts
                    .iter()
                    .map(|(id, src, _, own)| (*id, *src, *own))
                    .collect::<Vec<_>>(),
            );
            let result = self.compiled.entry(key).or_insert_with(|| {
                let chain: Vec<_> = parts
                    .iter()
                    .map(|(id, src, info, own)| ChainShader {
                        id,
                        source: src,
                        params: info.params.iter().map(|p| p.id.clone()).collect(),
                        own_pass: *own,
                    })
                    .collect();
                let r = quads.compile_chain(device, &chain);
                if let Err(e) = &r {
                    tracing::warn!("shader chain: {e}");
                }
                r
            });
            if let Err(e) = result {
                self.surface_errors.insert(surface.id, e.clone());
                continue;
            }
            used.push(key);
            self.surfaces
                .insert(surface.id, SurfaceChain { key, params });
        }
        // Forget programs no surface uses any more (but keep a few around
        // for quick toggling).
        if self.compiled.len() > used.len() + 16 {
            self.compiled.retain(|k, _| used.contains(k));
        }

        // The show carries the sources of its custom shaders.
        let mut embedded = BTreeMap::new();
        let in_cues = project
            .cues
            .iter()
            .flat_map(|c| c.surfaces.values())
            .flat_map(|l| l.filters.iter());
        for f in project
            .surfaces
            .iter()
            .flat_map(|s| s.filters.iter())
            .chain(in_cues)
        {
            if let Some(s) = self.shaders.get(&f.shader)
                && s.origin != ShaderOrigin::BuiltIn
            {
                embedded.insert(f.shader.clone(), s.source.clone());
            }
        }
        if embedded != project.shader_sources {
            project.shader_sources = embedded.clone();
            self.embedded = embedded;
        }
    }

    /// The compiled chain and parameter block of a surface, if it has one.
    pub fn chain(&self, id: SurfaceId) -> Option<(&FilterPipelines, [f32; MAX_CHAIN_PARAMS])> {
        let c = self.surfaces.get(&id)?;
        let pipelines = self.compiled.get(&c.key)?.as_ref().ok()?;
        Some((pipelines, c.params))
    }

    /// Copies a file into the library. Returns the new shader's id.
    pub fn add_file(&mut self, path: &Path) -> Result<String, String> {
        let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if !src.contains("fn effect(") {
            return Err("this file doesn't define fn effect(color, uv, src_uv); see the README in the shader folder".into());
        }
        let id = shader_id(path);
        self.write(&id, &src)?;
        Ok(id)
    }

    /// Writes a shader into the library folder as `<id>.wgsl`.
    pub fn write(&mut self, id: &str, src: &str) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(format!("{id}.wgsl"));
        std::fs::write(&path, src).map_err(|e| e.to_string())?;
        self.scanned = None; // pick it up now
        self.files.clear();
        Ok(path)
    }

    /// A new shader from the template, with a free name. Returns its path.
    pub fn create(&mut self) -> Result<PathBuf, String> {
        let mut n = 1;
        let id = loop {
            let id = format!("my_shader_{n}");
            if !self.shaders.contains_key(&id) && !self.dir.join(format!("{id}.wgsl")).exists() {
                break id;
            }
            n += 1;
        };
        self.write(
            &id,
            &TEMPLATE.replace("My shader", &format!("My shader {n}")),
        )
    }

    /// Removes a shader file from the library (built-ins can't be removed).
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let Some(path) = self.shaders.get(id).and_then(|s| s.path.clone()) else {
            return Err("only shaders in your library can be removed".into());
        };
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        self.scanned = None;
        self.files.clear();
        Ok(())
    }

    /// Source of a shader the show carries, to add it to the library.
    pub fn source(&self, id: &str) -> Option<&str> {
        self.shaders.get(id).map(|s| s.source.as_str())
    }
}

/// Opens a folder or file in the system's file browser / editor.
pub fn reveal(path: &Path) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(err) = std::process::Command::new(cmd).arg(path).spawn() {
        tracing::warn!("couldn't open {}: {err}", path.display());
    }
}

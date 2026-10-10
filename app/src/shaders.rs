//! Shader filters: compiles the WGSL files surfaces use, keeps them cached,
//! and recompiles a file soon after it changes on disk (live editing).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rustmap_render::{FilterPipelines, QuadRenderer, wgpu};

/// How often a used file is checked for changes.
const RECHECK: Duration = Duration::from_millis(500);

struct Entry {
    modified: Option<SystemTime>,
    checked: Instant,
    result: Result<FilterPipelines, String>,
}

#[derive(Default)]
pub struct ShaderCache {
    entries: HashMap<PathBuf, Entry>,
    /// Why a file can't be used, for the UI.
    pub errors: HashMap<PathBuf, String>,
}

impl ShaderCache {
    /// Compiles new files, recompiles changed ones, forgets unused ones.
    pub fn refresh<'a>(
        &mut self,
        quads: &QuadRenderer,
        device: &wgpu::Device,
        used: impl Iterator<Item = &'a Path>,
    ) {
        let now = Instant::now();
        let mut keep = Vec::new();
        for path in used {
            keep.push(path.to_owned());
            if let Some(e) = self.entries.get(path)
                && now.duration_since(e.checked) < RECHECK
            {
                continue;
            }
            let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            if let Some(e) = self.entries.get_mut(path) {
                e.checked = now;
                if e.modified == modified {
                    continue;
                }
            }
            let result = match std::fs::read_to_string(path) {
                Ok(src) => quads.compile_filter(device, &src),
                Err(err) => Err(format!("can't read the file: {err}")),
            };
            match &result {
                Ok(_) => tracing::info!("shader filter ready: {}", path.display()),
                Err(err) => tracing::warn!("shader filter {}: {err}", path.display()),
            }
            self.entries.insert(
                path.to_owned(),
                Entry {
                    modified,
                    checked: now,
                    result,
                },
            );
        }
        self.entries.retain(|p, _| keep.contains(p));
        self.errors = self
            .entries
            .iter()
            .filter_map(|(p, e)| e.result.as_ref().err().map(|err| (p.clone(), err.clone())))
            .collect();
    }

    pub fn get(&self, path: &Path) -> Option<&FilterPipelines> {
        self.entries.get(path)?.result.as_ref().ok()
    }
}

/// Where the example shader filters live: next to the app in a bundle
/// (`Resources/shaders`), else the source tree's `assets/shaders`.
pub fn examples_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidates = [
        exe.parent()?.join("../Resources/shaders"),
        exe.parent()?.join("shaders"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/shaders"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

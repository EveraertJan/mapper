//! Show files (`.rustmap`): the project as RON text.
//!
//! - Readable and diffable, so shows can live in git.
//! - Media paths are stored relative to the show file when possible, so a
//!   show folder can be moved or copied to another machine.
//! - Written to a temporary file and renamed into place, so a crash or power
//!   cut mid-save never leaves a half-written show.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::Project;

pub const EXTENSION: &str = "rustmap";
const FORMAT: &str = "rustmap-show";
const VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a valid RUSTMAP show: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error(
        "{path} was saved by a newer RUSTMAP (format version {found}, this build reads up to {VERSION})"
    )]
    TooNew { path: PathBuf, found: u32 },
}

#[derive(Serialize, Deserialize)]
struct ShowFile {
    format: String,
    version: u32,
    project: Project,
}

/// Saves `project` to `path` atomically.
pub fn save(project: &Project, path: &Path) -> Result<(), FileError> {
    let base = path.parent().unwrap_or(Path::new("."));
    let mut stored = project.clone();
    for m in &mut stored.media {
        if let Some(p) = m.path_mut() {
            *p = relative_to(p, base);
        }
    }
    let file = ShowFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        project: stored,
    };
    let text =
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default().struct_names(true))
            .map_err(|e| FileError::Parse {
                path: path.to_owned(),
                reason: e.to_string(),
            })?;
    write_atomic(path, text.as_bytes())
}

/// Loads a show; media paths come back absolute.
pub fn load(path: &Path) -> Result<Project, FileError> {
    let text = std::fs::read_to_string(path).map_err(|source| FileError::Read {
        path: path.to_owned(),
        source,
    })?;
    let file: ShowFile = ron::from_str(&text).map_err(|e| FileError::Parse {
        path: path.to_owned(),
        reason: e.to_string(),
    })?;
    if file.format != FORMAT {
        return Err(FileError::Parse {
            path: path.to_owned(),
            reason: format!("unexpected format {:?}", file.format),
        });
    }
    if file.version > VERSION {
        return Err(FileError::TooNew {
            path: path.to_owned(),
            found: file.version,
        });
    }
    let base = path.parent().unwrap_or(Path::new("."));
    let mut project = file.project;
    for m in &mut project.media {
        if let Some(p) = m.path_mut()
            && p.is_relative()
        {
            *p = normalize(&base.join(&*p));
        }
    }
    Ok(project)
}

/// Writes `bytes` to a temp file next to `path`, flushes it, then renames.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    use std::io::Write;
    let err = |source| FileError::Write {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    let mut f = std::fs::File::create(&tmp).map_err(err)?;
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(err)
}

/// `path` relative to `base` if they share a root, else unchanged.
fn relative_to(path: &Path, base: &Path) -> PathBuf {
    if !path.is_absolute() || !base.is_absolute() {
        return path.to_owned();
    }
    let (path_n, base_n) = (normalize(path), normalize(base));
    let p: Vec<_> = path_n.components().collect();
    let b: Vec<_> = base_n.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    // Only the root in common (e.g. another drive): keep absolute.
    if common <= 1 {
        return path.to_owned();
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c.as_os_str());
    }
    out
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Extension of fixture type files.
pub const FIXTURE_EXTENSION: &str = "rmfixture";
const FIXTURE_FORMAT: &str = "rustmap-fixture-type";

#[derive(Serialize, Deserialize)]
struct FixtureFile {
    format: String,
    version: u32,
    fixture_type: crate::lights::DmxType,
}

/// Saves one DMX fixture type, to reuse it in other shows.
pub fn save_dmx_type(ty: &crate::lights::DmxType, path: &Path) -> Result<(), FileError> {
    let file = FixtureFile {
        format: FIXTURE_FORMAT.to_owned(),
        version: VERSION,
        fixture_type: ty.clone(),
    };
    let text =
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default().struct_names(true))
            .map_err(|e| FileError::Parse {
                path: path.to_owned(),
                reason: e.to_string(),
            })?;
    write_atomic(path, text.as_bytes())
}

/// Loads a fixture type file. Its id is meaningless until
/// [`Project::add_dmx_type`] assigns one.
pub fn load_dmx_type(path: &Path) -> Result<crate::lights::DmxType, FileError> {
    let text = std::fs::read_to_string(path).map_err(|source| FileError::Read {
        path: path.to_owned(),
        source,
    })?;
    let file: FixtureFile = ron::from_str(&text).map_err(|e| FileError::Parse {
        path: path.to_owned(),
        reason: e.to_string(),
    })?;
    if file.format != FIXTURE_FORMAT {
        return Err(FileError::Parse {
            path: path.to_owned(),
            reason: "not a RUSTMAP fixture type".into(),
        });
    }
    if file.version > VERSION {
        return Err(FileError::TooNew {
            path: path.to_owned(),
            found: file.version,
        });
    }
    Ok(file.fixture_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestPattern;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustmap-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample(dir: &Path) -> Project {
        let mut p = Project::default();
        p.name = "Facade".into();
        let o = p.add_output();
        p.output_mut(o).unwrap().test_pattern = TestPattern::Solid([0.1, 0.2, 0.3]);
        let s = p.add_surface(o).unwrap();
        let m = p.add_image(dir.join("media/wall.png"));
        p.add_ndi("STUDIO (Resolume)");
        p.surface_mut(s).unwrap().media = Some(m);
        p.surface_mut(s).unwrap().corners[2] = [0.91, 0.87];
        p.add_fixture(o, crate::Fixture::default_strip());
        let f = p.add_fixture(o, crate::Fixture::default_matrix()).unwrap();
        p.fixture_mut(f).unwrap().patch.protocol = crate::Protocol::Sacn;
        p.led.send = true;
        let par = p.add_dmx_type(crate::DmxType::templates()[2].clone());
        let f = p.add_dmx_fixture(par).unwrap();
        p.dmx_fixture_mut(f).unwrap().values[1] = 200;
        p
    }

    #[test]
    fn fixture_types_round_trip() {
        let dir = temp_dir("fixture-type");
        let ty = crate::DmxType::templates().pop().unwrap();
        let file = dir.join("head.rmfixture");
        save_dmx_type(&ty, &file).unwrap();
        assert_eq!(load_dmx_type(&file).unwrap(), ty);
        // A show file is not a fixture type.
        save(&sample(&dir), &dir.join("show.rustmap")).unwrap();
        assert!(load_dmx_type(&dir.join("show.rustmap")).is_err());
    }

    #[test]
    fn round_trip_is_identical() {
        let dir = temp_dir("roundtrip");
        let p = sample(&dir);
        let file = dir.join("show.rustmap");
        save(&p, &file).unwrap();
        assert_eq!(load(&file).unwrap(), p);
        // Ids keep counting after a reload.
        let mut q = load(&file).unwrap();
        assert!(q.add_surface(q.outputs[0].id).unwrap() > p.surfaces[0].id);
    }

    #[test]
    fn media_paths_are_stored_relative() {
        let dir = temp_dir("relative");
        let file = dir.join("show.rustmap");
        save(&sample(&dir), &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"media/wall.png\""), "{text}");
        assert!(!text.contains(dir.to_str().unwrap()));
    }

    #[test]
    fn moved_show_folder_still_finds_media() {
        let a = temp_dir("move-a");
        let b = temp_dir("move-b");
        save(&sample(&a), &a.join("show.rustmap")).unwrap();
        std::fs::copy(a.join("show.rustmap"), b.join("show.rustmap")).unwrap();
        let p = load(&b.join("show.rustmap")).unwrap();
        assert_eq!(p.media[0].path().unwrap(), b.join("media/wall.png"));
    }

    #[test]
    fn sibling_folders_use_dot_dot() {
        assert_eq!(
            relative_to(Path::new("/shows/media/a.png"), Path::new("/shows/2026")),
            PathBuf::from("../media/a.png")
        );
        assert_eq!(
            normalize(Path::new("/shows/2026/../media/a.png")),
            PathBuf::from("/shows/media/a.png")
        );
    }

    #[test]
    fn rejects_garbage_and_future_versions() {
        let dir = temp_dir("bad");
        let garbage = dir.join("garbage.rustmap");
        std::fs::write(&garbage, "not a show").unwrap();
        assert!(matches!(load(&garbage), Err(FileError::Parse { .. })));

        let future = dir.join("future.rustmap");
        save(&Project::default(), &future).unwrap();
        let text = std::fs::read_to_string(&future)
            .unwrap()
            .replace("version: 1", "version: 99");
        std::fs::write(&future, text).unwrap();
        assert!(matches!(
            load(&future),
            Err(FileError::TooNew { found: 99, .. })
        ));
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let file = dir.join("show.rustmap");
        save(&Project::default(), &file).unwrap();
        save(&Project::default(), &file).unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("show.rustmap")]);
    }
}

//! The open show: its file, undo history, unsaved-changes tracking, autosave.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rustmap_core::{History, Project, file};
use rustmap_ui::FileView;

const AUTOSAVE_EVERY: Duration = Duration::from_secs(60);

pub struct Document {
    pub path: Option<PathBuf>,
    pub history: History,
    /// Content as last saved (or as opened), to tell whether there are unsaved changes.
    saved: Project,
    last_autosave: Instant,
}

/// What the person chose when asked about unsaved changes.
pub enum Unsaved {
    Proceed,
    Cancel,
}

impl Document {
    pub fn new(project: &Project) -> Self {
        Self {
            path: None,
            history: History::new(project),
            saved: project.clone(),
            last_autosave: Instant::now(),
        }
    }

    pub fn opened(project: &Project, path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::new(project)
        }
    }

    pub fn is_dirty(&self, project: &Project) -> bool {
        !History::same_content(project, &self.saved)
    }

    pub fn view(&self, project: &Project) -> FileView {
        FileView {
            name: self
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            dirty: self.is_dirty(project),
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            autosave_available: self.newer_autosave().is_some(),
            presenting: false,
        }
    }

    pub fn window_title(&self, project: &Project) -> String {
        let view = self.view(project);
        format!(
            "{}{} — RUSTMAP",
            view.name.as_deref().unwrap_or("Unsaved show"),
            if view.dirty { " •" } else { "" }
        )
    }

    /// Saves to the current path, or asks for one. Returns false if cancelled or failed.
    pub fn save(&mut self, project: &Project, ask_path: bool) -> bool {
        let path = match (&self.path, ask_path) {
            (Some(p), false) => p.clone(),
            _ => match pick_save_path(self.path.as_deref()) {
                Some(p) => p,
                None => return false,
            },
        };
        match file::save(project, &path) {
            Ok(()) => {
                tracing::info!("saved {}", path.display());
                let _ = std::fs::remove_file(autosave_path(Some(&path)));
                self.saved = project.clone();
                self.path = Some(path);
                true
            }
            Err(err) => {
                error_dialog("Could not save the show", &err.to_string());
                false
            }
        }
    }

    /// If there are unsaved changes, asks whether to save them first.
    pub fn confirm_discard(&mut self, project: &Project) -> Unsaved {
        if !self.is_dirty(project) {
            return Unsaved::Proceed;
        }
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Unsaved changes")
            .set_description("This show has changes that are not saved. Save them first?")
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                "Save".into(),
                "Don't save".into(),
                "Cancel".into(),
            ))
            .show();
        match answer {
            rfd::MessageDialogResult::Custom(s) if s == "Save" => {
                if self.save(project, false) {
                    Unsaved::Proceed
                } else {
                    Unsaved::Cancel
                }
            }
            rfd::MessageDialogResult::Yes => {
                if self.save(project, false) {
                    Unsaved::Proceed
                } else {
                    Unsaved::Cancel
                }
            }
            rfd::MessageDialogResult::Custom(s) if s == "Don't save" => Unsaved::Proceed,
            rfd::MessageDialogResult::No => Unsaved::Proceed,
            _ => Unsaved::Cancel,
        }
    }

    /// Writes an autosave every minute while there are unsaved changes.
    pub fn autosave(&mut self, project: &Project) {
        if self.last_autosave.elapsed() < AUTOSAVE_EVERY || !self.is_dirty(project) {
            return;
        }
        self.last_autosave = Instant::now();
        let path = autosave_path(self.path.as_deref());
        match file::save(project, &path) {
            Ok(()) => tracing::debug!("autosaved to {}", path.display()),
            Err(err) => tracing::warn!("autosave failed: {err}"),
        }
    }

    /// The autosave for this show, if it is newer than the saved file.
    pub fn newer_autosave(&self) -> Option<PathBuf> {
        let auto = autosave_path(self.path.as_deref());
        let auto_time = modified(&auto)?;
        match self.path.as_deref().and_then(modified) {
            Some(saved) if saved >= auto_time => None,
            _ => Some(auto),
        }
    }
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// `show.rustmap` -> `show.rustmap.autosave` next to it; unsaved shows go to
/// the user's data folder.
pub fn autosave_path(show: Option<&Path>) -> PathBuf {
    match show {
        Some(p) => {
            let mut s = p.as_os_str().to_owned();
            s.push(".autosave");
            PathBuf::from(s)
        }
        None => dirs::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("RUSTMAP")
            .join("Unsaved show.rustmap.autosave"),
    }
}

pub fn pick_open_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open show")
        .add_filter("RUSTMAP show", &[file::EXTENSION])
        .pick_file()
}

fn pick_save_path(current: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save show")
        .add_filter("RUSTMAP show", &[file::EXTENSION])
        .set_file_name(current.and_then(|p| p.file_name()).map_or_else(
            || "Untitled.rustmap".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        ));
    if let Some(dir) = current.and_then(|p| p.parent()) {
        dialog = dialog.set_directory(dir);
    }
    let path = dialog.save_file()?;
    Some(if path.extension().is_some_and(|e| e == file::EXTENSION) {
        path
    } else {
        path.with_extension(file::EXTENSION)
    })
}

pub fn error_dialog(title: &str, message: &str) {
    tracing::error!("{title}: {message}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Points media `id` at `new_path`, then fixes every other missing image whose
/// file name exists in the same folder. Returns how many were relinked.
pub fn relink(project: &mut Project, id: rustmap_core::MediaId, new_path: &Path) -> usize {
    let dir = new_path.parent().map(Path::to_path_buf);
    let mut count = 0;
    for m in &mut project.media {
        let id_matches = m.id == id;
        let Some(path) = m.path_mut() else {
            continue;
        };
        if id_matches {
            *path = new_path.to_owned();
            count += 1;
        } else if !path.exists()
            && let (Some(dir), Some(name)) = (&dir, path.file_name())
            && dir.join(name).is_file()
        {
            *path = dir.join(name);
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relink_finds_siblings_by_name() {
        let dir = std::env::temp_dir().join(format!("rustmap-relink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["a.png", "b.png"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut p = Project::default();
        let a = p.add_image("/old/place/a.png".into());
        let b = p.add_image("/old/place/b.png".into());
        let c = p.add_image("/old/place/c.png".into()); // not in the new folder
        assert_eq!(relink(&mut p, a, &dir.join("a.png")), 2);
        assert_eq!(p.media_item(b).unwrap().path().unwrap(), dir.join("b.png"));
        assert_eq!(
            p.media_item(c).unwrap().path().unwrap(),
            Path::new("/old/place/c.png")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn autosave_sits_next_to_the_show() {
        assert_eq!(
            autosave_path(Some(Path::new("/shows/facade.rustmap"))),
            PathBuf::from("/shows/facade.rustmap.autosave")
        );
    }
}

//! Undo/redo by snapshots.
//!
//! Projects are small, so instead of a command per edit RUSTMAP keeps copies:
//! once an edit has *settled* (mouse released, no text field active) and the
//! project differs from the last snapshot, a new snapshot is taken. That
//! catches every kind of edit — drags, typed values, sliders, menus — and a
//! whole drag becomes one undo step.
//!
//! Live-operation state (which windows are open, fullscreen, monitor
//! assignment, NDI publishing) is not part of history: undoing a corner drag
//! must never close a projector window mid-show.

use crate::project::Project;

const LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct History {
    past: Vec<Project>,
    future: Vec<Project>,
    committed: Project,
}

/// The project with live-operation fields blanked, for comparing content.
fn content(p: &Project) -> Project {
    let mut c = p.clone();
    c.production = false;
    // Sending to the LEDs is operating the show too.
    c.led.send = false;
    c.led.blackout = false;
    // Pause/play is operating the show, not editing it.
    for m in &mut c.media {
        if let crate::media::MediaSource::Video { playback, .. } = &mut m.source {
            playback.paused = false;
        }
    }
    for o in &mut c.outputs {
        o.enabled = false;
        o.fullscreen = false;
        o.monitor = None;
        o.ndi_send = false;
    }
    c
}

/// Copies live-operation fields from `live` onto `restored` (matched by output id).
fn keep_live_state(restored: &mut Project, live: &Project) {
    restored.led.send = live.led.send;
    restored.led.blackout = live.led.blackout;
    for o in &mut restored.outputs {
        if let Some(l) = live.output(o.id) {
            o.enabled = l.enabled;
            o.fullscreen = l.fullscreen;
            o.monitor.clone_from(&l.monitor);
            o.ndi_send = l.ndi_send;
        }
    }
    for m in &mut restored.media {
        if let crate::media::MediaSource::Video { playback, .. } = &mut m.source
            && let Some(crate::media::MediaSource::Video {
                playback: live_pb, ..
            }) = live.media_item(m.id).map(|l| &l.source)
        {
            playback.paused = live_pb.paused;
        }
    }
}

impl History {
    pub fn new(project: &Project) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            committed: content(project),
        }
    }

    /// Call once per frame. Takes a snapshot when `settled` and the content changed.
    pub fn observe(&mut self, project: &Project, settled: bool) {
        if !settled {
            return;
        }
        let now = content(project);
        if now != self.committed {
            self.past.push(std::mem::replace(&mut self.committed, now));
            if self.past.len() > LIMIT {
                self.past.remove(0);
            }
            self.future.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo(&mut self, project: &mut Project) -> bool {
        // An edit that hasn't settled yet counts as done first.
        self.observe(project, true);
        let Some(prev) = self.past.pop() else {
            return false;
        };
        self.future
            .push(std::mem::replace(&mut self.committed, prev.clone()));
        let mut restored = prev;
        keep_live_state(&mut restored, project);
        *project = restored;
        true
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.past
            .push(std::mem::replace(&mut self.committed, next.clone()));
        let mut restored = next;
        keep_live_state(&mut restored, project);
        *project = restored;
        true
    }

    /// True if `project`'s content differs from `other`'s (ignores live state).
    pub fn same_content(a: &Project, b: &Project) -> bool {
        content(a) == content(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settled_drag_is_one_step() {
        let mut p = Project::default();
        let o = p.add_output();
        let s = p.add_surface(o).unwrap();
        let mut h = History::new(&p);

        for i in 1..=10 {
            p.surface_mut(s).unwrap().corners[0][0] = i as f32 * 0.01;
            h.observe(&p, false); // mouse still down
        }
        h.observe(&p, true);
        assert!(h.undo(&mut p));
        assert_eq!(p.surface(s).unwrap().corners[0][0], 0.2);
        assert!(!h.can_undo());
        assert!(h.redo(&mut p));
        assert_eq!(p.surface(s).unwrap().corners[0][0], 10.0 * 0.01);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut p = Project::default();
        let mut h = History::new(&p);
        p.add_output();
        h.observe(&p, true);
        h.undo(&mut p);
        assert!(h.can_redo());
        p.name = "x".into();
        h.observe(&p, true);
        assert!(!h.can_redo());
    }

    #[test]
    fn window_state_is_not_history() {
        let mut p = Project::default();
        let o = p.add_output();
        let s = p.add_surface(o).unwrap();
        let mut h = History::new(&p);

        p.output_mut(o).unwrap().enabled = true;
        h.observe(&p, true);
        assert!(!h.can_undo(), "opening a window is not an undo step");

        p.surface_mut(s).unwrap().opacity = 0.5;
        h.observe(&p, true);
        h.undo(&mut p);
        assert_eq!(p.surface(s).unwrap().opacity, 1.0);
        assert!(
            p.output(o).unwrap().enabled,
            "undo keeps the projector window open"
        );
    }

    #[test]
    fn undo_commits_an_unsettled_edit_first() {
        let mut p = Project::default();
        let mut h = History::new(&p);
        p.name = "typed".into();
        h.observe(&p, false); // still typing
        h.undo(&mut p);
        assert_eq!(p.name, "Untitled");
    }

    #[test]
    fn history_is_bounded() {
        let mut p = Project::default();
        let mut h = History::new(&p);
        for i in 0..(LIMIT + 50) {
            p.name = i.to_string();
            h.observe(&p, true);
        }
        let mut undos = 0;
        while h.undo(&mut p) {
            undos += 1;
        }
        assert_eq!(undos, LIMIT);
    }
}

//! Groups, cues and fades for DMX fixtures.
//!
//! A fixture's `values` are the "programmer": the console edits them and they
//! are what goes out. A *cue* is a stored copy of some fixtures' values.
//! Going to a cue moves the fixtures' values there, over the cue's fade time:
//! the [`CuePlayer`] writes the in-between values into the fixtures every
//! frame, so faders, group faders and the DMX monitor all show what is
//! actually sent. The app keeps undo from snapshotting while a fade runs, so
//! a whole fade is one undo step. Touching a fader mid-fade takes that
//! channel out of the fade.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::lights::{ChannelKind, DmxFixtureId};
use crate::project::Project;

/// Stable identifier of a fixture group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DmxGroupId(pub u32);

/// Stable identifier of a cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CueId(pub u32);

/// Several fixtures controlled together by one set of faders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmxGroup {
    pub id: DmxGroupId,
    pub name: String,
    pub fixtures: Vec<DmxFixtureId>,
}

/// A stored look: values for some fixtures, and how long to fade to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub id: CueId,
    pub name: String,
    /// Fade time in seconds (0 = jump).
    pub fade: f32,
    pub values: BTreeMap<DmxFixtureId, Vec<u16>>,
}

/// One fader of a group: every member channel of the same kind (or, for
/// "Other", the same label) moves together.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupControl {
    pub label: String,
    pub kind: ChannelKind,
    /// (fixture, channel index) pairs it drives.
    pub members: Vec<(DmxFixtureId, usize)>,
}

impl GroupControl {
    /// Mean position of the members, 0..=1.
    pub fn value(&self, project: &Project) -> f32 {
        let mut sum = 0.0;
        let mut n = 0;
        for &(id, i) in &self.members {
            if let Some((f, ty)) = project
                .dmx_fixture(id)
                .and_then(|f| project.dmx_type(f.type_id).map(|t| (f, t)))
            {
                sum += f32::from(f.value(ty, i)) / f32::from(ty.channels[i].max());
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { sum / n as f32 }
    }

    /// Moves every member to `v` (0..=1 of its own range).
    pub fn set(&self, project: &mut Project, v: f32) {
        let v = v.clamp(0.0, 1.0);
        for &(id, i) in &self.members {
            let Some(ty) = project
                .dmx_fixture(id)
                .and_then(|f| project.dmx_type(f.type_id))
                .cloned()
            else {
                continue;
            };
            if let Some(f) = project.dmx_fixture_mut(id) {
                f.fit_to(&ty);
                f.values[i] = (v * f32::from(ty.channels[i].max())).round() as u16;
            }
        }
    }
}

/// The faders a group offers, in order of first appearance.
pub fn group_controls(project: &Project, group: &DmxGroup) -> Vec<GroupControl> {
    let mut out: Vec<GroupControl> = Vec::new();
    for &id in &group.fixtures {
        let Some((f, ty)) = project
            .dmx_fixture(id)
            .and_then(|f| project.dmx_type(f.type_id).map(|t| (f, t)))
        else {
            continue;
        };
        for (i, c) in ty.channels.iter().enumerate() {
            let existing = out
                .iter_mut()
                .find(|g| g.kind == c.kind && (c.kind != ChannelKind::Other || g.label == c.name));
            match existing {
                Some(g) => g.members.push((f.id, i)),
                None => out.push(GroupControl {
                    label: if c.kind == ChannelKind::Other {
                        c.name.clone()
                    } else {
                        c.kind.label().to_owned()
                    },
                    kind: c.kind,
                    members: vec![(f.id, i)],
                }),
            }
        }
    }
    out
}

/// Channels that blend during a fade; everything else jumps.
fn fades(kind: ChannelKind) -> bool {
    kind == ChannelKind::Dimmer
        || kind.is_color()
        || matches!(
            kind,
            ChannelKind::Pan | ChannelKind::Tilt | ChannelKind::Zoom | ChannelKind::Focus
        )
}

struct Fade {
    start: f64,
    duration: f64,
    /// Per fixture: (channel, from, to, value last written) for every
    /// channel still fading.
    channels: HashMap<DmxFixtureId, Vec<(usize, u16, u16, u16)>>,
}

/// Runs cue changes. Lives in the app, not in the show file or undo history.
#[derive(Default)]
pub struct CuePlayer {
    /// The cue last gone to.
    pub current: Option<CueId>,
    fade: Option<Fade>,
}

impl CuePlayer {
    /// Goes to a cue. Channels that blend start fading from where they are
    /// now (also mid-fade); the rest, and everything when the fade time is
    /// 0, take the cue's values at once.
    pub fn go(&mut self, project: &mut Project, cue: CueId, now: f64) {
        let Some(c) = project.cue(cue).cloned() else {
            return;
        };
        self.fade = None;
        let mut channels = HashMap::new();
        for (id, values) in &c.values {
            let Some(ty) = project
                .dmx_fixture(*id)
                .and_then(|f| project.dmx_type(f.type_id))
                .cloned()
            else {
                continue;
            };
            let Some(f) = project.dmx_fixture_mut(*id) else {
                continue;
            };
            f.fit_to(&ty);
            let mut fading = Vec::new();
            for (i, def) in ty.channels.iter().enumerate() {
                let to = values.get(i).copied().unwrap_or(def.default).min(def.max());
                let from = f.values[i];
                if c.fade > 0.0 && fades(def.kind) && from != to {
                    fading.push((i, from, to, from));
                } else {
                    f.values[i] = to;
                }
            }
            if !fading.is_empty() {
                channels.insert(*id, fading);
            }
        }
        self.current = Some(cue);
        if !channels.is_empty() {
            self.fade = Some(Fade {
                start: now,
                duration: f64::from(c.fade),
                channels,
            });
        }
    }

    /// Re-applies the current cue (after faders were moved away from it).
    pub fn again(&mut self, project: &mut Project, now: f64) {
        if let Some(c) = self.current {
            self.go(project, c, now);
        }
    }

    /// Next or previous cue in list order (wraps).
    pub fn step(&mut self, project: &mut Project, forward: bool, now: f64) {
        let n = project.cues.len();
        if n == 0 {
            return;
        }
        let at = self
            .current
            .and_then(|c| project.cues.iter().position(|q| q.id == c));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        let id = project.cues[next].id;
        self.go(project, id, now);
    }

    /// Stops a running fade where it is (undo, opening another show).
    pub fn stop(&mut self) {
        self.fade = None;
    }

    /// Forgets the current cue too.
    pub fn reset(&mut self) {
        self.fade = None;
        self.current = None;
    }

    /// Whether a fade is running.
    pub fn fading(&self) -> bool {
        self.fade.is_some()
    }

    /// 0..1 while a fade runs.
    pub fn fade_progress(&self, now: f64) -> Option<f32> {
        let f = self.fade.as_ref()?;
        Some((((now - f.start) / f.duration) as f32).clamp(0.0, 1.0))
    }

    /// Moves fading channels to where they should be at `now`. Call once per
    /// frame, before the values are sent.
    pub fn tick(&mut self, project: &mut Project, now: f64) {
        let Some(fade) = &mut self.fade else {
            return;
        };
        let t = (((now - fade.start) / fade.duration) as f32).clamp(0.0, 1.0);
        fade.channels.retain(|id, chans| {
            let Some(f) = project.dmx_fixture_mut(*id) else {
                return false;
            };
            chans.retain_mut(|(i, from, to, last)| {
                let Some(v) = f.values.get_mut(*i) else {
                    return false;
                };
                if *v != *last {
                    return false; // someone moved the fader: theirs now
                }
                let a = f32::from(*from);
                *v = (a + (f32::from(*to) - a) * t).round() as u16;
                *last = *v;
                t < 1.0
            });
            !chans.is_empty()
        });
        if t >= 1.0 || fade.channels.is_empty() {
            self.fade = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DmxType;

    /// Two RGBW pars + a head.
    fn rig() -> (Project, Vec<DmxFixtureId>) {
        let mut p = Project::default();
        let par = p.add_dmx_type(DmxType::templates()[2].clone());
        let head = p.add_dmx_type(DmxType::templates()[3].clone());
        let a = p.add_dmx_fixture(par).unwrap();
        let b = p.add_dmx_fixture(par).unwrap();
        let h = p.add_dmx_fixture(head).unwrap();
        (p, vec![a, b, h])
    }

    #[test]
    fn group_fader_moves_every_member() {
        let (mut p, ids) = rig();
        let g = p.add_dmx_group();
        p.dmx_group_mut(g).unwrap().fixtures = ids.clone();
        let group = p.dmx_group(g).unwrap().clone();
        let controls = group_controls(&p, &group);
        let dim = controls
            .iter()
            .find(|c| c.kind == ChannelKind::Dimmer)
            .unwrap();
        assert_eq!(dim.members.len(), 3); // two pars and the head
        dim.set(&mut p, 0.5);
        assert_eq!(p.dmx_fixture(ids[0]).unwrap().values[0], 128);
        assert_eq!(p.dmx_fixture(ids[1]).unwrap().values[0], 128);
        assert_eq!(p.dmx_fixture(ids[2]).unwrap().values[3], 128);
        assert!((dim.value(&p) - 0.502).abs() < 0.01);
        // Pan exists on the head only.
        assert_eq!(
            controls
                .iter()
                .find(|c| c.kind == ChannelKind::Pan)
                .unwrap()
                .members
                .len(),
            1
        );
    }

    #[test]
    fn cue_records_and_recalls() {
        let (mut p, ids) = rig();
        p.dmx_fixture_mut(ids[0]).unwrap().values[1] = 200;
        let cue = p.record_cue(None);
        p.dmx_fixture_mut(ids[0]).unwrap().values[1] = 5;
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        assert_eq!(p.dmx_fixture(ids[0]).unwrap().values[1], 200);
        assert_eq!(player.current, Some(cue));
        assert!(!player.fading()); // fade time 0 = jump
        // Current: re-apply after the faders moved.
        p.dmx_fixture_mut(ids[0]).unwrap().values[1] = 9;
        player.again(&mut p, 1.0);
        assert_eq!(p.dmx_fixture(ids[0]).unwrap().values[1], 200);
    }

    #[test]
    fn selected_fixtures_only() {
        let (mut p, ids) = rig();
        let cue = p.record_cue(Some(&ids[..1]));
        assert_eq!(p.cue(cue).unwrap().values.len(), 1);
        p.dmx_fixture_mut(ids[1]).unwrap().values[1] = 99;
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        assert_eq!(p.dmx_fixture(ids[1]).unwrap().values[1], 99); // untouched
    }

    #[test]
    fn fade_moves_the_faders_and_jumps_the_rest() {
        let (mut p, ids) = rig();
        let h = ids[2];
        p.dmx_fixture_mut(h).unwrap().values[3] = 200; // dimmer
        p.dmx_fixture_mut(h).unwrap().values[5] = 20; // color wheel
        let cue = p.record_cue(Some(&[h]));
        p.cue_mut(cue).unwrap().fade = 2.0;
        p.dmx_fixture_mut(h).unwrap().values[3] = 0;
        p.dmx_fixture_mut(h).unwrap().values[5] = 0;
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 10.0);
        assert_eq!(p.dmx_fixture(h).unwrap().values[3], 0); // starts where it was
        assert_eq!(p.dmx_fixture(h).unwrap().values[5], 20); // wheel jumps
        player.tick(&mut p, 11.0);
        assert_eq!(p.dmx_fixture(h).unwrap().values[3], 100);
        assert!((player.fade_progress(11.0).unwrap() - 0.5).abs() < 1e-4);
        player.tick(&mut p, 12.5);
        assert_eq!(p.dmx_fixture(h).unwrap().values[3], 200);
        assert!(!player.fading());
    }

    #[test]
    fn a_new_cue_mid_fade_starts_from_where_it_is() {
        let (mut p, ids) = rig();
        let a = ids[0];
        p.dmx_fixture_mut(a).unwrap().values[0] = 255;
        let full = p.record_cue(Some(&[a]));
        p.dmx_fixture_mut(a).unwrap().values[0] = 0;
        let dark = p.record_cue(Some(&[a]));
        p.cue_mut(full).unwrap().fade = 1.0;
        p.cue_mut(dark).unwrap().fade = 1.0;
        let mut player = CuePlayer::default();
        player.go(&mut p, full, 0.0);
        player.tick(&mut p, 0.4);
        assert_eq!(p.dmx_fixture(a).unwrap().values[0], 102);
        player.go(&mut p, dark, 0.4);
        player.tick(&mut p, 0.9);
        assert_eq!(p.dmx_fixture(a).unwrap().values[0], 51);
    }

    #[test]
    fn touching_a_fader_takes_it_out_of_the_fade() {
        let (mut p, ids) = rig();
        let a = ids[0];
        p.dmx_fixture_mut(a).unwrap().values[0] = 255;
        p.dmx_fixture_mut(a).unwrap().values[1] = 255;
        let cue = p.record_cue(Some(&[a]));
        p.cue_mut(cue).unwrap().fade = 1.0;
        p.dmx_fixture_mut(a).unwrap().values[0] = 0;
        p.dmx_fixture_mut(a).unwrap().values[1] = 0;
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        player.tick(&mut p, 0.5);
        p.dmx_fixture_mut(a).unwrap().values[0] = 7; // grabbed
        player.tick(&mut p, 0.75);
        assert_eq!(p.dmx_fixture(a).unwrap().values[0], 7);
        assert_eq!(p.dmx_fixture(a).unwrap().values[1], 191);
    }

    #[test]
    fn step_wraps_both_ways() {
        let (mut p, _) = rig();
        let c1 = p.record_cue(None);
        let c2 = p.record_cue(None);
        let mut player = CuePlayer::default();
        player.step(&mut p, true, 0.0);
        assert_eq!(player.current, Some(c1));
        player.step(&mut p, true, 0.0);
        assert_eq!(player.current, Some(c2));
        player.step(&mut p, true, 0.0);
        assert_eq!(player.current, Some(c1));
        player.step(&mut p, false, 0.0);
        assert_eq!(player.current, Some(c2));
    }

    #[test]
    fn removing_fixtures_cleans_groups_and_cues() {
        let (mut p, ids) = rig();
        let g = p.add_dmx_group();
        p.dmx_group_mut(g).unwrap().fixtures = ids.clone();
        let c = p.record_cue(None);
        p.remove_dmx_fixture(ids[0]);
        assert_eq!(p.dmx_group(g).unwrap().fixtures.len(), 2);
        assert!(!p.cue(c).unwrap().values.contains_key(&ids[0]));
    }
}

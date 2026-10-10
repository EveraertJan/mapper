//! Groups and cues.
//!
//! A *cue* is a stored look of the whole show: DMX fixture values, how each
//! surface looks (visible, opacity, color, blend, which media it shows, and
//! optionally its geometry) and whether each video plays. Its *scope* says
//! which of those it applies, so a lights-only cue leaves the mapping alone.
//!
//! Going to a cue moves the show there over the cue's fade time: the
//! [`CuePlayer`] writes the in-between values into the project every frame,
//! so faders, inspectors and outputs all show what is happening. The app
//! keeps undo from snapshotting while a fade runs, so a whole fade is one
//! undo step. Touching a fader or a surface mid-fade takes it out of the fade.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::lights::{ChannelKind, DmxFixtureId};
use crate::media::{MediaId, MediaSource};
use crate::project::Project;
use crate::surface::{BlendMode, ColorAdjust, Mask, Mesh, Shape, Surface, SurfaceId};

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

/// A stored look of the show, and how long to fade to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub id: CueId,
    pub name: String,
    /// Fade time in seconds (0 = jump).
    pub fade: f32,
    /// DMX fixture values.
    pub values: BTreeMap<DmxFixtureId, Vec<u16>>,
    /// How surfaces look.
    #[serde(default)]
    pub surfaces: BTreeMap<SurfaceId, SurfaceLook>,
    /// Video play state.
    #[serde(default)]
    pub media: BTreeMap<MediaId, MediaCue>,
    /// What going to this cue changes.
    #[serde(default)]
    pub scope: CueScope,
    /// Rewind this cue's videos to the start when it is recalled.
    #[serde(default)]
    pub restart: bool,
    /// Recorded from a selection: *Update* re-records the same items instead
    /// of the whole show.
    #[serde(default)]
    pub partial: bool,
}

/// Which parts of a cue are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CueScope {
    /// DMX fixture values.
    pub lights: bool,
    /// Surface visibility, opacity, color, blend and media.
    pub surfaces: bool,
    /// Surface corners, mesh, mask and input region. Off by default, so
    /// recalling a cue never undoes a calibration.
    pub geometry: bool,
    /// Video play / pause (and restart).
    pub media: bool,
}

impl Default for CueScope {
    fn default() -> Self {
        Self {
            lights: true,
            surfaces: true,
            geometry: false,
            media: true,
        }
    }
}

/// How a surface looks in a cue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceLook {
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// The shader chain.
    #[serde(default)]
    pub filters: Vec<crate::shaders::FilterUse>,
    /// Cues saved before color became a shader; migrated on load.
    #[serde(default, skip_serializing)]
    pub color: ColorAdjust,
    pub media: Option<MediaId>,
    pub shape: Shape,
    pub corners: [[f32; 2]; 4],
    pub input: [[f32; 2]; 4],
    pub mesh: Option<Mesh>,
    pub mask: Option<Mask>,
}

impl SurfaceLook {
    pub fn capture(s: &Surface) -> Self {
        Self {
            visible: s.visible,
            opacity: s.opacity,
            blend: s.blend,
            filters: s.filters.clone(),
            color: ColorAdjust::default(),
            media: s.media,
            shape: s.shape,
            corners: s.corners,
            input: s.input,
            mesh: s.mesh.clone(),
            mask: s.mask.clone(),
        }
    }
}

/// A video's state in a cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaCue {
    pub paused: bool,
}

impl Cue {
    /// "3 fixtures · 2 surfaces · 1 video", for the cue list.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let mut add = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        add(self.values.len(), "fixture", "fixtures");
        add(self.surfaces.len(), "surface", "surfaces");
        add(self.media.len(), "video", "videos");
        if parts.is_empty() {
            "empty".to_owned()
        } else {
            parts.join(" · ")
        }
    }
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

/// Surface numbers that fade, flattened: opacity, with `filters` every
/// shader parameter, and with `geometry` the corners, input region, mesh
/// points and mask points.
fn surface_floats(s: &Surface, filters: bool, geometry: bool) -> Vec<f32> {
    let mut v = vec![s.opacity];
    if filters {
        v.extend(s.filters.iter().flat_map(|f| f.params.iter().copied()));
    }
    if geometry {
        v.extend(s.corners.iter().flatten());
        v.extend(s.input.iter().flatten());
        if let Some(m) = &s.mesh {
            v.extend(m.points.iter().flatten());
        }
        if let Some(m) = &s.mask {
            v.extend(m.points.iter().flatten());
            v.push(m.feather);
        }
    }
    v
}

/// Writes [`surface_floats`] back (same layout).
fn set_surface_floats(s: &mut Surface, filters: bool, geometry: bool, v: &[f32]) {
    let mut it = v.iter().copied();
    let mut next = || it.next().unwrap_or_default();
    s.opacity = next();
    if filters {
        for p in s.filters.iter_mut().flat_map(|f| f.params.iter_mut()) {
            *p = next();
        }
    }
    if geometry {
        for p in s.corners.iter_mut().chain(s.input.iter_mut()) {
            *p = [next(), next()];
        }
        if let Some(m) = &mut s.mesh {
            for p in &mut m.points {
                *p = [next(), next()];
            }
        }
        if let Some(m) = &mut s.mask {
            for p in &mut m.points {
                *p = [next(), next()];
            }
            m.feather = next();
        }
    }
}

/// Whether two chains have the same shaders in the same order (with the
/// same number of values), so their parameters can be blended.
fn same_chain(a: &Surface, b: &Surface) -> bool {
    a.filters.len() == b.filters.len()
        && a.filters.iter().zip(&b.filters).all(|(x, y)| {
            x.shader == y.shader
                && x.enabled == y.enabled
                && x.own_pass == y.own_pass
                && x.params.len() == y.params.len()
        })
}

/// Whether two surfaces' geometry has the same structure, so it can be
/// blended point by point.
fn same_structure(a: &Surface, b: &Surface) -> bool {
    let mesh = match (&a.mesh, &b.mesh) {
        (None, None) => true,
        (Some(x), Some(y)) => x.cols == y.cols && x.rows == y.rows,
        _ => false,
    };
    let mask = match (&a.mask, &b.mask) {
        (None, None) => true,
        (Some(x), Some(y)) => x.points.len() == y.points.len(),
        _ => false,
    };
    mesh && mask
}

struct SurfaceFade {
    filters: bool,
    geometry: bool,
    from: Vec<f32>,
    to: Vec<f32>,
    last: Vec<f32>,
    /// Fading out: hide it at the end and restore this opacity.
    hide_with: Option<f32>,
}

struct Fade {
    start: f64,
    duration: f64,
    /// Per fixture: (channel, from, to, value last written) for every
    /// channel still fading.
    channels: HashMap<DmxFixtureId, Vec<(usize, u16, u16, u16)>>,
    surfaces: HashMap<SurfaceId, SurfaceFade>,
}

/// Runs cue changes. Lives in the app, not in the show file or undo history.
#[derive(Default)]
pub struct CuePlayer {
    /// The cue last gone to.
    pub current: Option<CueId>,
    fade: Option<Fade>,
    /// Videos to rewind, for the app to pick up ([`Self::take_restarts`]).
    restarts: Vec<MediaId>,
}

impl CuePlayer {
    /// Goes to a cue: applies what its scope covers. Numbers (dimmer and
    /// colors, opacity, surface color, geometry) fade from where they are
    /// now, also mid-fade; everything else switches at once.
    pub fn go(&mut self, project: &mut Project, cue: CueId, now: f64) {
        let Some(c) = project.cue(cue).cloned() else {
            return;
        };
        self.fade = None;
        let fading = c.fade > 0.0;
        let mut channels = HashMap::new();
        if c.scope.lights {
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
                let mut list = Vec::new();
                for (i, def) in ty.channels.iter().enumerate() {
                    let to = values.get(i).copied().unwrap_or(def.default).min(def.max());
                    let from = f.values[i];
                    if fading && fades(def.kind) && from != to {
                        list.push((i, from, to, from));
                    } else {
                        f.values[i] = to;
                    }
                }
                if !list.is_empty() {
                    channels.insert(*id, list);
                }
            }
        }

        let mut surfaces = HashMap::new();
        if c.scope.surfaces || c.scope.geometry {
            for (id, look) in &c.surfaces {
                let media_ok = look.media.is_none_or(|m| project.media_item(m).is_some());
                let Some(s) = project.surface_mut(*id) else {
                    continue;
                };
                // Where it ends up.
                let mut target = s.clone();
                if c.scope.surfaces {
                    target.visible = look.visible;
                    target.opacity = look.opacity;
                    target.blend = look.blend;
                    target.filters.clone_from(&look.filters);
                    if media_ok {
                        target.media = look.media;
                    }
                }
                let mut geometry = c.scope.geometry;
                if geometry {
                    target.shape = look.shape;
                    target.corners = look.corners;
                    target.input = look.input;
                    target.mesh.clone_from(&look.mesh);
                    target.mask.clone_from(&look.mask);
                    if !same_structure(s, &target) {
                        // Different mesh size or mask: no point-by-point
                        // blend, so the geometry switches at once.
                        s.mesh.clone_from(&target.mesh);
                        s.mask.clone_from(&target.mask);
                        s.corners = target.corners;
                        s.input = target.input;
                        geometry = false;
                    }
                    s.shape = target.shape;
                    if let (Some(a), Some(b)) = (&mut s.mesh, &target.mesh) {
                        a.smooth = b.smooth;
                    }
                    if let (Some(a), Some(b)) = (&mut s.mask, &target.mask) {
                        a.invert = b.invert;
                    }
                }
                if !fading {
                    *s = target;
                    continue;
                }
                // Switches happen at the start of the fade.
                s.blend = target.blend;
                s.media = target.media;
                // A different chain switches; the same chain fades its values.
                let filters = same_chain(s, &target);
                if !filters {
                    s.filters.clone_from(&target.filters);
                }
                // Visibility fades through opacity.
                let shown = |x: &Surface| if x.visible { x.opacity } else { 0.0 };
                let hide_with = (!target.visible).then_some(target.opacity);
                let from_opacity = shown(s);
                s.visible = s.visible || target.visible;
                s.opacity = from_opacity;
                target.opacity = shown(&target);
                let from = surface_floats(s, filters, geometry);
                let to = surface_floats(&target, filters, geometry);
                if from == to {
                    if let Some(o) = hide_with {
                        s.visible = false;
                        s.opacity = o;
                    }
                    continue;
                }
                surfaces.insert(
                    *id,
                    SurfaceFade {
                        filters,
                        geometry,
                        last: from.clone(),
                        from,
                        to,
                        hide_with,
                    },
                );
            }
        }

        if c.scope.media {
            for (id, m) in &c.media {
                let Some(item) = project.media.iter_mut().find(|x| x.id == *id) else {
                    continue;
                };
                if let MediaSource::Video { playback, .. } = &mut item.source {
                    playback.paused = m.paused;
                    if c.restart {
                        self.restarts.push(*id);
                    }
                }
            }
        }

        self.current = Some(cue);
        if !channels.is_empty() || !surfaces.is_empty() {
            self.fade = Some(Fade {
                start: now,
                duration: f64::from(c.fade),
                channels,
                surfaces,
            });
        }
    }

    /// Re-applies the current cue (after things were moved away from it).
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

    /// Videos the last cues asked to rewind.
    pub fn take_restarts(&mut self) -> Vec<MediaId> {
        std::mem::take(&mut self.restarts)
    }

    /// Stops a running fade where it is (undo, opening another show).
    pub fn stop(&mut self) {
        self.fade = None;
    }

    /// Forgets the current cue too.
    pub fn reset(&mut self) {
        self.fade = None;
        self.current = None;
        self.restarts.clear();
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

    /// Moves fading values to where they should be at `now`. Call once per
    /// frame.
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
        fade.surfaces.retain(|id, f| {
            let Some(s) = project.surface_mut(*id) else {
                return false;
            };
            if surface_floats(s, f.filters, f.geometry) != f.last {
                return false; // edited mid-fade: leave it there
            }
            let v: Vec<f32> = f
                .from
                .iter()
                .zip(&f.to)
                .map(|(a, b)| a + (b - a) * t)
                .collect();
            set_surface_floats(s, f.filters, f.geometry, &v);
            f.last = surface_floats(s, f.filters, f.geometry);
            if t < 1.0 {
                return true;
            }
            if let Some(o) = f.hide_with {
                s.visible = false;
                s.opacity = o;
            }
            false
        });
        if t >= 1.0 || (fade.channels.is_empty() && fade.surfaces.is_empty()) {
            self.fade = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DmxType;
    use crate::media::MediaSource;

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

    fn stage() -> (Project, SurfaceId, MediaId, MediaId) {
        let mut p = Project::default();
        let out = p.add_output();
        let s = p.add_surface(out).unwrap();
        let a = p.add_video("a.mp4".into());
        let b = p.add_video("b.mp4".into());
        p.surface_mut(s).unwrap().media = Some(a);
        (p, s, a, b)
    }

    #[test]
    fn surface_look_fades_and_media_switches() {
        let (mut p, s, a, b) = stage();
        {
            let x = p.surface_mut(s).unwrap();
            x.opacity = 0.2;
            x.filters = vec![crate::FilterUse {
                shader: "color".into(),
                enabled: true,
                own_pass: false,
                params: vec![0.4],
            }];
            x.media = Some(b);
            x.blend = BlendMode::Add;
        }
        let cue = p.record_cue(None);
        p.cue_mut(cue).unwrap().fade = 2.0;
        {
            let x = p.surface_mut(s).unwrap();
            x.opacity = 1.0;
            x.filters[0].params[0] = 0.0;
            x.media = Some(a);
            x.blend = BlendMode::Normal;
        }
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        let x = p.surface(s).unwrap();
        assert_eq!(x.media, Some(b)); // switches at the start
        assert_eq!(x.blend, BlendMode::Add);
        assert_eq!(x.opacity, 1.0);
        player.tick(&mut p, 1.0);
        let x = p.surface(s).unwrap();
        assert!((x.opacity - 0.6).abs() < 1e-5);
        assert!((x.filters[0].params[0] - 0.2).abs() < 1e-5);
        player.tick(&mut p, 2.0);
        assert!((p.surface(s).unwrap().opacity - 0.2).abs() < 1e-5);
        assert!(!player.fading());
    }

    #[test]
    fn visibility_fades_through_opacity() {
        let (mut p, s, _, _) = stage();
        p.surface_mut(s).unwrap().visible = false;
        p.surface_mut(s).unwrap().opacity = 0.8;
        let hidden = p.record_cue(None);
        p.surface_mut(s).unwrap().visible = true;
        let shown = p.record_cue(None);
        p.cue_mut(hidden).unwrap().fade = 1.0;
        p.cue_mut(shown).unwrap().fade = 1.0;
        let mut player = CuePlayer::default();
        // Fade out: stays visible while dimming, hidden at the end with
        // its opacity back.
        player.go(&mut p, hidden, 0.0);
        player.tick(&mut p, 0.5);
        let x = p.surface(s).unwrap();
        assert!(x.visible && (x.opacity - 0.4).abs() < 1e-5);
        player.tick(&mut p, 1.0);
        let x = p.surface(s).unwrap();
        assert!(!x.visible && (x.opacity - 0.8).abs() < 1e-5);
        // Fade in from nothing.
        player.go(&mut p, shown, 2.0);
        let x = p.surface(s).unwrap();
        assert!(x.visible && x.opacity == 0.0);
        player.tick(&mut p, 2.25);
        assert!((p.surface(s).unwrap().opacity - 0.2).abs() < 1e-5);
    }

    #[test]
    fn geometry_only_when_asked_and_it_morphs() {
        let (mut p, s, _, _) = stage();
        let start = p.surface(s).unwrap().corners;
        p.surface_mut(s).unwrap().corners[0] = [0.0, 0.0];
        let cue = p.record_cue(None);
        p.surface_mut(s).unwrap().corners = start;
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        assert_eq!(p.surface(s).unwrap().corners, start); // geometry off by default
        p.cue_mut(cue).unwrap().scope.geometry = true;
        p.cue_mut(cue).unwrap().fade = 1.0;
        player.go(&mut p, cue, 0.0);
        player.tick(&mut p, 0.5);
        let c = p.surface(s).unwrap().corners[0];
        assert!((c[0] - start[0][0] / 2.0).abs() < 1e-5);
        player.tick(&mut p, 1.0);
        assert_eq!(p.surface(s).unwrap().corners[0], [0.0, 0.0]);
    }

    #[test]
    fn different_mesh_sizes_switch_at_once() {
        let (mut p, s, _, _) = stage();
        let mesh = |cols: u32, rows: u32| Mesh {
            cols,
            rows,
            points: vec![[0.5, 0.5]; (cols * rows) as usize],
            smooth: true,
        };
        p.surface_mut(s).unwrap().mesh = Some(mesh(3, 3));
        let cue = p.record_cue(None);
        {
            let c = p.cue_mut(cue).unwrap();
            c.scope.geometry = true;
            c.fade = 1.0;
        }
        p.surface_mut(s).unwrap().mesh = Some(mesh(4, 4));
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        assert_eq!(p.surface(s).unwrap().mesh.as_ref().unwrap().cols, 3);
    }

    #[test]
    fn videos_pause_play_and_restart() {
        let (mut p, _, a, b) = stage();
        let paused = |p: &Project, id| match &p.media_item(id).unwrap().source {
            MediaSource::Video { playback, .. } => playback.paused,
            _ => unreachable!(),
        };
        let set = |p: &mut Project, id, v| {
            if let Some(MediaSource::Video { playback, .. }) = p
                .media
                .iter_mut()
                .find(|m| m.id == id)
                .map(|m| &mut m.source)
            {
                playback.paused = v;
            }
        };
        set(&mut p, a, true);
        let cue = p.record_cue(None);
        p.cue_mut(cue).unwrap().restart = true;
        set(&mut p, a, false);
        set(&mut p, b, true);
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        assert!(paused(&p, a));
        assert!(!paused(&p, b));
        assert_eq!(player.take_restarts(), vec![a, b]);
        assert!(player.take_restarts().is_empty());
        // Media off in the scope: videos untouched.
        set(&mut p, a, false);
        p.cue_mut(cue).unwrap().scope.media = false;
        player.go(&mut p, cue, 0.0);
        assert!(!paused(&p, a));
    }

    #[test]
    fn scope_can_leave_lights_or_surfaces_alone() {
        let (mut p, ids) = rig();
        let out = p.add_output();
        let s = p.add_surface(out).unwrap();
        p.dmx_fixture_mut(ids[0]).unwrap().values[0] = 255;
        p.surface_mut(s).unwrap().opacity = 0.5;
        let cue = p.record_cue(None);
        p.dmx_fixture_mut(ids[0]).unwrap().values[0] = 0;
        p.surface_mut(s).unwrap().opacity = 1.0;
        p.cue_mut(cue).unwrap().scope.lights = false;
        let mut player = CuePlayer::default();
        player.go(&mut p, cue, 0.0);
        assert_eq!(p.dmx_fixture(ids[0]).unwrap().values[0], 0);
        assert_eq!(p.surface(s).unwrap().opacity, 0.5);
        p.cue_mut(cue).unwrap().scope = CueScope {
            surfaces: false,
            ..CueScope::default()
        };
        p.surface_mut(s).unwrap().opacity = 1.0;
        player.go(&mut p, cue, 0.0);
        assert_eq!(p.dmx_fixture(ids[0]).unwrap().values[0], 255);
        assert_eq!(p.surface(s).unwrap().opacity, 1.0);
    }

    #[test]
    fn partial_cues_update_the_same_items() {
        let (mut p, s, _, _) = stage();
        let out = p.outputs[0].id;
        let other = p.add_surface(out).unwrap();
        let cue = p.record_cue_of(Some((&[], &[s])));
        assert_eq!(p.cue(cue).unwrap().surfaces.len(), 1);
        assert!(p.cue(cue).unwrap().media.is_empty());
        p.surface_mut(s).unwrap().opacity = 0.3;
        p.update_cue(cue);
        let c = p.cue(cue).unwrap();
        assert_eq!(c.surfaces.len(), 1);
        assert_eq!(c.surfaces[&s].opacity, 0.3);
        assert!(!c.surfaces.contains_key(&other));
        let all = p.record_cue(None);
        assert_eq!(p.cue(all).unwrap().surfaces.len(), 2);
        assert_eq!(p.cue(all).unwrap().summary(), "2 surfaces · 2 videos");
    }

    #[test]
    fn removing_surfaces_and_media_cleans_cues() {
        let (mut p, s, a, _) = stage();
        let cue = p.record_cue(None);
        p.remove_media(a);
        let c = p.cue(cue).unwrap();
        assert!(!c.media.contains_key(&a));
        assert_eq!(c.surfaces[&s].media, None);
        p.remove_surface(s);
        assert!(p.cue(cue).unwrap().surfaces.is_empty());
    }
}

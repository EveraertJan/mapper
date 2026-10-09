use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fixture::{Fixture, FixtureId, FixtureShape, LedSettings};
use crate::lights::{DmxFixture, DmxFixtureId, DmxType, DmxTypeId};
use crate::media::{Media, MediaId, MediaSource};
use crate::output::{OutputConfig, OutputId};
use crate::surface::{Surface, SurfaceId};

/// The whole show.
///
/// Ids are handed out by counters and never reused, so a removed item can be
/// restored later (undo) without colliding with something new.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub name: String,
    pub outputs: Vec<OutputConfig>,
    pub media: Vec<Media>,
    /// Draw order: later surfaces are drawn on top.
    pub surfaces: Vec<Surface>,
    /// A production file opens straight into presentation: every output
    /// fullscreen on its projector, no editing guides.
    pub production: bool,
    /// LED fixtures, on outputs like surfaces.
    pub fixtures: Vec<Fixture>,
    pub led: LedSettings,
    /// DMX fixture types defined in this show.
    pub dmx_types: Vec<DmxType>,
    /// Patched DMX fixtures, in console order.
    pub dmx_fixtures: Vec<DmxFixture>,
    next_output_id: u32,
    next_media_id: u32,
    next_surface_id: u32,
    next_fixture_id: u32,
    next_dmx_type_id: u32,
    next_dmx_fixture_id: u32,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "Untitled".to_owned(),
            outputs: Vec::new(),
            media: Vec::new(),
            surfaces: Vec::new(),
            production: false,
            fixtures: Vec::new(),
            led: LedSettings::default(),
            dmx_types: Vec::new(),
            dmx_fixtures: Vec::new(),
            next_output_id: 1,
            next_media_id: 1,
            next_surface_id: 1,
            next_fixture_id: 1,
            next_dmx_type_id: 1,
            next_dmx_fixture_id: 1,
        }
    }
}

impl Project {
    // ---- outputs ----

    /// Adds a new output with default settings and returns its id.
    pub fn add_output(&mut self) -> OutputId {
        let id = OutputId(self.next_output_id);
        self.next_output_id += 1;
        self.outputs.push(OutputConfig::new(id));
        id
    }

    /// Removes an output and every surface on it.
    pub fn remove_output(&mut self, id: OutputId) -> Option<OutputConfig> {
        let index = self.outputs.iter().position(|o| o.id == id)?;
        self.surfaces.retain(|s| s.output != id);
        self.fixtures.retain(|f| f.output != id);
        Some(self.outputs.remove(index))
    }

    // ---- LED fixtures ----

    /// Adds a fixture to `output`, patched right after the last fixture
    /// (same protocol and destination). `None` if the output doesn't exist.
    pub fn add_fixture(&mut self, output: OutputId, shape: FixtureShape) -> Option<FixtureId> {
        self.output(output)?;
        let id = FixtureId(self.next_fixture_id);
        self.next_fixture_id += 1;
        let mut f = Fixture::new(id, output, shape);
        if let Some(last) = self.fixtures.last() {
            f.patch = last.patch.clone();
            let (universe, channel) = last.next_free();
            f.patch.universe = universe.min(f.patch.protocol.last_universe());
            f.patch.channel = channel;
        }
        self.fixtures.push(f);
        Some(id)
    }

    pub fn remove_fixture(&mut self, id: FixtureId) -> Option<Fixture> {
        let index = self.fixtures.iter().position(|f| f.id == id)?;
        Some(self.fixtures.remove(index))
    }

    pub fn fixture(&self, id: FixtureId) -> Option<&Fixture> {
        self.fixtures.iter().find(|f| f.id == id)
    }

    pub fn fixture_mut(&mut self, id: FixtureId) -> Option<&mut Fixture> {
        self.fixtures.iter_mut().find(|f| f.id == id)
    }

    // ---- DMX fixtures ----

    /// Adds a fixture type (e.g. a copy of a template or an imported file)
    /// under a fresh id; a name already in use gets a number.
    pub fn add_dmx_type(&mut self, mut ty: DmxType) -> DmxTypeId {
        let id = DmxTypeId(self.next_dmx_type_id);
        self.next_dmx_type_id += 1;
        ty.id = id;
        let base = ty.name.clone();
        let mut n = 2;
        while self.dmx_types.iter().any(|t| t.name == ty.name) {
            ty.name = format!("{base} {n}");
            n += 1;
        }
        self.dmx_types.push(ty);
        id
    }

    /// Removes a type and every fixture of it.
    pub fn remove_dmx_type(&mut self, id: DmxTypeId) -> Option<DmxType> {
        let index = self.dmx_types.iter().position(|t| t.id == id)?;
        self.dmx_fixtures.retain(|f| f.type_id != id);
        Some(self.dmx_types.remove(index))
    }

    pub fn dmx_type(&self, id: DmxTypeId) -> Option<&DmxType> {
        self.dmx_types.iter().find(|t| t.id == id)
    }

    pub fn dmx_type_mut(&mut self, id: DmxTypeId) -> Option<&mut DmxType> {
        self.dmx_types.iter_mut().find(|t| t.id == id)
    }

    /// Patches a new fixture of type `type_id` right after the last DMX
    /// fixture (next universe if it doesn't fit), at default values.
    pub fn add_dmx_fixture(&mut self, type_id: DmxTypeId) -> Option<DmxFixtureId> {
        let ty = self.dmx_type(type_id)?;
        let footprint = ty.footprint().max(1);
        let values = ty.defaults();
        let count = self
            .dmx_fixtures
            .iter()
            .filter(|f| f.type_id == type_id)
            .count();
        let name = format!("{} {}", ty.name, count + 1);
        let mut patch = crate::lights::first_patch();
        if let Some(last) = self.dmx_fixtures.last() {
            patch = last.patch.clone();
            let used = self
                .dmx_type(last.type_id)
                .map_or(1, |t| t.footprint().max(1));
            let next = last.first_channel() + used;
            if next + footprint > crate::fixture::UNIVERSE_SIZE {
                patch.universe = patch.universe.saturating_add(1);
                patch.channel = 1;
            } else {
                patch.channel = next as u16 + 1;
            }
        }
        let id = DmxFixtureId(self.next_dmx_fixture_id);
        self.next_dmx_fixture_id += 1;
        self.dmx_fixtures.push(DmxFixture {
            id,
            name,
            type_id,
            enabled: true,
            patch,
            values,
        });
        Some(id)
    }

    pub fn remove_dmx_fixture(&mut self, id: DmxFixtureId) -> Option<DmxFixture> {
        let index = self.dmx_fixtures.iter().position(|f| f.id == id)?;
        Some(self.dmx_fixtures.remove(index))
    }

    pub fn dmx_fixture(&self, id: DmxFixtureId) -> Option<&DmxFixture> {
        self.dmx_fixtures.iter().find(|f| f.id == id)
    }

    pub fn dmx_fixture_mut(&mut self, id: DmxFixtureId) -> Option<&mut DmxFixture> {
        self.dmx_fixtures.iter_mut().find(|f| f.id == id)
    }

    /// After editing type `id`: resize every fixture's values to match.
    pub fn refit_dmx_type(&mut self, id: DmxTypeId) {
        let Some(ty) = self.dmx_type(id).cloned() else {
            return;
        };
        for f in self.dmx_fixtures.iter_mut().filter(|f| f.type_id == id) {
            f.fit_to(&ty);
        }
    }

    /// Whether anything takes DMX channels (LED or DMX fixtures).
    pub fn has_lights(&self) -> bool {
        !self.fixtures.is_empty() || !self.dmx_fixtures.is_empty()
    }

    pub fn fixtures_on(&self, output: OutputId) -> impl Iterator<Item = &Fixture> {
        self.fixtures.iter().filter(move |f| f.output == output)
    }

    pub fn output(&self, id: OutputId) -> Option<&OutputConfig> {
        self.outputs.iter().find(|o| o.id == id)
    }

    pub fn output_mut(&mut self, id: OutputId) -> Option<&mut OutputConfig> {
        self.outputs.iter_mut().find(|o| o.id == id)
    }

    // ---- media ----

    /// Adds an image file as media, named after the file.
    pub fn add_image(&mut self, path: PathBuf) -> MediaId {
        let id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        let name = path.file_name().map_or_else(
            || format!("Media {}", id.0),
            |n| n.to_string_lossy().into_owned(),
        );
        self.media.push(Media {
            id,
            name,
            source: MediaSource::Image { path },
        });
        id
    }

    /// Adds a video file as media, playing and looping.
    pub fn add_video(&mut self, path: PathBuf) -> MediaId {
        let id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        let name = path.file_name().map_or_else(
            || format!("Video {}", id.0),
            |n| n.to_string_lossy().into_owned(),
        );
        self.media.push(Media {
            id,
            name,
            source: MediaSource::Video {
                path,
                playback: Default::default(),
            },
        });
        id
    }

    /// Adds a live NDI source as media. Reuses an existing item for the same source.
    pub fn add_ndi(&mut self, source: &str) -> MediaId {
        if let Some(m) = self
            .media
            .iter()
            .find(|m| matches!(&m.source, MediaSource::Ndi { source: s } if s == source))
        {
            return m.id;
        }
        let id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(Media {
            id,
            name: source.to_owned(),
            source: MediaSource::Ndi {
                source: source.to_owned(),
            },
        });
        id
    }

    /// Removes media; surfaces that showed it become empty.
    pub fn remove_media(&mut self, id: MediaId) -> Option<Media> {
        let index = self.media.iter().position(|m| m.id == id)?;
        for s in &mut self.surfaces {
            if s.media == Some(id) {
                s.media = None;
            }
        }
        Some(self.media.remove(index))
    }

    pub fn media_item(&self, id: MediaId) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    // ---- surfaces ----

    /// Adds a quad surface on `output`. `None` if the output does not exist.
    pub fn add_surface(&mut self, output: OutputId) -> Option<SurfaceId> {
        self.output(output)?;
        let id = SurfaceId(self.next_surface_id);
        self.next_surface_id += 1;
        self.surfaces.push(Surface::new(id, output));
        Some(id)
    }

    pub fn remove_surface(&mut self, id: SurfaceId) -> Option<Surface> {
        let index = self.surfaces.iter().position(|s| s.id == id)?;
        Some(self.surfaces.remove(index))
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.id == id)
    }

    pub fn surface_mut(&mut self, id: SurfaceId) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|s| s.id == id)
    }

    /// Surfaces on one output, in draw order (bottom first).
    pub fn surfaces_on(&self, output: OutputId) -> impl Iterator<Item = &Surface> {
        self.surfaces.iter().filter(move |s| s.output == output)
    }

    /// Moves a surface one step up (+1) or down (-1) in draw order among the
    /// surfaces of the same output.
    pub fn restack_surface(&mut self, id: SurfaceId, up: bool) {
        let Some(i) = self.surfaces.iter().position(|s| s.id == id) else {
            return;
        };
        let output = self.surfaces[i].output;
        let neighbour = if up {
            self.surfaces[i + 1..]
                .iter()
                .position(|s| s.output == output)
                .map(|j| i + 1 + j)
        } else {
            self.surfaces[..i].iter().rposition(|s| s.output == output)
        };
        if let Some(j) = neighbour {
            self.surfaces.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_never_reused() {
        let mut p = Project::default();
        let a = p.add_output();
        let b = p.add_output();
        assert_ne!(a, b);
        p.remove_output(b);
        let c = p.add_output();
        assert_ne!(
            b, c,
            "removed ids must not be reused (undo/redo relies on it)"
        );
    }

    #[test]
    fn lookup_and_remove() {
        let mut p = Project::default();
        let a = p.add_output();
        p.output_mut(a).unwrap().name = "Left".into();
        assert_eq!(p.output(a).unwrap().name, "Left");
        assert!(p.remove_output(a).is_some());
        assert!(p.output(a).is_none());
        assert!(p.remove_output(a).is_none());
    }

    #[test]
    fn removing_output_removes_its_surfaces() {
        let mut p = Project::default();
        let a = p.add_output();
        let b = p.add_output();
        let sa = p.add_surface(a).unwrap();
        let sb = p.add_surface(b).unwrap();
        p.remove_output(a);
        assert!(p.surface(sa).is_none());
        assert!(p.surface(sb).is_some());
    }

    #[test]
    fn surface_needs_existing_output() {
        let mut p = Project::default();
        assert!(p.add_surface(OutputId(42)).is_none());
    }

    #[test]
    fn removing_media_clears_surfaces() {
        let mut p = Project::default();
        let o = p.add_output();
        let s = p.add_surface(o).unwrap();
        let m = p.add_image("/shows/wall.png".into());
        assert_eq!(p.media_item(m).unwrap().name, "wall.png");
        p.surface_mut(s).unwrap().media = Some(m);
        p.remove_media(m);
        assert_eq!(p.surface(s).unwrap().media, None);
    }

    #[test]
    fn ndi_sources_are_not_duplicated() {
        let mut p = Project::default();
        let a = p.add_ndi("STUDIO (Resolume)");
        let b = p.add_ndi("STUDIO (Resolume)");
        let c = p.add_ndi("LAPTOP (OBS)");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(p.media_item(a).unwrap().is_live());
        assert!(p.media_item(a).unwrap().path().is_none());
    }

    #[test]
    fn restack_moves_within_same_output() {
        let mut p = Project::default();
        let a = p.add_output();
        let b = p.add_output();
        let s1 = p.add_surface(a).unwrap();
        let _other = p.add_surface(b).unwrap();
        let s2 = p.add_surface(a).unwrap();
        let order = |p: &Project| p.surfaces_on(a).map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(order(&p), vec![s1, s2]);
        p.restack_surface(s1, true);
        assert_eq!(order(&p), vec![s2, s1]);
        p.restack_surface(s1, true); // already on top
        assert_eq!(order(&p), vec![s2, s1]);
        p.restack_surface(s1, false);
        assert_eq!(order(&p), vec![s1, s2]);
    }
}

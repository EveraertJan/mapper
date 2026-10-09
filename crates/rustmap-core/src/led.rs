//! Turning sampled pixel colors into DMX universes.

use std::collections::{BTreeMap, HashMap};

use crate::fixture::{FixtureId, UNIVERSE_SIZE, UniverseKey};
use crate::lights::DmxFixtureId;
use crate::project::Project;

/// One universe's 512 channels.
pub type Universe = [u8; UNIVERSE_SIZE];

/// Any light that takes DMX channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LightId {
    Led(FixtureId),
    Dmx(DmxFixtureId),
}

/// Who drives one DMX channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub light: LightId,
    /// LED: pixel index. DMX fixture: channel index in its type.
    pub index: usize,
    /// Byte within that pixel or channel (color component, or 1 = fine byte).
    pub byte: usize,
}

/// Builds every universe the project's lights use. `colors` holds each LED
/// fixture's sampled pixels in wiring order (missing or short: black).
/// Disabled lights keep their universes but leave them at zero. The grand
/// master scales LED pixels and DMX intensity channels; blackout zeroes them.
pub fn build_universes(
    project: &Project,
    colors: &HashMap<FixtureId, Vec<[u8; 3]>>,
) -> BTreeMap<UniverseKey, Universe> {
    let master = if project.led.blackout {
        0.0
    } else {
        project.led.master.clamp(0.0, 1.0)
    };
    let mut out: BTreeMap<UniverseKey, Universe> = BTreeMap::new();
    for f in &project.fixtures {
        let cpp = f.color_order.channels();
        let pixels = colors.get(&f.id).filter(|_| f.enabled);
        let level = (f.brightness.clamp(0.0, 1.0) * master * 256.0) as u32;
        for (i, a) in f.addresses().into_iter().enumerate() {
            let universe = out
                .entry(f.universe_key(a.universe))
                .or_insert([0; UNIVERSE_SIZE]);
            let Some(&[r, g, b]) = pixels.and_then(|p| p.get(i)) else {
                continue;
            };
            let scale = |c: u8| ((u32::from(c) * level) >> 8) as u8;
            f.color_order.write(
                [scale(r), scale(g), scale(b)],
                &mut universe[a.channel..a.channel + cpp],
            );
        }
    }
    for f in &project.dmx_fixtures {
        let Some(ty) = project.dmx_type(f.type_id) else {
            continue;
        };
        let universe = out.entry(f.universe_key()).or_insert([0; UNIVERSE_SIZE]);
        f.write(ty, master, universe);
    }
    out
}

/// Every channel span of every light: (universe, first channel, width, owner).
fn spans(project: &Project) -> Vec<(UniverseKey, usize, usize, LightId, usize)> {
    let mut out = Vec::new();
    for f in &project.fixtures {
        let cpp = f.color_order.channels();
        for (i, a) in f.addresses().into_iter().enumerate() {
            out.push((
                f.universe_key(a.universe),
                a.channel,
                cpp,
                LightId::Led(f.id),
                i,
            ));
        }
    }
    for f in &project.dmx_fixtures {
        let Some(ty) = project.dmx_type(f.type_id) else {
            continue;
        };
        let key = f.universe_key();
        for (i, at) in f.layout(ty) {
            out.push((
                key.clone(),
                at,
                ty.channels[i].width(),
                LightId::Dmx(f.id),
                i,
            ));
        }
    }
    out
}

/// Who drives each channel (the first light patched there).
pub fn channel_owners(project: &Project) -> HashMap<(UniverseKey, usize), Owner> {
    let mut owners = HashMap::new();
    for (key, at, width, light, index) in spans(project) {
        for byte in 0..width {
            owners
                .entry((key.clone(), at + byte))
                .or_insert(Owner { light, index, byte });
        }
    }
    owners
}

/// Lights whose channels overlap an earlier light's: (later, earlier).
pub fn patch_conflicts(project: &Project) -> Vec<(LightId, LightId)> {
    let mut owners: HashMap<(UniverseKey, usize), LightId> = HashMap::new();
    let mut conflicts = Vec::new();
    for (key, at, width, light, _) in spans(project) {
        for ch in at..at + width {
            if let Some(other) = owners.insert((key.clone(), ch), light)
                && other != light
            {
                if !conflicts.contains(&(light, other)) {
                    conflicts.push((light, other));
                }
                break;
            }
        }
    }
    conflicts
}

/// "Channels overlap …" for every light sharing channels with `light`.
pub fn conflicts_for(project: &Project, light: LightId) -> Vec<String> {
    patch_conflicts(project)
        .into_iter()
        .filter_map(|(a, b)| match (a == light, b == light) {
            (true, _) => Some(b),
            (_, true) => Some(a),
            _ => None,
        })
        .map(|other| format!("Channels overlap {}", light_name(project, other)))
        .collect()
}

/// Display name of a light.
pub fn light_name(project: &Project, id: LightId) -> &str {
    match id {
        LightId::Led(id) => project.fixture(id).map_or("?", |f| f.name.as_str()),
        LightId::Dmx(id) => project.dmx_fixture(id).map_or("?", |f| f.name.as_str()),
    }
}

/// "Strip 1 pixel 13" or "Par 2 · Red" (with "fine" for the second byte of a 16-bit channel).
pub fn describe_owner(project: &Project, owner: Owner) -> String {
    let name = light_name(project, owner.light);
    match owner.light {
        LightId::Led(_) => format!("{name} pixel {}", owner.index + 1),
        LightId::Dmx(id) => {
            let channel = project
                .dmx_fixture(id)
                .and_then(|f| project.dmx_type(f.type_id))
                .and_then(|t| t.channels.get(owner.index))
                .map_or("?", |c| c.name.as_str());
            if owner.byte == 1 {
                format!("{name} · {channel} fine")
            } else {
                format!("{name} · {channel}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{ColorOrder, Fixture, FixtureShape};

    fn strip(project: &mut Project, pixels: u32) -> FixtureId {
        let o = project.outputs[0].id;
        project
            .add_fixture(
                o,
                FixtureShape::Strip {
                    start: [0.0, 0.0],
                    end: [1.0, 0.0],
                    pixels,
                },
            )
            .unwrap()
    }

    #[test]
    fn colors_land_on_their_channels() {
        let mut p = Project::default();
        p.add_output();
        let a = strip(&mut p, 2);
        let b = strip(&mut p, 1);
        // New fixtures are patched after the previous one.
        assert_eq!(p.fixture(b).unwrap().patch.channel, 7);
        p.fixture_mut(b).unwrap().color_order = ColorOrder::Grb;
        let colors = HashMap::from([(a, vec![[1, 2, 3], [4, 5, 6]]), (b, vec![[255, 128, 0]])]);
        let u = build_universes(&p, &colors);
        assert_eq!(u.len(), 1);
        let data = u.values().next().unwrap();
        assert_eq!(&data[..9], &[1, 2, 3, 4, 5, 6, 128, 255, 0]);
        assert!(data[9..].iter().all(|v| *v == 0));
    }

    #[test]
    fn brightness_and_disable() {
        let mut p = Project::default();
        p.add_output();
        let a = strip(&mut p, 1);
        let colors = HashMap::from([(a, vec![[200, 100, 50]])]);
        p.fixture_mut(a).unwrap().brightness = 0.5;
        let u = build_universes(&p, &colors);
        assert_eq!(&u.values().next().unwrap()[..3], &[100, 50, 25]);
        p.led.master = 0.5;
        let u = build_universes(&p, &colors);
        assert_eq!(&u.values().next().unwrap()[..3], &[50, 25, 12]);
        p.led.blackout = true;
        let u = build_universes(&p, &colors);
        assert_eq!(&u.values().next().unwrap()[..3], &[0, 0, 0]);
        p.led.blackout = false;
        p.fixture_mut(a).unwrap().enabled = false;
        let u = build_universes(&p, &colors);
        assert_eq!(&u.values().next().unwrap()[..3], &[0, 0, 0]);
    }

    #[test]
    fn dmx_fixtures_share_the_universe() {
        let mut p = Project::default();
        p.add_output();
        let strip_id = strip(&mut p, 2); // channels 1..6
        let par = p.add_dmx_type(crate::DmxType::templates()[1].clone());
        let a = p.add_dmx_fixture(par).unwrap();
        // Patched after the last DMX fixture; the first one starts at 1, so move it.
        p.dmx_fixture_mut(a).unwrap().patch.channel = 7;
        let b = p.add_dmx_fixture(par).unwrap();
        assert_eq!(p.dmx_fixture(b).unwrap().patch.channel, 10);
        p.dmx_fixture_mut(a).unwrap().values = vec![255, 128, 0];
        let colors = HashMap::from([(strip_id, vec![[1, 2, 3], [4, 5, 6]])]);
        let u = build_universes(&p, &colors);
        assert_eq!(u.len(), 1);
        assert_eq!(
            &u.values().next().unwrap()[..12],
            &[1, 2, 3, 4, 5, 6, 255, 128, 0, 0, 0, 0]
        );
        assert!(patch_conflicts(&p).is_empty());
        let key = p.dmx_fixture(a).unwrap().universe_key();
        let owners = channel_owners(&p);
        assert_eq!(
            describe_owner(&p, owners[&(key.clone(), 7)]),
            "RGB PAR 1 · Green"
        );
        assert_eq!(describe_owner(&p, owners[&(key, 0)]), "Strip 1 pixel 1");
        // Overlap with the strip.
        p.dmx_fixture_mut(b).unwrap().patch.channel = 3; // 3..5, inside the strip
        assert_eq!(
            patch_conflicts(&p),
            vec![(LightId::Dmx(b), LightId::Led(strip_id))]
        );
    }

    #[test]
    fn conflicts_and_owners() {
        let mut p = Project::default();
        p.add_output();
        let a = strip(&mut p, 10);
        let b = strip(&mut p, 10);
        assert!(patch_conflicts(&p).is_empty());
        p.fixture_mut(b).unwrap().patch.channel = 25; // inside a's 1..30
        assert_eq!(
            patch_conflicts(&p),
            vec![(LightId::Led(b), LightId::Led(a))]
        );
        let owners = channel_owners(&p);
        let key = p.fixture(a).unwrap().universe_key(0);
        let owner = |k: &UniverseKey, ch| owners.get(&(k.clone(), ch)).map(|o| (o.light, o.index));
        assert_eq!(owner(&key, 3), Some((LightId::Led(a), 1)));
        assert_eq!(owner(&key, 40), Some((LightId::Led(b), 5)));
        let _ = Fixture::default_strip();
    }
}

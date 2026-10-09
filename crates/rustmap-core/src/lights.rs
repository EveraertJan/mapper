//! DMX fixtures: lights you define yourself (a type with named channels)
//! and patch onto a universe, controlled from the console.

use serde::{Deserialize, Serialize};

use crate::fixture::{Patch, Protocol, UNIVERSE_SIZE, UniverseKey};

/// Stable identifier of a fixture type in a show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DmxTypeId(pub u32);

/// Stable identifier of a patched DMX fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DmxFixtureId(pub u32);

/// A kind of light: its name and channel layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmxType {
    pub id: DmxTypeId,
    pub name: String,
    pub channels: Vec<ChannelDef>,
}

/// One control of a fixture type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelDef {
    /// Label shown on the fader, e.g. "Pan" or "Gobo wheel".
    pub name: String,
    pub kind: ChannelKind,
    /// 16-bit: takes two DMX channels (coarse, then fine).
    #[serde(default)]
    pub fine: bool,
    /// Value the fixture starts at and returns to on Reset (0..=max).
    #[serde(default)]
    pub default: u16,
    /// Named value ranges, e.g. "Open" 0–7, "Strobe slow→fast" 10–255.
    #[serde(default)]
    pub ranges: Vec<ValueRange>,
}

/// A labelled range of 8-bit values on a channel (16-bit channels use the coarse byte).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueRange {
    pub from: u8,
    pub to: u8,
    pub label: String,
}

/// What a channel does. Decides its fader color and how the grand master,
/// blackout and Locate treat it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChannelKind {
    Dimmer,
    Red,
    Green,
    Blue,
    White,
    Amber,
    Uv,
    Pan,
    Tilt,
    Strobe,
    Zoom,
    Focus,
    ColorWheel,
    Gobo,
    Speed,
    #[default]
    Other,
}

impl ChannelKind {
    pub const ALL: [ChannelKind; 16] = [
        ChannelKind::Dimmer,
        ChannelKind::Red,
        ChannelKind::Green,
        ChannelKind::Blue,
        ChannelKind::White,
        ChannelKind::Amber,
        ChannelKind::Uv,
        ChannelKind::Pan,
        ChannelKind::Tilt,
        ChannelKind::Strobe,
        ChannelKind::Zoom,
        ChannelKind::Focus,
        ChannelKind::ColorWheel,
        ChannelKind::Gobo,
        ChannelKind::Speed,
        ChannelKind::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ChannelKind::Dimmer => "Dimmer",
            ChannelKind::Red => "Red",
            ChannelKind::Green => "Green",
            ChannelKind::Blue => "Blue",
            ChannelKind::White => "White",
            ChannelKind::Amber => "Amber",
            ChannelKind::Uv => "UV",
            ChannelKind::Pan => "Pan",
            ChannelKind::Tilt => "Tilt",
            ChannelKind::Strobe => "Strobe / shutter",
            ChannelKind::Zoom => "Zoom",
            ChannelKind::Focus => "Focus",
            ChannelKind::ColorWheel => "Color wheel",
            ChannelKind::Gobo => "Gobo",
            ChannelKind::Speed => "Speed",
            ChannelKind::Other => "Other",
        }
    }

    pub fn is_color(self) -> bool {
        matches!(
            self,
            ChannelKind::Red
                | ChannelKind::Green
                | ChannelKind::Blue
                | ChannelKind::White
                | ChannelKind::Amber
                | ChannelKind::Uv
        )
    }
}

impl ChannelDef {
    pub fn new(name: &str, kind: ChannelKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            fine: false,
            default: 0,
            ranges: Vec::new(),
        }
    }

    /// Highest value: 255, or 65535 for 16-bit.
    pub fn max(&self) -> u16 {
        if self.fine { u16::MAX } else { 255 }
    }

    /// DMX channels taken.
    pub fn width(&self) -> usize {
        if self.fine { 2 } else { 1 }
    }

    /// The labelled range a value falls in.
    pub fn range_at(&self, value: u16) -> Option<&ValueRange> {
        let coarse = if self.fine {
            (value >> 8) as u8
        } else {
            value as u8
        };
        self.ranges
            .iter()
            .find(|r| (r.from.min(r.to)..=r.from.max(r.to)).contains(&coarse))
    }
}

impl DmxType {
    /// DMX channels the type takes.
    pub fn footprint(&self) -> usize {
        self.channels.iter().map(ChannelDef::width).sum()
    }

    /// Default values, one per channel.
    pub fn defaults(&self) -> Vec<u16> {
        self.channels
            .iter()
            .map(|c| c.default.min(c.max()))
            .collect()
    }

    /// Whether the grand master works on the dimmer (if there is one) or
    /// directly on the colors.
    fn has_dimmer(&self) -> bool {
        self.channels.iter().any(|c| c.kind == ChannelKind::Dimmer)
    }

    /// Whether the grand master and blackout scale this channel.
    pub fn is_intensity(&self, index: usize) -> bool {
        let Some(c) = self.channels.get(index) else {
            return false;
        };
        if self.has_dimmer() {
            c.kind == ChannelKind::Dimmer
        } else {
            c.kind.is_color()
        }
    }

    /// Ready-made types to start from. Ids are placeholders; the project
    /// assigns real ones when a template is added.
    pub fn templates() -> Vec<DmxType> {
        use ChannelKind::*;
        let t = |name: &str, channels: Vec<ChannelDef>| DmxType {
            id: DmxTypeId(0),
            name: name.to_owned(),
            channels,
        };
        let ch = ChannelDef::new;
        let with_ranges = |mut c: ChannelDef, ranges: &[(u8, u8, &str)]| {
            c.ranges = ranges
                .iter()
                .map(|(from, to, label)| ValueRange {
                    from: *from,
                    to: *to,
                    label: (*label).to_owned(),
                })
                .collect();
            c
        };
        let fine = |mut c: ChannelDef, default: u16| {
            c.fine = true;
            c.default = default;
            c
        };
        vec![
            t("Dimmer", vec![ch("Dimmer", Dimmer)]),
            t(
                "RGB PAR",
                vec![ch("Red", Red), ch("Green", Green), ch("Blue", Blue)],
            ),
            t(
                "RGBW PAR + dimmer",
                vec![
                    ch("Dimmer", Dimmer),
                    ch("Red", Red),
                    ch("Green", Green),
                    ch("Blue", Blue),
                    ch("White", White),
                    with_ranges(
                        ch("Strobe", Strobe),
                        &[(0, 9, "Off"), (10, 255, "Slow to fast")],
                    ),
                ],
            ),
            t(
                "Moving head (example)",
                vec![
                    fine(ch("Pan", Pan), 32768),
                    fine(ch("Tilt", Tilt), 32768),
                    ch("Speed", Speed),
                    ch("Dimmer", Dimmer),
                    with_ranges(
                        ch("Shutter", Strobe),
                        &[
                            (0, 7, "Closed"),
                            (8, 15, "Open"),
                            (16, 255, "Strobe slow to fast"),
                        ],
                    ),
                    with_ranges(
                        ch("Color", ColorWheel),
                        &[
                            (0, 9, "White"),
                            (10, 19, "Red"),
                            (20, 29, "Green"),
                            (30, 39, "Blue"),
                            (128, 255, "Rotate"),
                        ],
                    ),
                    with_ranges(
                        ch("Gobo", Gobo),
                        &[(0, 7, "Open"), (8, 15, "Gobo 1"), (16, 23, "Gobo 2")],
                    ),
                ],
            ),
        ]
    }
}

/// A light patched on a universe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmxFixture {
    pub id: DmxFixtureId,
    pub name: String,
    pub type_id: DmxTypeId,
    /// Off: its channels go out as zero.
    pub enabled: bool,
    pub patch: Patch,
    /// One value per channel of the type (0..=255, or 0..=65535 for 16-bit).
    pub values: Vec<u16>,
}

impl DmxFixture {
    pub fn universe_key(&self) -> UniverseKey {
        UniverseKey {
            protocol: self.patch.protocol,
            destination: self.patch.destination.trim().to_owned(),
            universe: self.patch.universe,
        }
    }

    /// 0-based first channel.
    pub fn first_channel(&self) -> usize {
        usize::from(self.patch.channel.clamp(1, 512)) - 1
    }

    /// Value of channel `index`, falling back to the type's default.
    pub fn value(&self, ty: &DmxType, index: usize) -> u16 {
        let def = &ty.channels[index];
        self.values
            .get(index)
            .copied()
            .unwrap_or(def.default)
            .min(def.max())
    }

    /// Keeps `values` the same length as the type's channels (after a type edit).
    pub fn fit_to(&mut self, ty: &DmxType) {
        let defaults = ty.defaults();
        self.values.truncate(defaults.len());
        let n = self.values.len();
        self.values.extend_from_slice(&defaults[n..]);
        for (v, c) in self.values.iter_mut().zip(&ty.channels) {
            *v = (*v).min(c.max());
        }
    }

    /// Where each of the type's channels lands: (channel index, DMX channel
    /// offsets 0-based). Channels past 512 are left out.
    pub fn layout(&self, ty: &DmxType) -> Vec<(usize, usize)> {
        let mut at = self.first_channel();
        let mut out = Vec::with_capacity(ty.channels.len());
        for (i, c) in ty.channels.iter().enumerate() {
            if at + c.width() > UNIVERSE_SIZE {
                break;
            }
            out.push((i, at));
            at += c.width();
        }
        out
    }

    /// Whether the whole footprint fits in the universe.
    pub fn fits(&self, ty: &DmxType) -> bool {
        self.first_channel() + ty.footprint() <= UNIVERSE_SIZE
    }

    /// Writes the fixture into `universe`. `master` (0..1) scales intensity
    /// channels; disabled fixtures write zeros.
    pub fn write(&self, ty: &DmxType, master: f32, universe: &mut [u8; UNIVERSE_SIZE]) {
        self.write_values(ty, &self.values, master, universe);
    }

    /// Like [`Self::write`] with explicit `values` (a cue fade in progress).
    pub fn write_values(
        &self,
        ty: &DmxType,
        values: &[u16],
        master: f32,
        universe: &mut [u8; UNIVERSE_SIZE],
    ) {
        let master = master.clamp(0.0, 1.0);
        for (i, at) in self.layout(ty) {
            let def = &ty.channels[i];
            let mut v = if self.enabled {
                values.get(i).copied().unwrap_or(def.default).min(def.max())
            } else {
                0
            };
            if ty.is_intensity(i) {
                v = (f32::from(v) * master).round() as u16;
            }
            if def.fine {
                universe[at..at + 2].copy_from_slice(&v.to_be_bytes());
            } else {
                universe[at] = v.min(255) as u8;
            }
        }
    }

    /// Values for Locate: intensity and colors full, everything else at default.
    pub fn locate_values(ty: &DmxType) -> Vec<u16> {
        ty.channels
            .iter()
            .map(|c| {
                if c.kind == ChannelKind::Dimmer || c.kind.is_color() {
                    c.max()
                } else {
                    c.default.min(c.max())
                }
            })
            .collect()
    }
}

/// Default patch for the first DMX fixture of a show.
pub fn first_patch() -> Patch {
    Patch {
        protocol: Protocol::ArtNet,
        destination: String::new(),
        universe: 0,
        channel: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head() -> DmxType {
        DmxType::templates()
            .into_iter()
            .find(|t| t.name.starts_with("Moving head"))
            .unwrap()
    }

    #[test]
    fn footprint_counts_fine_channels_twice() {
        let t = head();
        assert_eq!(t.channels.len(), 7);
        assert_eq!(t.footprint(), 9);
    }

    #[test]
    fn writes_coarse_fine_and_master() {
        let t = head();
        let mut f = DmxFixture {
            id: DmxFixtureId(1),
            name: "Head".into(),
            type_id: t.id,
            enabled: true,
            patch: Patch {
                channel: 10,
                ..first_patch()
            },
            values: t.defaults(),
        };
        f.values[0] = 0x1234; // pan
        f.values[3] = 200; // dimmer
        f.values[4] = 12; // shutter
        let mut u = [0u8; UNIVERSE_SIZE];
        f.write(&t, 0.5, &mut u);
        assert_eq!(&u[9..11], &[0x12, 0x34]);
        assert_eq!(&u[11..13], &[0x80, 0x00]); // tilt default
        assert_eq!(u[14], 100); // dimmer halved by the master
        assert_eq!(u[15], 12); // shutter not scaled
        assert_eq!(t.channels[4].range_at(12).unwrap().label, "Open");
        f.enabled = false;
        f.write(&t, 1.0, &mut u);
        assert!(u.iter().all(|v| *v == 0));
    }

    #[test]
    fn master_scales_colors_without_a_dimmer() {
        let par = &DmxType::templates()[1];
        assert!(par.is_intensity(0) && par.is_intensity(2));
        let head = head();
        assert!(!head.is_intensity(5)); // color wheel
        assert!(head.is_intensity(3)); // dimmer
    }

    #[test]
    fn fit_to_follows_type_edits() {
        let mut t = DmxType::templates()[1].clone();
        let mut f = DmxFixture {
            id: DmxFixtureId(1),
            name: "Par".into(),
            type_id: t.id,
            enabled: true,
            patch: first_patch(),
            values: vec![10, 20, 30],
        };
        t.channels.push(ChannelDef {
            default: 7,
            ..ChannelDef::new("Strobe", ChannelKind::Strobe)
        });
        f.fit_to(&t);
        assert_eq!(f.values, vec![10, 20, 30, 7]);
        t.channels.truncate(2);
        f.fit_to(&t);
        assert_eq!(f.values, vec![10, 20]);
    }

    #[test]
    fn footprint_past_512_is_cut() {
        let t = head();
        let f = DmxFixture {
            id: DmxFixtureId(1),
            name: "Head".into(),
            type_id: t.id,
            enabled: true,
            patch: Patch {
                channel: 508,
                ..first_patch()
            },
            values: t.defaults(),
        };
        assert!(!f.fits(&t));
        // 507..: pan(2) tilt(2) speed(1) = 5 channels fit exactly.
        assert_eq!(f.layout(&t).len(), 3);
    }
}

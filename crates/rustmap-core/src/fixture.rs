//! LED fixtures: strips, matrices and free pixel lists placed on an output.
//! Each pixel samples the output image at its position, and the colors go
//! out over Art-Net or sACN.

use serde::{Deserialize, Serialize};

use crate::output::OutputId;

/// Stable identifier of a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FixtureId(pub u32);

impl std::fmt::Display for FixtureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Most pixels one fixture can have.
pub const FIXTURE_MAX_PIXELS: usize = 16_384;
/// DMX channels in a universe.
pub const UNIVERSE_SIZE: usize = 512;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    pub id: FixtureId,
    pub name: String,
    pub output: OutputId,
    /// Off: sends nothing (its channels stay at zero).
    pub enabled: bool,
    pub shape: FixtureShape,
    pub color_order: ColorOrder,
    pub patch: Patch,
    /// Output level, 0..1.
    pub brightness: f32,
    /// Average the image around each pixel (half the pixel spacing) instead
    /// of taking one point, so fine detail doesn't flicker.
    #[serde(default = "default_true")]
    pub smooth: bool,
}

fn default_true() -> bool {
    true
}

/// Where the pixels are, in normalized output space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FixtureShape {
    /// Pixels evenly spaced from `start` (first pixel) to `end` (last pixel).
    Strip {
        start: [f32; 2],
        end: [f32; 2],
        pixels: u32,
    },
    /// A grid of pixels filling a quad (TL, TR, BR, BL), wired as `wiring` says.
    Matrix {
        corners: [[f32; 2]; 4],
        cols: u32,
        rows: u32,
        wiring: Wiring,
    },
    /// Any pixel positions, in wiring order.
    Custom { points: Vec<[f32; 2]> },
}

/// How a matrix's pixels are chained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Wiring {
    pub start: Corner,
    /// Columns first (top to bottom) instead of rows first.
    pub vertical: bool,
    /// Every other row (or column) runs back the other way.
    pub serpentine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Corner {
    #[default]
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomRight,
        Corner::BottomLeft,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomRight => "Bottom right",
            Corner::BottomLeft => "Bottom left",
        }
    }
}

/// Channel layout of one pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorOrder {
    #[default]
    Rgb,
    Rbg,
    Grb,
    Gbr,
    Brg,
    Bgr,
    /// RGB plus a white channel (white = the gray part of the color).
    Rgbw,
    Grbw,
    /// One channel per pixel (brightness), for single-color strips and dimmers.
    White,
}

impl ColorOrder {
    pub const ALL: [ColorOrder; 9] = [
        ColorOrder::Rgb,
        ColorOrder::Rbg,
        ColorOrder::Grb,
        ColorOrder::Gbr,
        ColorOrder::Brg,
        ColorOrder::Bgr,
        ColorOrder::Rgbw,
        ColorOrder::Grbw,
        ColorOrder::White,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ColorOrder::Rgb => "RGB",
            ColorOrder::Rbg => "RBG",
            ColorOrder::Grb => "GRB",
            ColorOrder::Gbr => "GBR",
            ColorOrder::Brg => "BRG",
            ColorOrder::Bgr => "BGR",
            ColorOrder::Rgbw => "RGBW",
            ColorOrder::Grbw => "GRBW",
            ColorOrder::White => "W (single)",
        }
    }

    pub fn channels(self) -> usize {
        match self {
            ColorOrder::Rgbw | ColorOrder::Grbw => 4,
            ColorOrder::White => 1,
            _ => 3,
        }
    }

    /// Writes one pixel's channels (`out.len() == self.channels()`).
    pub fn write(self, [r, g, b]: [u8; 3], out: &mut [u8]) {
        let w = r.min(g).min(b);
        let luma = ((u32::from(r) * 54 + u32::from(g) * 183 + u32::from(b) * 19) >> 8) as u8;
        match self {
            ColorOrder::Rgb => out.copy_from_slice(&[r, g, b]),
            ColorOrder::Rbg => out.copy_from_slice(&[r, b, g]),
            ColorOrder::Grb => out.copy_from_slice(&[g, r, b]),
            ColorOrder::Gbr => out.copy_from_slice(&[g, b, r]),
            ColorOrder::Brg => out.copy_from_slice(&[b, r, g]),
            ColorOrder::Bgr => out.copy_from_slice(&[b, g, r]),
            ColorOrder::Rgbw => out.copy_from_slice(&[r - w, g - w, b - w, w]),
            ColorOrder::Grbw => out.copy_from_slice(&[g - w, r - w, b - w, w]),
            ColorOrder::White => out[0] = luma,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum Protocol {
    #[default]
    ArtNet,
    Sacn,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::ArtNet => "Art-Net",
            Protocol::Sacn => "sACN",
        }
    }

    /// Lowest valid universe (Art-Net counts from 0, sACN from 1).
    pub fn first_universe(self) -> u16 {
        match self {
            Protocol::ArtNet => 0,
            Protocol::Sacn => 1,
        }
    }

    pub fn last_universe(self) -> u16 {
        match self {
            Protocol::ArtNet => 0x7fff,
            Protocol::Sacn => 63_999,
        }
    }
}

/// Where a fixture's first pixel starts. Pixels never split across
/// universes: one that doesn't fit starts the next universe at channel 1
/// (170 RGB or 128 RGBW pixels per universe).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Patch {
    pub protocol: Protocol,
    /// Controller IP. Empty: broadcast (Art-Net) or multicast (sACN).
    pub destination: String,
    pub universe: u16,
    /// DMX start channel, 1..=512.
    pub channel: u16,
}

/// One universe on the wire: protocol, destination and number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UniverseKey {
    pub protocol: Protocol,
    /// Trimmed destination ("" = broadcast/multicast).
    pub destination: String,
    pub universe: u16,
}

impl std::fmt::Display for UniverseKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.protocol.label(), self.universe)?;
        if !self.destination.is_empty() {
            write!(f, " to {}", self.destination)?;
        }
        Ok(())
    }
}

/// Where one pixel's channels land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelAddress {
    /// Universe number.
    pub universe: u16,
    /// 0-based first channel.
    pub channel: usize,
}

impl Fixture {
    pub fn new(id: FixtureId, output: OutputId, shape: FixtureShape) -> Self {
        let kind = match shape {
            FixtureShape::Strip { .. } => "Strip",
            FixtureShape::Matrix { .. } => "Matrix",
            FixtureShape::Custom { .. } => "Pixels",
        };
        Self {
            id,
            name: format!("{kind} {}", id.0),
            output,
            enabled: true,
            shape,
            color_order: ColorOrder::Rgb,
            patch: Patch {
                protocol: Protocol::ArtNet,
                destination: String::new(),
                universe: 0,
                channel: 1,
            },
            brightness: 1.0,
            smooth: true,
        }
    }

    /// A horizontal strip across the middle of the output.
    pub fn default_strip() -> FixtureShape {
        FixtureShape::Strip {
            start: [0.2, 0.5],
            end: [0.8, 0.5],
            pixels: 60,
        }
    }

    /// A 16 × 16 matrix in the middle of the output.
    pub fn default_matrix() -> FixtureShape {
        FixtureShape::Matrix {
            corners: [[0.35, 0.3], [0.65, 0.3], [0.65, 0.7], [0.35, 0.7]],
            cols: 16,
            rows: 16,
            wiring: Wiring {
                serpentine: true,
                ..Wiring::default()
            },
        }
    }

    pub fn pixel_count(&self) -> usize {
        let n = match &self.shape {
            FixtureShape::Strip { pixels, .. } => *pixels as usize,
            FixtureShape::Matrix { cols, rows, .. } => *cols as usize * *rows as usize,
            FixtureShape::Custom { points } => points.len(),
        };
        n.min(FIXTURE_MAX_PIXELS)
    }

    /// Pixel positions in normalized output space, in wiring order.
    pub fn pixel_positions(&self) -> Vec<[f32; 2]> {
        let n = self.pixel_count();
        match &self.shape {
            FixtureShape::Strip { start, end, .. } => (0..n)
                .map(|i| {
                    let t = if n > 1 {
                        i as f32 / (n - 1) as f32
                    } else {
                        0.5
                    };
                    [
                        start[0] + (end[0] - start[0]) * t,
                        start[1] + (end[1] - start[1]) * t,
                    ]
                })
                .collect(),
            FixtureShape::Matrix {
                corners,
                cols,
                rows,
                wiring,
            } => {
                let uvs: Vec<[f32; 2]> = matrix_order(*cols, *rows, *wiring)
                    .into_iter()
                    .take(n)
                    .map(|(c, r)| {
                        [
                            (c as f32 + 0.5) / *cols as f32,
                            (r as f32 + 0.5) / *rows as f32,
                        ]
                    })
                    .collect();
                let quad = rustmap_geom::quad_from_arrays(corners);
                rustmap_geom::map_uv(&quad, &uvs)
                    .into_iter()
                    .map(|p| p.to_array())
                    .collect()
            }
            FixtureShape::Custom { points } => points.iter().take(n).copied().collect(),
        }
    }

    /// What the GPU samples per pixel: `[x, y, radius_x, radius_y]` in
    /// normalized output space (radius 0 = a single point).
    pub fn sample_points(&self) -> Vec<[f32; 4]> {
        let [rx, ry] = if self.smooth {
            self.sample_radius()
        } else {
            [0.0; 2]
        };
        self.pixel_positions()
            .into_iter()
            .map(|[x, y]| [x, y, rx, ry])
            .collect()
    }

    /// Half the spacing between neighboring pixels.
    fn sample_radius(&self) -> [f32; 2] {
        let dist =
            |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        match &self.shape {
            FixtureShape::Strip { start, end, pixels } => {
                let r = dist(*start, *end) / (pixels.saturating_sub(1).max(1)) as f32 / 2.0;
                [r, r]
            }
            FixtureShape::Matrix {
                corners: c,
                cols,
                rows,
                ..
            } => {
                let w = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
                let h = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
                [
                    w / *cols.max(&1) as f32 / 2.0,
                    h / *rows.max(&1) as f32 / 2.0,
                ]
            }
            FixtureShape::Custom { .. } => [0.0; 2],
        }
    }

    /// Handles that place the fixture: strip ends, matrix corners or the points.
    pub fn handles(&self) -> Vec<[f32; 2]> {
        match &self.shape {
            FixtureShape::Strip { start, end, .. } => vec![*start, *end],
            FixtureShape::Matrix { corners, .. } => corners.to_vec(),
            FixtureShape::Custom { points } => points.clone(),
        }
    }

    pub fn set_handle(&mut self, i: usize, p: [f32; 2]) {
        match &mut self.shape {
            FixtureShape::Strip { start, end, .. } => match i {
                0 => *start = p,
                1 => *end = p,
                _ => {}
            },
            FixtureShape::Matrix { corners, .. } => {
                if let Some(c) = corners.get_mut(i) {
                    *c = p;
                }
            }
            FixtureShape::Custom { points } => {
                if let Some(c) = points.get_mut(i) {
                    *c = p;
                }
            }
        }
    }

    pub fn translate(&mut self, d: [f32; 2]) {
        for i in 0..self.handles().len() {
            let h = self.handles()[i];
            self.set_handle(i, [h[0] + d[0], h[1] + d[1]]);
        }
    }

    pub fn universe_key(&self, universe: u16) -> UniverseKey {
        UniverseKey {
            protocol: self.patch.protocol,
            destination: self.patch.destination.trim().to_owned(),
            universe,
        }
    }

    /// DMX address of every pixel, in wiring order. Stops at the protocol's
    /// last universe.
    pub fn addresses(&self) -> Vec<PixelAddress> {
        let cpp = self.color_order.channels();
        let mut universe = self.patch.universe;
        let mut channel = usize::from(self.patch.channel.clamp(1, 512)) - 1;
        let mut out = Vec::with_capacity(self.pixel_count());
        for _ in 0..self.pixel_count() {
            if channel + cpp > UNIVERSE_SIZE {
                if universe >= self.patch.protocol.last_universe() {
                    break;
                }
                universe += 1;
                channel = 0;
            }
            out.push(PixelAddress { universe, channel });
            channel += cpp;
        }
        out
    }

    /// First and last universe this fixture uses.
    pub fn universe_range(&self) -> Option<(u16, u16)> {
        let a = self.addresses();
        Some((a.first()?.universe, a.last()?.universe))
    }

    /// Next free spot after this fixture (for patching the next one).
    pub fn next_free(&self) -> (u16, u16) {
        match self.addresses().last() {
            Some(a) => {
                let next = a.channel + self.color_order.channels();
                if next + 3 > UNIVERSE_SIZE {
                    (a.universe.saturating_add(1), 1)
                } else {
                    (a.universe, next as u16 + 1)
                }
            }
            None => (self.patch.universe, self.patch.channel),
        }
    }
}

/// (col, row) of each pixel in wiring order.
fn matrix_order(cols: u32, rows: u32, wiring: Wiring) -> Vec<(u32, u32)> {
    let (lines, len) = if wiring.vertical {
        (cols, rows)
    } else {
        (rows, cols)
    };
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for line in 0..lines {
        for k in 0..len {
            let k = if wiring.serpentine && line % 2 == 1 {
                len - 1 - k
            } else {
                k
            };
            let (mut c, mut r) = if wiring.vertical {
                (line, k)
            } else {
                (k, line)
            };
            if matches!(wiring.start, Corner::TopRight | Corner::BottomRight) {
                c = cols - 1 - c;
            }
            if matches!(wiring.start, Corner::BottomLeft | Corner::BottomRight) {
                r = rows - 1 - r;
            }
            out.push((c, r));
        }
    }
    out
}

/// Global LED output settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedSettings {
    /// Send to the network. Saved, so a production file starts sending.
    pub send: bool,
    /// Packets per second (most controllers want 30–44).
    pub fps: f32,
    /// Grand master (0..1): scales LED pixels and DMX intensity channels.
    pub master: f32,
    /// Everything dark. Operating the show, so kept out of undo.
    pub blackout: bool,
}

impl Default for LedSettings {
    fn default() -> Self {
        Self {
            send: false,
            fps: 40.0,
            master: 1.0,
            blackout: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(cols: u32, rows: u32, wiring: Wiring) -> Fixture {
        Fixture::new(
            FixtureId(1),
            OutputId(1),
            FixtureShape::Matrix {
                corners: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                cols,
                rows,
                wiring,
            },
        )
    }

    #[test]
    fn strip_positions_hit_both_ends() {
        let f = Fixture::new(
            FixtureId(1),
            OutputId(1),
            FixtureShape::Strip {
                start: [0.0, 0.5],
                end: [1.0, 0.5],
                pixels: 5,
            },
        );
        let p = f.pixel_positions();
        assert_eq!(p.len(), 5);
        assert_eq!(p[0], [0.0, 0.5]);
        assert_eq!(p[4], [1.0, 0.5]);
        assert!((p[2][0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn matrix_wiring_orders() {
        let order = |w| matrix_order(3, 2, w);
        assert_eq!(
            order(Wiring::default()),
            vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]
        );
        let snake = Wiring {
            serpentine: true,
            ..Wiring::default()
        };
        assert_eq!(
            order(snake),
            vec![(0, 0), (1, 0), (2, 0), (2, 1), (1, 1), (0, 1)]
        );
        let vertical_br = Wiring {
            start: Corner::BottomRight,
            vertical: true,
            serpentine: true,
        };
        assert_eq!(
            order(vertical_br),
            vec![(2, 1), (2, 0), (1, 0), (1, 1), (0, 1), (0, 0)]
        );
    }

    #[test]
    fn matrix_pixel_centers() {
        let f = matrix(2, 2, Wiring::default());
        let p = f.pixel_positions();
        assert_eq!(p.len(), 4);
        assert!((p[0][0] - 0.25).abs() < 1e-5 && (p[0][1] - 0.25).abs() < 1e-5);
        assert!((p[3][0] - 0.75).abs() < 1e-5 && (p[3][1] - 0.75).abs() < 1e-5);
    }

    #[test]
    fn pixels_do_not_split_across_universes() {
        let mut f = matrix(20, 20, Wiring::default()); // 400 RGB pixels
        f.patch.universe = 3;
        let a = f.addresses();
        assert_eq!(a.len(), 400);
        assert_eq!(
            a[169],
            PixelAddress {
                universe: 3,
                channel: 507
            }
        );
        assert_eq!(
            a[170],
            PixelAddress {
                universe: 4,
                channel: 0
            }
        );
        assert_eq!(f.universe_range(), Some((3, 5)));
        // Starting mid-universe.
        f.patch.channel = 500;
        let a = f.addresses();
        assert_eq!(
            a[0],
            PixelAddress {
                universe: 3,
                channel: 499
            }
        );
        assert_eq!(
            a[4],
            PixelAddress {
                universe: 4,
                channel: 0
            }
        );
        // RGBW: 128 per universe.
        f.patch.channel = 1;
        f.color_order = ColorOrder::Rgbw;
        assert_eq!(f.addresses()[128].universe, 4);
    }

    #[test]
    fn color_orders() {
        let mut out = [0u8; 4];
        ColorOrder::Grb.write([10, 20, 30], &mut out[..3]);
        assert_eq!(&out[..3], &[20, 10, 30]);
        ColorOrder::Rgbw.write([200, 100, 50], &mut out);
        assert_eq!(out, [150, 50, 0, 50]);
        let mut w = [0u8];
        ColorOrder::White.write([255, 255, 255], &mut w);
        assert_eq!(w[0], 255);
    }

    #[test]
    fn next_free_follows_the_fixture() {
        let mut f = matrix(10, 1, Wiring::default()); // 30 channels
        f.patch.universe = 2;
        assert_eq!(f.next_free(), (2, 31));
        f.patch.channel = 490; // 7 pixels fit, 3 wrap into universe 3
        assert_eq!(f.next_free(), (3, 10));
    }
}

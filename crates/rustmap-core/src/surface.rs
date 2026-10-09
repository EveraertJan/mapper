use serde::{Deserialize, Serialize};

use crate::media::MediaId;
use crate::output::OutputId;

/// Stable identifier of a surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SurfaceId(pub u32);

impl std::fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A quad on an output that shows one media item, perspective-warped.
///
/// `corners` are in normalized output space (0..1, y down) in the order
/// top-left, top-right, bottom-right, bottom-left, so the mapping survives a
/// change of output resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub id: SurfaceId,
    pub name: String,
    pub output: OutputId,
    pub media: Option<MediaId>,
    pub corners: [[f32; 2]; 4],
    pub opacity: f32,
    pub visible: bool,
    #[serde(default)]
    pub blend: BlendMode,
    #[serde(default)]
    pub shape: Shape,
    /// Which part of the media is shown, in media texture space (0..1, y
    /// down), same corner order. Any quad: crops, rotates or keystones the source.
    #[serde(default = "full_frame")]
    pub input: [[f32; 2]; 4],
    /// Mesh warp for curved or uneven surfaces. When set it replaces the
    /// four-corner perspective mapping (and `corners` follow its outer corners).
    #[serde(default)]
    pub mesh: Option<Mesh>,
    /// Polygon mask in surface space, so it follows any warp.
    #[serde(default)]
    pub mask: Option<Mask>,
    #[serde(default)]
    pub color: ColorAdjust,
}

/// A warp grid: `cols × rows` control points, row-major, in normalized output space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub cols: u32,
    pub rows: u32,
    pub points: Vec<[f32; 2]>,
    /// Smooth curves through the points (Catmull-Rom) instead of straight lines.
    pub smooth: bool,
}

impl Mesh {
    pub fn point_index(&self, col: u32, row: u32) -> usize {
        (row * self.cols + col) as usize
    }

    /// Outer corners, TL, TR, BR, BL.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let (c, r) = (self.cols - 1, self.rows - 1);
        [
            self.points[self.point_index(0, 0)],
            self.points[self.point_index(c, 0)],
            self.points[self.point_index(c, r)],
            self.points[self.point_index(0, r)],
        ]
    }
}

/// Most mask points the renderer takes.
pub const MASK_MAX_POINTS: usize = 32;

/// A polygon cut-out in surface texture space (0..1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub points: Vec<[f32; 2]>,
    /// Soft edge width, in surface-space units (0..0.25).
    pub feather: f32,
    /// Show everything except the inside of the polygon.
    pub invert: bool,
}

impl Mask {
    /// A rectangle inset from the surface edges, to start editing from.
    pub fn inset() -> Self {
        Self {
            points: vec![[0.15, 0.15], [0.85, 0.15], [0.85, 0.85], [0.15, 0.85]],
            feather: 0.02,
            invert: false,
        }
    }
}

/// Per-surface color correction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorAdjust {
    /// Added to every channel (-1..1).
    pub brightness: f32,
    /// Around mid-gray (0..2, 1 = unchanged).
    pub contrast: f32,
    /// 0 = gray, 1 = unchanged, 2 = double.
    pub saturation: f32,
    /// Hue rotation in degrees.
    pub hue: f32,
    /// Per-channel multiplier, e.g. to match projectors.
    pub gain: [f32; 3],
}

impl Default for ColorAdjust {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            hue: 0.0,
            gain: [1.0; 3],
        }
    }
}

impl ColorAdjust {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

fn full_frame() -> [[f32; 2]; 4] {
    Surface::FULL
}

/// How a surface combines with what is below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Screen,
    Multiply,
}

impl BlendMode {
    pub const ALL: [BlendMode; 4] = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Screen,
        BlendMode::Multiply,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Add => "Add",
            BlendMode::Screen => "Screen",
            BlendMode::Multiply => "Multiply",
        }
    }
}

/// The visible shape inside the surface's four corners. Triangles and
/// circles are masks on the perspective-warped quad, so they warp correctly
/// and keep the same four handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Shape {
    #[default]
    Quad,
    /// Apex at the middle of the top edge.
    Triangle,
    /// Ellipse touching all four edges.
    Circle,
}

impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Quad, Shape::Triangle, Shape::Circle];

    /// Closed outline in surface uv space (0..1), for drawing guides.
    pub fn outline_uv(self, segments: usize) -> Vec<[f32; 2]> {
        match self {
            Shape::Quad => vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            Shape::Triangle => vec![[0.5, 0.0], [1.0, 1.0], [0.0, 1.0]],
            Shape::Circle => (0..segments.max(8))
                .map(|i| {
                    let a = i as f32 / segments.max(8) as f32 * std::f32::consts::TAU;
                    [0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()]
                })
                .collect(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Shape::Quad => "Quad",
            Shape::Triangle => "Triangle",
            Shape::Circle => "Circle",
        }
    }
}

impl Surface {
    /// Default placement: a centered rectangle covering the middle of the output.
    pub const DEFAULT_CORNERS: [[f32; 2]; 4] = [[0.2, 0.2], [0.8, 0.2], [0.8, 0.8], [0.2, 0.8]];
    /// The whole media frame (default input region).
    pub const FULL: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    pub fn new(id: SurfaceId, output: OutputId) -> Self {
        Self {
            id,
            name: format!("Quad {}", id.0),
            output,
            media: None,
            corners: Self::DEFAULT_CORNERS,
            opacity: 1.0,
            visible: true,
            blend: BlendMode::Normal,
            shape: Shape::Quad,
            input: Self::FULL,
            mesh: None,
            mask: None,
            color: ColorAdjust::default(),
        }
    }

    /// Moves the whole surface (corners and mesh) by `d` in output space.
    pub fn translate(&mut self, d: [f32; 2]) {
        for c in &mut self.corners {
            c[0] += d[0];
            c[1] += d[1];
        }
        if let Some(mesh) = &mut self.mesh {
            for p in &mut mesh.points {
                p[0] += d[0];
                p[1] += d[1];
            }
        }
    }

    /// Keeps `corners` equal to the mesh's outer corners after a mesh edit.
    pub fn sync_corners_from_mesh(&mut self) {
        if let Some(mesh) = &self.mesh {
            self.corners = mesh.corners();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_moves_mesh_too() {
        let mut s = Surface::new(SurfaceId(1), OutputId(1));
        s.mesh = Some(Mesh {
            cols: 2,
            rows: 2,
            points: s.corners.to_vec(),
            smooth: false,
        });
        s.mesh.as_mut().unwrap().points.swap(2, 3); // row-major: TL TR BL BR
        s.translate([0.1, -0.1]);
        assert!((s.corners[0][0] - 0.3).abs() < 1e-6);
        assert!((s.mesh.as_ref().unwrap().points[0][1] - 0.1).abs() < 1e-6);
        s.mesh.as_mut().unwrap().points[3] = [0.95, 0.95];
        s.sync_corners_from_mesh();
        assert_eq!(s.corners[2], [0.95, 0.95]);
    }

    #[test]
    fn old_show_files_get_defaults() {
        // A surface saved before M5 has none of the new fields.
        let old = r#"(id: SurfaceId(1), name: "Q", output: OutputId(1), media: None,
            corners: ((0.2, 0.2), (0.8, 0.2), (0.8, 0.8), (0.2, 0.8)), opacity: 1.0, visible: true)"#;
        let s: Surface = ron::from_str(old).unwrap();
        assert!(s.mesh.is_none() && s.mask.is_none());
        assert!(s.color.is_identity());
        assert_eq!(s.input, Surface::FULL);
    }
}

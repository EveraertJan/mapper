//! Where a surface's texture lands on the output, for editors and guides:
//! the perspective quad or the mesh, behind one interface.

use rustmap_core::{Shape, Surface};
use rustmap_geom::{Mat3, MeshGrid, Quad, Tessellation, Vec2, is_convex, project, square_to_quad};

/// Subdivisions per mesh cell for outlines and hit tests (finer than enough on screen).
const OUTLINE_STEPS: usize = 8;

/// A surface's warp, ready for mapping points both ways.
pub enum Warp {
    Perspective {
        quad: Quad,
        forward: Mat3,
        inverse: Option<Mat3>,
    },
    Mesh {
        grid: MeshGrid,
        tess: Tessellation,
    },
}

impl Warp {
    pub fn of(s: &Surface) -> Self {
        if let Some(m) = &s.mesh
            && let Some(grid) =
                MeshGrid::from_arrays(m.cols as usize, m.rows as usize, &m.points, m.smooth)
        {
            let tess = grid.tessellate(OUTLINE_STEPS);
            return Warp::Mesh { grid, tess };
        }
        let quad = rustmap_geom::quad_from_arrays(&s.corners);
        let forward = square_to_quad(&quad);
        let inverse = (is_convex(&quad) && forward.determinant().abs() > 1e-12)
            .then(|| forward.inverse())
            .filter(|m| m.is_finite());
        Warp::Perspective {
            quad,
            forward,
            inverse,
        }
    }

    /// Surface uv (0..1) → output position.
    pub fn map(&self, uv: Vec2) -> Vec2 {
        match self {
            Warp::Perspective { quad, forward, .. } => project(forward, uv).unwrap_or_else(|| {
                quad[0]
                    .lerp(quad[1], uv.x)
                    .lerp(quad[3].lerp(quad[2], uv.x), uv.y)
            }),
            Warp::Mesh { grid, .. } => grid.eval(uv),
        }
    }

    /// Output position → surface uv, if the point is on the surface (meshes)
    /// or the quad can be inverted (perspective; works outside the quad too).
    pub fn uv_at(&self, p: Vec2) -> Option<Vec2> {
        match self {
            Warp::Perspective { inverse, .. } => inverse.as_ref().and_then(|m| project(m, p)),
            Warp::Mesh { tess, .. } => tess.uv_at(p),
        }
    }

    /// Maps a polyline given in uv, subdividing each edge so it follows curves.
    pub fn map_polyline(&self, uv: &[[f32; 2]], closed: bool) -> Vec<Vec2> {
        let steps = match self {
            Warp::Perspective { .. } => 1,
            Warp::Mesh { .. } => 24,
        };
        let n = uv.len();
        let edges = if closed { n } else { n.saturating_sub(1) };
        let mut out = Vec::with_capacity(edges * steps + 1);
        for i in 0..edges {
            let a = Vec2::from(uv[i]);
            let b = Vec2::from(uv[(i + 1) % n]);
            for k in 0..steps {
                out.push(self.map(a.lerp(b, k as f32 / steps as f32)));
            }
        }
        if !closed && let Some(last) = uv.last() {
            out.push(self.map(Vec2::from(*last)));
        }
        out
    }

    /// The visible outline (quad, triangle or circle).
    pub fn outline(&self, shape: Shape) -> Vec<Vec2> {
        self.map_polyline(&shape.outline_uv(64), true)
    }

    /// Interior grid lines of a mesh (empty for perspective quads).
    pub fn grid_lines(&self) -> Vec<Vec<Vec2>> {
        let Warp::Mesh { grid, .. } = self else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let line = |a: [f32; 2], b: [f32; 2]| self.map_polyline(&[a, b], false);
        for c in 1..grid.cols - 1 {
            let u = c as f32 / (grid.cols - 1) as f32;
            lines.push(line([u, 0.0], [u, 1.0]));
        }
        for r in 1..grid.rows - 1 {
            let v = r as f32 / (grid.rows - 1) as f32;
            lines.push(line([0.0, v], [1.0, v]));
        }
        lines
    }

    /// Whether output point `p` is on the surface.
    pub fn contains(&self, p: Vec2) -> bool {
        match self {
            Warp::Perspective { quad, .. } => rustmap_geom::contains(quad, p),
            Warp::Mesh { tess, .. } => tess.uv_at(p).is_some(),
        }
    }
}

/// The handles that move a surface in the Output view, in output space:
/// its four corners, or every mesh point.
pub fn warp_handles(s: &Surface) -> Vec<Vec2> {
    match &s.mesh {
        Some(m) => m.points.iter().map(|p| Vec2::from(*p)).collect(),
        None => s.corners.iter().map(|p| Vec2::from(*p)).collect(),
    }
}

/// Moves warp handle `i` (see [`warp_handles`]) to `p`, keeping corners and mesh in sync.
pub fn set_warp_handle(s: &mut Surface, i: usize, p: Vec2) {
    match &mut s.mesh {
        Some(m) => {
            if let Some(pt) = m.points.get_mut(i) {
                *pt = p.to_array();
            }
            s.sync_corners_from_mesh();
        }
        None => {
            if let Some(c) = s.corners.get_mut(i) {
                *c = p.to_array();
            }
        }
    }
}

/// Mesh index of outer corner `corner` (TL, TR, BR, BL), or the corner itself without a mesh.
pub fn corner_handle(s: &Surface, corner: usize) -> usize {
    match &s.mesh {
        Some(m) => {
            let (c, r) = (m.cols - 1, m.rows - 1);
            [(0, 0), (c, 0), (c, r), (0, r)]
                .get(corner)
                .map_or(0, |&(col, row)| m.point_index(col, row))
        }
        None => corner,
    }
}

/// Mask points in output space (where they show on the warped surface).
pub fn mask_handles(s: &Surface, warp: &Warp) -> Vec<Vec2> {
    s.mask.as_ref().map_or_else(Vec::new, |m| {
        m.points.iter().map(|p| warp.map(Vec2::from(*p))).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustmap_core::{Mesh, OutputId, SurfaceId};

    #[test]
    fn perspective_round_trip() {
        let mut s = Surface::new(SurfaceId(1), OutputId(1));
        s.corners = [[0.1, 0.2], [0.9, 0.1], [0.8, 0.9], [0.2, 0.7]];
        let w = Warp::of(&s);
        let uv = Vec2::new(0.3, 0.6);
        let back = w.uv_at(w.map(uv)).unwrap();
        assert!((back - uv).length() < 1e-4);
    }

    #[test]
    fn mesh_round_trip_and_handles() {
        let mut s = Surface::new(SurfaceId(1), OutputId(1));
        let grid = MeshGrid::from_quad(&rustmap_geom::quad_from_arrays(&s.corners), 3, 3, true);
        s.mesh = Some(Mesh {
            cols: 3,
            rows: 3,
            points: grid.points_as_arrays(),
            smooth: true,
        });
        assert_eq!(warp_handles(&s).len(), 9);
        set_warp_handle(&mut s, 4, Vec2::new(0.55, 0.45));
        let br = corner_handle(&s, 2);
        set_warp_handle(&mut s, br, Vec2::new(0.9, 0.9));
        assert_eq!(s.corners[2], [0.9, 0.9]);
        let w = Warp::of(&s);
        assert!((w.map(Vec2::splat(0.5)) - Vec2::new(0.55, 0.45)).length() < 1e-5);
        let uv = Vec2::new(0.7, 0.2);
        let back = w.uv_at(w.map(uv)).unwrap();
        assert!((back - uv).length() < 2e-2, "{back}");
        assert!(w.contains(Vec2::splat(0.5)));
        assert!(!w.contains(Vec2::splat(0.05)));
        assert_eq!(w.grid_lines().len(), 2);
    }
}

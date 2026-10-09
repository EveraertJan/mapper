//! Mesh warping: a grid of control points that bends a surface onto curved
//! or uneven objects.
//!
//! Points are stored row-major, `cols × rows` of them, in normalized output
//! space. Grid coordinate `(u, v)` in 0..1 maps through the points either
//! bilinearly or with a Catmull-Rom spline, which passes exactly through
//! every control point and stays smooth across cells.

use glam::Vec2;

use crate::homography::{project, square_to_quad};
use crate::quad::Quad;

/// A control grid. `cols`, `rows` ≥ 2.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshGrid {
    pub cols: usize,
    pub rows: usize,
    pub points: Vec<Vec2>,
    /// Catmull-Rom (true) or bilinear (false) between points.
    pub smooth: bool,
}

impl MeshGrid {
    /// A grid that reproduces the perspective quad exactly at its points, so
    /// switching a surface to mesh warping doesn't move anything.
    pub fn from_quad(q: &Quad, cols: usize, rows: usize, smooth: bool) -> Self {
        let (cols, rows) = (cols.max(2), rows.max(2));
        let m = square_to_quad(q);
        let mut points = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                let uv = Vec2::new(c as f32 / (cols - 1) as f32, r as f32 / (rows - 1) as f32);
                points.push(project(&m, uv).unwrap_or_else(|| bilinear(q, uv)));
            }
        }
        Self {
            cols,
            rows,
            points,
            smooth,
        }
    }

    /// From plain arrays (how show files store meshes). `None` if the point
    /// count doesn't match `cols × rows` or the grid is smaller than 2×2.
    pub fn from_arrays(
        cols: usize,
        rows: usize,
        points: &[[f32; 2]],
        smooth: bool,
    ) -> Option<Self> {
        (cols >= 2 && rows >= 2 && points.len() == cols * rows).then(|| Self {
            cols,
            rows,
            points: points.iter().map(|&p| Vec2::from(p)).collect(),
            smooth,
        })
    }

    pub fn points_as_arrays(&self) -> Vec<[f32; 2]> {
        self.points.iter().map(|p| p.to_array()).collect()
    }

    pub fn point(&self, c: usize, r: usize) -> Vec2 {
        self.points[r.min(self.rows - 1) * self.cols + c.min(self.cols - 1)]
    }

    /// The four outer corners (TL, TR, BR, BL).
    pub fn corners(&self) -> Quad {
        [
            self.point(0, 0),
            self.point(self.cols - 1, 0),
            self.point(self.cols - 1, self.rows - 1),
            self.point(0, self.rows - 1),
        ]
    }

    /// Position of grid coordinate `uv` (0..1 each).
    pub fn eval(&self, uv: Vec2) -> Vec2 {
        let gx = uv.x.clamp(0.0, 1.0) * (self.cols - 1) as f32;
        let gy = uv.y.clamp(0.0, 1.0) * (self.rows - 1) as f32;
        let ci = (gx.floor() as usize).min(self.cols - 2);
        let ri = (gy.floor() as usize).min(self.rows - 2);
        let (tx, ty) = (gx - ci as f32, gy - ri as f32);

        if !self.smooth {
            let a = self.point(ci, ri).lerp(self.point(ci + 1, ri), tx);
            let b = self.point(ci, ri + 1).lerp(self.point(ci + 1, ri + 1), tx);
            return a.lerp(b, ty);
        }

        // Catmull-Rom over the 4×4 neighbourhood; at the borders the missing
        // neighbours are extrapolated linearly so edges stay straight-ish
        // instead of curling.
        let p = |c: isize, r: isize| -> Vec2 {
            let cc = c.clamp(0, self.cols as isize - 1) as usize;
            let rr = r.clamp(0, self.rows as isize - 1) as usize;
            let mut v = self.point(cc, rr);
            if c < 0 {
                v = 2.0 * self.point(0, rr) - self.point(1, rr);
            } else if c >= self.cols as isize {
                v = 2.0 * self.point(self.cols - 1, rr) - self.point(self.cols - 2, rr);
            }
            if r < 0 || r >= self.rows as isize {
                let (edge, inner) = if r < 0 {
                    (0, 1)
                } else {
                    (self.rows - 1, self.rows - 2)
                };
                let col_point = |rr: usize| {
                    if c < 0 {
                        2.0 * self.point(0, rr) - self.point(1, rr)
                    } else if c >= self.cols as isize {
                        2.0 * self.point(self.cols - 1, rr) - self.point(self.cols - 2, rr)
                    } else {
                        self.point(cc, rr)
                    }
                };
                v = 2.0 * col_point(edge) - col_point(inner);
            }
            v
        };
        let (c0, r0) = (ci as isize, ri as isize);
        let row = |r: isize| catmull_rom(p(c0 - 1, r), p(c0, r), p(c0 + 1, r), p(c0 + 2, r), tx);
        catmull_rom(row(r0 - 1), row(r0), row(r0 + 1), row(r0 + 2), ty)
    }

    /// Same shape with a different number of control points.
    pub fn resample(&self, cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(2), rows.max(2));
        let mut points = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                points.push(self.eval(Vec2::new(
                    c as f32 / (cols - 1) as f32,
                    r as f32 / (rows - 1) as f32,
                )));
            }
        }
        Self {
            cols,
            rows,
            points,
            smooth: self.smooth,
        }
    }

    /// Triangles covering the surface: positions, texture coordinates and
    /// indices. `per_cell` subdivisions per grid cell along each axis.
    pub fn tessellate(&self, per_cell: usize) -> Tessellation {
        let per_cell = per_cell.max(1);
        let nx = (self.cols - 1) * per_cell + 1;
        let ny = (self.rows - 1) * per_cell + 1;
        let mut positions = Vec::with_capacity(nx * ny);
        let mut uvs = Vec::with_capacity(nx * ny);
        for y in 0..ny {
            for x in 0..nx {
                let uv = Vec2::new(x as f32 / (nx - 1) as f32, y as f32 / (ny - 1) as f32);
                positions.push(self.eval(uv));
                uvs.push(uv);
            }
        }
        let mut indices = Vec::with_capacity((nx - 1) * (ny - 1) * 6);
        for y in 0..ny - 1 {
            for x in 0..nx - 1 {
                let i = (y * nx + x) as u32;
                let (r, d, rd) = (i + 1, i + nx as u32, i + nx as u32 + 1);
                indices.extend_from_slice(&[i, r, rd, i, rd, d]);
            }
        }
        Tessellation {
            positions,
            uvs,
            indices,
        }
    }

    /// The boundary, clockwise from the top-left, for outlines and hit tests.
    pub fn outline(&self, per_cell: usize) -> Vec<Vec2> {
        let n = per_cell.max(1);
        let (sx, sy) = ((self.cols - 1) * n, (self.rows - 1) * n);
        let mut out = Vec::with_capacity(2 * (sx + sy));
        let at =
            |x: usize, y: usize| self.eval(Vec2::new(x as f32 / sx as f32, y as f32 / sy as f32));
        out.extend((0..sx).map(|x| at(x, 0)));
        out.extend((0..sy).map(|y| at(sx, y)));
        out.extend((1..=sx).rev().map(|x| at(x, sy)));
        out.extend((1..=sy).rev().map(|y| at(0, y)));
        out
    }
}

/// Output of [`MeshGrid::tessellate`].
#[derive(Debug, Clone, Default)]
pub struct Tessellation {
    pub positions: Vec<Vec2>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
}

impl Tessellation {
    /// The surface texture coordinate under output point `p`, if `p` is on
    /// the surface. Used to place mask points on warped surfaces.
    pub fn uv_at(&self, p: Vec2) -> Option<Vec2> {
        for tri in self.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i as usize);
            let (pa, pb, pc) = (self.positions[a], self.positions[b], self.positions[c]);
            let v0 = pb - pa;
            let v1 = pc - pa;
            let v2 = p - pa;
            let den = v0.perp_dot(v1);
            if den.abs() < 1e-12 {
                continue;
            }
            let s = v2.perp_dot(v1) / den;
            let t = v0.perp_dot(v2) / den;
            if s >= -1e-5 && t >= -1e-5 && s + t <= 1.0 + 1e-5 {
                let (ua, ub, uc) = (self.uvs[a], self.uvs[b], self.uvs[c]);
                return Some(ua + (ub - ua) * s + (uc - ua) * t);
            }
        }
        None
    }
}

fn catmull_rom(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn bilinear(q: &Quad, uv: Vec2) -> Vec2 {
    q[0].lerp(q[1], uv.x).lerp(q[3].lerp(q[2], uv.x), uv.y)
}

/// Where to insert a new point into a closed polygon so it lands on the
/// nearest edge: returns the index the point should get.
pub fn polygon_insert_index(points: &[Vec2], p: Vec2) -> usize {
    if points.len() < 2 {
        return points.len();
    }
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            (i, (a + ab * t - p).length())
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map_or(n, |(i, _)| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keystone() -> Quad {
        [
            Vec2::new(0.1, 0.05),
            Vec2::new(0.9, 0.15),
            Vec2::new(0.8, 0.95),
            Vec2::new(0.2, 0.7),
        ]
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn from_quad_matches_the_homography_at_control_points() {
        let q = keystone();
        let m = square_to_quad(&q);
        let g = MeshGrid::from_quad(&q, 4, 3, true);
        assert_eq!(g.points.len(), 12);
        for r in 0..3 {
            for c in 0..4 {
                let uv = Vec2::new(c as f32 / 3.0, r as f32 / 2.0);
                assert!(close(g.point(c, r), project(&m, uv).unwrap()));
            }
        }
        for (a, b) in g.corners().iter().zip(q) {
            assert!(close(*a, b), "corners are the quad's corners");
        }
    }

    #[test]
    fn interpolation_passes_through_control_points() {
        for smooth in [false, true] {
            let mut g = MeshGrid::from_quad(&keystone(), 4, 4, smooth);
            g.points[5] += Vec2::new(0.05, -0.03); // bend an inner point
            for r in 0..4 {
                for c in 0..4 {
                    let uv = Vec2::new(c as f32 / 3.0, r as f32 / 3.0);
                    assert!(
                        close(g.eval(uv), g.point(c, r)),
                        "smooth={smooth} at ({c},{r})"
                    );
                }
            }
        }
    }

    #[test]
    fn flat_grid_is_flat_between_points() {
        // A regular grid of a rectangle: smooth interpolation must stay linear.
        let rect = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let g = MeshGrid::from_quad(&rect, 5, 5, true);
        for (u, v) in [(0.1, 0.2), (0.33, 0.77), (0.95, 0.05)] {
            assert!(close(g.eval(Vec2::new(u, v)), Vec2::new(u, v)));
        }
    }

    #[test]
    fn resample_keeps_the_shape() {
        let mut g = MeshGrid::from_quad(&keystone(), 3, 3, true);
        g.points[4] += Vec2::new(0.04, 0.02);
        let fine = g.resample(7, 5);
        for uv in [
            Vec2::new(0.5, 0.5),
            Vec2::new(0.25, 0.75),
            Vec2::new(1.0, 0.0),
        ] {
            assert!((fine.eval(uv) - g.eval(uv)).length() < 2e-3, "{uv}");
        }
    }

    #[test]
    fn tessellation_and_inverse_round_trip() {
        let mut g = MeshGrid::from_quad(&keystone(), 4, 3, true);
        g.points[5] += Vec2::new(0.03, 0.03);
        let t = g.tessellate(8);
        assert_eq!(t.positions.len(), 25 * 17);
        assert_eq!(t.indices.len(), 24 * 16 * 6);
        let uv = Vec2::new(0.37, 0.61);
        let p = g.eval(uv);
        let back = t.uv_at(p).unwrap();
        assert!((back - uv).length() < 5e-3, "{back} vs {uv}");
        assert!(t.uv_at(Vec2::new(-1.0, -1.0)).is_none());
    }

    #[test]
    fn outline_is_closed_around_the_mesh() {
        let g = MeshGrid::from_quad(&keystone(), 3, 3, false);
        let o = g.outline(4);
        assert_eq!(o.len(), 2 * (8 + 8));
        assert!(close(o[0], keystone()[0]));
        assert!(crate::quad::contains(&keystone(), g.eval(Vec2::splat(0.5))));
    }

    #[test]
    fn insert_index_picks_nearest_edge() {
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        assert_eq!(polygon_insert_index(&square, Vec2::new(0.5, -0.1)), 1); // top edge
        assert_eq!(polygon_insert_index(&square, Vec2::new(1.1, 0.5)), 2); // right edge
        assert_eq!(polygon_insert_index(&square, Vec2::new(-0.1, 0.5)), 4); // left edge (closing)
    }
}

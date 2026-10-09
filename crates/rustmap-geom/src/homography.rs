use glam::{Mat3, Vec2, Vec3};

use crate::Quad;

/// Projective map from the unit square onto `q`.
///
/// `M * (u, v, 1) = (X, Y, W)` and the point lands at `(X/W, Y/W)`, with
/// `(0,0) (1,0) (1,1) (0,1)` going to `q[0] q[1] q[2] q[3]`.
///
/// Closed form from Heckbert, "Fundamentals of Texture Mapping and Image
/// Warping" (1989). W is 1 at `q[0]`, which keeps it well-scaled for the GPU.
pub fn square_to_quad(q: &Quad) -> Mat3 {
    let [p0, p1, p2, p3] = *q;
    let d1 = p1 - p2;
    let d2 = p3 - p2;
    let d3 = p0 - p1 + p2 - p3;

    let (g, h) = if d3.x.abs() < 1e-12 && d3.y.abs() < 1e-12 {
        // Parallelogram: the map is affine.
        (0.0, 0.0)
    } else {
        let den = d1.perp_dot(d2);
        (d3.perp_dot(d2) / den, d1.perp_dot(d3) / den)
    };

    let a = p1.x - p0.x + g * p1.x;
    let b = p3.x - p0.x + h * p3.x;
    let d = p1.y - p0.y + g * p1.y;
    let e = p3.y - p0.y + h * p3.y;

    Mat3::from_cols(
        Vec3::new(a, d, g),
        Vec3::new(b, e, h),
        Vec3::new(p0.x, p0.y, 1.0),
    )
}

/// Projective map taking quad `src` onto quad `dst`, corner for corner.
/// `None` if `src` is degenerate (three corners on a line).
pub fn homography(src: &Quad, dst: &Quad) -> Option<Mat3> {
    let s = square_to_quad(src);
    if s.determinant().abs() < 1e-12 || !s.is_finite() {
        return None;
    }
    let m = square_to_quad(dst) * s.inverse();
    m.is_finite().then_some(m)
}

/// Applies a homography to a point. `None` if the point maps to infinity.
pub fn project(m: &Mat3, p: Vec2) -> Option<Vec2> {
    let r = *m * p.extend(1.0);
    (r.z.abs() > 1e-12).then(|| r.truncate() / r.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    const UNIT: Quad = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];

    /// A keystoned quad like a projector pointed up at a wall.
    fn keystone() -> Quad {
        [
            Vec2::new(0.1, 0.05),
            Vec2::new(0.9, 0.15),
            Vec2::new(0.8, 0.95),
            Vec2::new(0.2, 0.7),
        ]
    }

    #[test]
    fn unit_square_is_identity() {
        let m = square_to_quad(&UNIT);
        assert!(m.abs_diff_eq(Mat3::IDENTITY, 1e-6));
    }

    #[test]
    fn corners_map_to_corners() {
        let q = keystone();
        let m = square_to_quad(&q);
        for (uv, expected) in UNIT.iter().zip(q) {
            assert!(approx(project(&m, *uv).unwrap(), expected));
        }
    }

    #[test]
    fn center_maps_to_diagonal_intersection() {
        // The defining property of a perspective map (and what an affine
        // two-triangle split gets wrong): the texture center lands where the
        // quad's diagonals cross.
        let q = keystone();
        let center = project(&square_to_quad(&q), Vec2::splat(0.5)).unwrap();
        let (a, b) = (q[0], q[2] - q[0]);
        let (c, d) = (q[1], q[3] - q[1]);
        let t = (c - a).perp_dot(d) / b.perp_dot(d);
        assert!(approx(center, a + b * t), "{center} vs {}", a + b * t);
    }

    #[test]
    fn w_is_one_at_first_corner_and_positive_for_convex() {
        let m = square_to_quad(&keystone());
        for uv in UNIT {
            let w = (m * uv.extend(1.0)).z;
            assert!(w > 0.0);
        }
        assert!(((m * Vec3::new(0.0, 0.0, 1.0)).z - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parallelogram_is_affine() {
        let q = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(3.0, 1.0),
            Vec2::new(1.0, 1.0),
        ];
        let m = square_to_quad(&q);
        assert_eq!(m.x_axis.z, 0.0);
        assert_eq!(m.y_axis.z, 0.0);
        assert!(approx(
            project(&m, Vec2::splat(0.5)).unwrap(),
            Vec2::new(1.5, 0.5)
        ));
    }

    #[test]
    fn quad_to_quad_round_trip() {
        let src = [
            Vec2::new(0.2, 0.1),
            Vec2::new(0.7, 0.2),
            Vec2::new(0.75, 0.8),
            Vec2::new(0.1, 0.9),
        ];
        let dst = keystone();
        let m = homography(&src, &dst).unwrap();
        for (s, d) in src.iter().zip(dst) {
            assert!(approx(project(&m, *s).unwrap(), d));
        }
        let back = m.inverse();
        let p = Vec2::new(0.4, 0.5);
        assert!(approx(project(&back, project(&m, p).unwrap()).unwrap(), p));
    }

    #[test]
    fn degenerate_source_is_rejected() {
        let line = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.2, 0.0),
        ];
        assert!(homography(&line, &keystone()).is_none());
    }
}

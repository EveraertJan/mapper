use glam::Vec2;

use crate::homography::square_to_quad;

/// Four corners: top-left, top-right, bottom-right, bottom-left.
pub type Quad = [Vec2; 4];

pub fn quad_from_arrays(c: &[[f32; 2]; 4]) -> Quad {
    c.map(Vec2::from)
}

pub fn quad_to_arrays(q: &Quad) -> [[f32; 2]; 4] {
    q.map(|p| p.to_array())
}

/// True if the quad is strictly convex (either winding). Only convex quads
/// have a valid perspective mapping; bow-ties and arrowheads do not.
pub fn is_convex(q: &Quad) -> bool {
    let mut sign = 0.0_f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = q[(i + 2) % 4];
        let cross = (b - a).perp_dot(c - b);
        if cross.abs() < 1e-9 {
            return false;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Homogeneous W of the perspective map at each corner. The GPU uses these to
/// interpolate texture coordinates perspective-correctly. `None` if the quad
/// is not convex (some W would be zero or negative).
pub fn corner_weights(q: &Quad) -> Option<[f32; 4]> {
    if !is_convex(q) {
        return None;
    }
    let m = square_to_quad(q);
    let uv = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];
    let w = uv.map(|uv| (m * uv.extend(1.0)).z);
    w.iter().all(|w| *w > 1e-6 && w.is_finite()).then_some(w)
}

/// Maps surface-uv points onto the quad (perspective-correct), e.g. to draw
/// a triangle or circle outline. Falls back to bilinear for non-convex quads.
pub fn map_uv(q: &Quad, uvs: &[[f32; 2]]) -> Vec<Vec2> {
    let m = crate::homography::square_to_quad(q);
    let convex = is_convex(q);
    uvs.iter()
        .map(|&[u, v]| {
            if convex && let Some(p) = crate::homography::project(&m, Vec2::new(u, v)) {
                return p;
            }
            let top = q[0].lerp(q[1], u);
            let bottom = q[3].lerp(q[2], u);
            top.lerp(bottom, v)
        })
        .collect()
}

/// Point-in-polygon (even-odd), valid for any simple quad.
pub fn contains(q: &Quad, p: Vec2) -> bool {
    let mut inside = false;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 3) % 4];
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// Index of the corner closest to `p` within `max_dist`, measured after
/// multiplying by `scale` (so hit radii can be given in screen pixels).
pub fn nearest_corner(q: &Quad, p: Vec2, scale: Vec2, max_dist: f32) -> Option<usize> {
    q.iter()
        .enumerate()
        .map(|(i, c)| (i, ((*c - p) * scale).length()))
        .filter(|(_, d)| *d <= max_dist)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// An upright rectangle centered in the output, as large as possible within
/// `fill` (0..1) of the output while keeping `aspect` (width / height, in
/// pixels). Returned in normalized output space.
pub fn centered_rect(aspect: f32, output_px: [u32; 2], fill: f32) -> Quad {
    let out = Vec2::new(output_px[0].max(1) as f32, output_px[1].max(1) as f32);
    let max = out * fill.clamp(0.01, 1.0);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        out.x / out.y
    };
    let size_px = if max.x / max.y > aspect {
        Vec2::new(max.y * aspect, max.y)
    } else {
        Vec2::new(max.x, max.x / aspect)
    };
    let half = size_px / out * 0.5;
    let c = Vec2::splat(0.5);
    [
        c + Vec2::new(-half.x, -half.y),
        c + Vec2::new(half.x, -half.y),
        c + Vec2::new(half.x, half.y),
        c + Vec2::new(-half.x, half.y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Quad {
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ]
    }

    #[test]
    fn convexity() {
        assert!(is_convex(&square()));
        let mut ccw = square();
        ccw.reverse();
        assert!(is_convex(&ccw), "winding direction must not matter");

        let mut bowtie = square();
        bowtie.swap(2, 3);
        assert!(!is_convex(&bowtie));

        let mut arrow = square();
        arrow[2] = Vec2::new(0.3, 0.3); // pushed inside
        assert!(!is_convex(&arrow));

        let mut flat = square();
        flat[1] = Vec2::new(0.5, 0.0);
        flat[2] = Vec2::new(1.0, 0.0);
        assert!(!is_convex(&flat));
    }

    #[test]
    fn weights_only_for_convex() {
        assert_eq!(corner_weights(&square()), Some([1.0; 4]));
        let mut bowtie = square();
        bowtie.swap(2, 3);
        assert!(corner_weights(&bowtie).is_none());
    }

    #[test]
    fn containment() {
        let q = square();
        assert!(contains(&q, Vec2::new(0.5, 0.5)));
        assert!(!contains(&q, Vec2::new(1.5, 0.5)));
        assert!(!contains(&q, Vec2::new(-0.1, 0.5)));
    }

    #[test]
    fn nearest_corner_respects_radius_in_scaled_units() {
        let q = square();
        // 1920x1080 output: 0.004 normalized x = ~7.7 px.
        let scale = Vec2::new(1920.0, 1080.0);
        assert_eq!(
            nearest_corner(&q, Vec2::new(0.004, 0.0), scale, 10.0),
            Some(0)
        );
        assert_eq!(nearest_corner(&q, Vec2::new(0.01, 0.0), scale, 10.0), None);
        assert_eq!(
            nearest_corner(&q, Vec2::new(0.999, 0.999), scale, 10.0),
            Some(2)
        );
    }

    #[test]
    fn centered_rect_keeps_pixel_aspect() {
        // A 4:3 image on a 16:9 output, filling 80% of the height.
        let q = centered_rect(4.0 / 3.0, [1920, 1080], 0.8);
        let w_px = (q[1].x - q[0].x) * 1920.0;
        let h_px = (q[3].y - q[0].y) * 1080.0;
        assert!((w_px / h_px - 4.0 / 3.0).abs() < 1e-3);
        assert!((h_px - 864.0).abs() < 1e-2);
        assert!(((q[0].x + q[1].x) / 2.0 - 0.5).abs() < 1e-6);
    }
}

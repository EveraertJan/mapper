use glam::Vec2;

/// What a dragged point can snap to, in normalized output space.
#[derive(Debug, Default, Clone)]
pub struct SnapTargets {
    /// Points to snap onto exactly (other surfaces' corners, output corners).
    pub points: Vec<Vec2>,
    /// Vertical lines (x positions), e.g. output edges and center.
    pub xs: Vec<f32>,
    /// Horizontal lines (y positions).
    pub ys: Vec<f32>,
}

impl SnapTargets {
    /// Output corners, edges and center lines.
    pub fn output_frame() -> Self {
        Self {
            points: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            xs: vec![0.0, 0.5, 1.0],
            ys: vec![0.0, 0.5, 1.0],
        }
    }
}

/// Snaps `p` to the nearest target within `radius`.
///
/// `scale` converts normalized units to screen pixels so the radius feels the
/// same at any zoom. Points win over lines; lines snap each axis independently,
/// so a corner can slide along an output edge.
pub fn snap_point(p: Vec2, targets: &SnapTargets, scale: Vec2, radius: f32) -> Vec2 {
    let best_point = targets
        .points
        .iter()
        .map(|t| (*t, ((*t - p) * scale).length()))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((t, _)) = best_point {
        return t;
    }

    let nearest = |value: f32, lines: &[f32], s: f32| {
        lines
            .iter()
            .map(|l| (*l, ((l - value) * s).abs()))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(value, |(l, _)| l)
    };
    Vec2::new(
        nearest(p.x, &targets.xs, scale.x),
        nearest(p.y, &targets.ys, scale.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALE: Vec2 = Vec2::new(1000.0, 1000.0);

    #[test]
    fn snaps_to_point_within_radius() {
        let t = SnapTargets::output_frame();
        assert_eq!(
            snap_point(Vec2::new(0.005, 0.004), &t, SCALE, 8.0),
            Vec2::ZERO
        );
    }

    #[test]
    fn slides_along_edge() {
        let t = SnapTargets::output_frame();
        let p = snap_point(Vec2::new(0.003, 0.3), &t, SCALE, 8.0);
        assert_eq!(p, Vec2::new(0.0, 0.3));
    }

    #[test]
    fn leaves_far_points_alone() {
        let t = SnapTargets::output_frame();
        let p = Vec2::new(0.3, 0.3);
        assert_eq!(snap_point(p, &t, SCALE, 8.0), p);
    }
}

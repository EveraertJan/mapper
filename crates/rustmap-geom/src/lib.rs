//! RUSTMAP geometry.
//!
//! Conventions used everywhere in RUSTMAP:
//! - Surface positions live in **normalized output space**: `(0,0)` is the
//!   top-left of the output, `(1,1)` the bottom-right, y points down. That
//!   keeps a mapping valid when the output resolution changes.
//! - A [`Quad`] lists its corners as top-left, top-right, bottom-right,
//!   bottom-left, matching texture coordinates `(0,0) (1,0) (1,1) (0,1)`.

mod homography;
mod mesh;
mod quad;
mod snap;

pub use glam::{Mat3, Vec2, Vec3};
pub use homography::{homography, project, square_to_quad};
pub use mesh::{MeshGrid, Tessellation, polygon_insert_index};
pub use quad::{
    Quad, centered_rect, contains, corner_weights, is_convex, map_uv, nearest_corner,
    quad_from_arrays, quad_to_arrays,
};
pub use snap::{SnapTargets, snap_point};

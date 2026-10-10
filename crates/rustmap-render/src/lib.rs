//! RUSTMAP rendering.
//!
//! Every output renders into its own offscreen [`OutputTarget`] at the
//! projector's resolution. That one texture is then
//! - blitted to the projector window ([`Blitter`]), and
//! - shown as a preview in the control UI,
//!
//! so the operator always sees exactly the pixels the projector gets.

mod blit;
mod chain;
mod edge_blend;
mod gpu;
mod image;
mod led_sampler;
mod overlay;
mod quad;
mod readback;
mod surface;
mod target;
mod test_pattern;

pub use blit::{Blitter, fit_rect};
pub use chain::ChainShader;
pub use edge_blend::{EdgeBlendRenderer, EdgeBlendState};
pub use gpu::{Gpu, GpuError};
pub use image::GpuImage;
pub use led_sampler::{LedSampler, LedSamples};
pub use overlay::{OverlayBuilder, OverlayRenderer, Rgba};
pub use quad::{FilterPipelines, ImageBinding, QuadDraw, QuadFill, QuadRenderer, QuadState};
pub use readback::{FrameReadback, ReadbackFrame};
pub use surface::{SurfaceFrame, WindowSurface};
pub use target::{OUTPUT_FORMAT, OutputTarget};
pub use test_pattern::{PatternParams, PatternState, TestPatternRenderer};

/// Re-export so downstream crates use the exact same wgpu.
pub use wgpu;

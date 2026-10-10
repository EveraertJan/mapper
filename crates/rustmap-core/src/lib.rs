//! RUSTMAP scene model.
//!
//! This crate is deliberately free of GPU and UI dependencies so the whole
//! scene can be built, edited and tested headless.

pub mod file;
pub mod fixture;
pub mod history;
pub mod led;
pub mod lights;
pub mod media;
pub mod output;
pub mod playback;
pub mod project;
pub mod shaders;
pub mod surface;

pub use fixture::{
    ColorOrder, Corner, FIXTURE_MAX_PIXELS, Fixture, FixtureId, FixtureShape, LedSettings, Patch,
    PixelAddress, Protocol, UNIVERSE_SIZE, UniverseKey, Wiring,
};
pub use history::History;
pub use lights::{
    ChannelDef, ChannelKind, DmxFixture, DmxFixtureId, DmxType, DmxTypeId, ValueRange,
};
pub use media::{Media, MediaId, MediaSource, Playback};
pub use output::{EdgeBlend, OutputConfig, OutputId, TestPattern};
pub use playback::{
    Cue, CueId, CuePlayer, CueScope, DmxGroup, DmxGroupId, GroupControl, MediaCue, SurfaceLook,
    group_controls,
};
pub use project::{OscSettings, Project};
pub use shaders::{FilterUse, ParamDef, ShaderInfo};
pub use surface::{BlendMode, ColorAdjust, MASK_MAX_POINTS, Mask, Mesh, Shape, Surface, SurfaceId};

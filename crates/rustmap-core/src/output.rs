use serde::{Deserialize, Serialize};

/// Stable identifier of an output (projector / display).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutputId(pub u32);

impl std::fmt::Display for OutputId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// What an output shows when nothing is mapped onto it (or as an overlay while aligning).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TestPattern {
    /// Nothing: plain black.
    Off,
    /// Alignment grid with center cross, circles and a moving bar to spot stutter.
    Grid,
    /// SMPTE-ish color bars plus a gray ramp, for color and gamma checks.
    ColorBars,
    /// Flat color, for brightness / edge-blend checks.
    Solid([f32; 3]),
}

impl TestPattern {
    /// Index used by the shader.
    pub fn shader_index(&self) -> u32 {
        match self {
            TestPattern::Off => 0,
            TestPattern::Grid => 1,
            TestPattern::ColorBars => 2,
            TestPattern::Solid(_) => 3,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TestPattern::Off => "Off",
            TestPattern::Grid => "Grid",
            TestPattern::ColorBars => "Color bars",
            TestPattern::Solid(_) => "Solid",
        }
    }
}

/// Configuration of one output. The app reconciles real windows against this
/// every frame, so loading a project (later) simply makes windows appear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputConfig {
    pub id: OutputId,
    pub name: String,
    /// Render resolution in pixels. The output is rendered at this size and
    /// scaled to the window, so previews and the projector show the same pixels.
    pub resolution: [u32; 2],
    /// Whether the output window is open.
    pub enabled: bool,
    /// Monitor name to place the window on, as reported by the OS.
    pub monitor: Option<String>,
    /// Fullscreen on `monitor` (or the window's current monitor if `None`).
    pub fullscreen: bool,
    pub test_pattern: TestPattern,
    /// Ties the test-pattern grid to a known cell size in pixels.
    pub grid_cell_px: u32,
    /// Draw surface outlines and corner handles on the projector while editing.
    #[serde(default = "default_true")]
    pub show_guides: bool,
    /// Publish this output on the network as an NDI source.
    #[serde(default)]
    pub ndi_send: bool,
    /// Soft edges where this projector overlaps a neighbour.
    #[serde(default)]
    pub edge_blend: EdgeBlend,
}

/// Edge blending: each side fades to black over a fraction of the output,
/// so two overlapping projectors add up to an even image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeBlend {
    /// Width of each ramp as a fraction of the output (0..0.5).
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    /// Projector gamma the ramp compensates for (≈2.2).
    pub gamma: f32,
    /// Ramp shape: 1 = linear, 2–3 = S-curve that hides misalignment better.
    pub curve: f32,
}

impl Default for EdgeBlend {
    fn default() -> Self {
        Self {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            gamma: 2.2,
            curve: 2.0,
        }
    }
}

impl EdgeBlend {
    pub fn is_active(&self) -> bool {
        self.left > 0.0 || self.right > 0.0 || self.top > 0.0 || self.bottom > 0.0
    }

    /// Brightness factor of one ramp at position `t` (0 = outer edge, 1 = full
    /// brightness), as sent to the projector: S-curve, then inverse gamma so
    /// the light from both projectors sums to 1 in the overlap.
    pub fn ramp(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let p = self.curve.max(0.1);
        let s = if t < 0.5 {
            0.5 * (2.0 * t).powf(p)
        } else {
            1.0 - 0.5 * (2.0 * (1.0 - t)).powf(p)
        };
        s.powf(1.0 / self.gamma.max(0.1))
    }
}

fn default_true() -> bool {
    true
}

impl OutputConfig {
    pub fn new(id: OutputId) -> Self {
        Self {
            id,
            name: format!("Output {}", id.0),
            resolution: [1920, 1080],
            enabled: false,
            monitor: None,
            fullscreen: false,
            test_pattern: TestPattern::Grid,
            grid_cell_px: 120,
            show_guides: true,
            ndi_send: false,
            edge_blend: EdgeBlend::default(),
        }
    }

    /// Name this output gets as an NDI source (NDI adds the machine name).
    pub fn ndi_name(&self) -> String {
        format!("RUSTMAP {}", self.name)
    }

    pub fn aspect(&self) -> f32 {
        self.resolution[0] as f32 / self.resolution[1].max(1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_ramps_sum_to_full_light() {
        let b = EdgeBlend::default();
        // Light is the signal raised to gamma; two complementary ramps must add to 1.
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let light = b.ramp(t).powf(b.gamma) + b.ramp(1.0 - t).powf(b.gamma);
            assert!((light - 1.0).abs() < 1e-4, "t={t}: {light}");
        }
        assert_eq!(b.ramp(0.0), 0.0);
        assert!((b.ramp(1.0) - 1.0).abs() < 1e-6);
    }
}

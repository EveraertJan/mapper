//! LED output: collects sampled colors per fixture, builds the DMX
//! universes, and sends them at the show's LED frame rate.

use std::collections::HashMap;

use rustmap_core::led::{build_universes, light_name, patch_conflicts};
use rustmap_core::{CueId, CuePlayer, FixtureId, OutputId, Project, Protocol};
use rustmap_dmx::{DmxSender, UniverseData, parse_destination};
use rustmap_ui::DmxView;

/// Which fixture owns which run of samples on one output.
pub type SampleLayout = Vec<(FixtureId, usize)>;

/// What the GPU should sample on `output`: every enabled fixture's points,
/// and which fixture each run belongs to.
pub fn sample_points(project: &Project, output: OutputId) -> (Vec<[f32; 4]>, SampleLayout) {
    let mut points = Vec::new();
    let mut layout = Vec::new();
    for f in project.fixtures_on(output).filter(|f| f.enabled) {
        let p = f.sample_points();
        layout.push((f.id, p.len()));
        points.extend(p);
    }
    (points, layout)
}

#[derive(Default)]
pub struct LedOutput {
    sender: Option<DmxSender>,
    sender_error: Option<String>,
    colors: HashMap<FixtureId, Vec<[u8; 3]>>,
    /// Cue changes and fades.
    player: CuePlayer,
    /// Seconds since the last frame went out.
    clock: f64,
    pub view: DmxView,
}

impl LedOutput {
    /// Stores freshly read samples, split per fixture.
    pub fn take_samples(&mut self, layout: &SampleLayout, samples: &[[u8; 3]]) {
        let mut at = 0;
        for (id, n) in layout {
            let end = (at + n).min(samples.len());
            self.colors.insert(*id, samples[at..end].to_vec());
            at = end;
        }
    }

    /// Goes to a cue (writes its values into the fixtures, fades the output).
    pub fn go(&mut self, project: &mut Project, cue: CueId, now: f64) {
        self.player.go(project, cue, now);
    }

    /// Next or previous cue.
    pub fn step(&mut self, project: &mut Project, forward: bool, now: f64) {
        self.player.step(project, forward, now);
    }

    /// Re-applies the current cue.
    pub fn again(&mut self, project: &mut Project, now: f64) {
        self.player.again(project, now);
    }

    /// Stops a fade where it is (undo / redo).
    pub fn stop_fade(&mut self) {
        self.player.stop();
    }

    /// Videos cues asked to rewind.
    pub fn take_restarts(&mut self) -> Vec<rustmap_core::MediaId> {
        self.player.take_restarts()
    }

    /// Forgets the current cue (another show was opened).
    pub fn reset_cues(&mut self) {
        self.player.reset();
    }

    /// While a fade runs the fixtures change every frame; undo should wait.
    pub fn fading(&self) -> bool {
        self.player.fading()
    }

    /// Builds the universes, refreshes the monitor, and sends when due.
    pub fn update(&mut self, project: &mut Project, now: f64, dt: f64) {
        puffin::profile_function!();
        self.colors.retain(|id, _| project.fixture(*id).is_some());
        self.player.tick(project, now);
        let project: &Project = project;
        let universes = build_universes(project, &self.colors);

        let mut errors = Vec::new();
        for (later, earlier) in patch_conflicts(project) {
            errors.push(format!(
                "{} overlaps channels of {}",
                light_name(project, later),
                light_name(project, earlier)
            ));
        }

        // Universes as they go on the wire (also checks every destination).
        let mut frame = Vec::with_capacity(universes.len());
        for (key, data) in &universes {
            match parse_destination(&key.destination) {
                Ok(destination) => frame.push(UniverseData {
                    protocol: match key.protocol {
                        Protocol::ArtNet => rustmap_dmx::Protocol::ArtNet,
                        Protocol::Sacn => rustmap_dmx::Protocol::Sacn,
                    },
                    destination,
                    universe: key.universe,
                    data: Box::new(*data),
                }),
                Err(err) => {
                    let msg = format!("{key}: {err}");
                    if !errors.contains(&msg) {
                        errors.push(msg);
                    }
                }
            }
        }

        // Network.
        if !project.led.send || !project.has_lights() {
            self.sender = None;
            self.sender_error = None;
            self.clock = 0.0;
        } else {
            if self.sender.is_none() && self.sender_error.is_none() {
                match DmxSender::new(&format!("RUSTMAP {}", project.name)) {
                    Ok(s) => self.sender = Some(s),
                    Err(err) => self.sender_error = Some(format!("Can't open network: {err}")),
                }
            }
            let period = 1.0 / f64::from(project.led.fps.clamp(1.0, 120.0));
            self.clock += dt;
            if let Some(sender) = &self.sender
                && self.clock >= period
            {
                // Don't burst to catch up after a stall.
                self.clock = (self.clock - period).min(period);
                sender.send(frame);
            }
        }
        if let Some(err) = &self.sender_error {
            errors.push(err.clone());
        }
        let stats = self
            .sender
            .as_ref()
            .map(DmxSender::stats)
            .unwrap_or_default();
        if let Some(err) = stats.last_error {
            errors.push(format!("Network: {err}"));
        }

        self.view = DmxView {
            universes: universes
                .into_iter()
                .map(|(k, v)| (k, Box::new(v)))
                .collect(),
            packets: stats.packets,
            frames: stats.frames,
            errors,
            current_cue: self.player.current,
            fade: self.player.fade_progress(now),
        };
    }
}

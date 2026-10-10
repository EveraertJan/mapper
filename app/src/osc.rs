//! OSC remote control: keeps a listener running as the show asks and turns
//! what arrives into cue changes, blackout and grand master moves.

use rustmap_core::Project;
use rustmap_osc::{Command, CueRef, OscListener};
use rustmap_ui::OscView;

use crate::led::LedOutput;

/// Messages kept for the "Recent" list.
const RECENT: usize = 8;

#[derive(Default)]
pub struct OscInput {
    listener: Option<OscListener>,
    /// (listen, port) the listener was last set up for, so a port that fails
    /// to bind is not retried every frame.
    wanted: Option<(bool, u16)>,
    pub view: OscView,
}

impl OscInput {
    /// Starts or stops the listener to match the show, then applies every
    /// message that arrived since last frame.
    pub fn update(&mut self, project: &mut Project, led: &mut LedOutput, now: f64) {
        let want = (project.osc.listen, project.osc.port);
        if self.wanted != Some(want) {
            self.wanted = Some(want);
            self.listener = None; // free the old port first
            self.view.error = None;
            if want.0 {
                match OscListener::start(want.1) {
                    Ok(l) => self.listener = Some(l),
                    Err(e) => {
                        tracing::warn!("OSC: can't listen on port {}: {e}", want.1);
                        self.view.error = Some(format!("Port {}: {e}", want.1));
                    }
                }
            }
        }
        self.view.listening = self.listener.as_ref().map(OscListener::port);

        let Some(listener) = &self.listener else {
            return;
        };
        for r in listener.drain() {
            let note = match &r.command {
                Some(c) => apply(c, project, led, now),
                None => "ignored".to_owned(),
            };
            self.view.received += 1;
            self.view.recent.insert(
                0,
                format!("{} {}  ({}) → {note}", r.address, r.args, r.from.ip()),
            );
            self.view.recent.truncate(RECENT);
        }
    }
}

/// Carries out one command; returns what happened, for the log.
fn apply(c: &Command, project: &mut Project, led: &mut LedOutput, now: f64) -> String {
    match c {
        Command::GoCue(r) => {
            let found = match r {
                CueRef::Number(n) => project.cues.get(n - 1),
                CueRef::Name(name) => project
                    .cues
                    .iter()
                    .find(|c| c.name.trim().eq_ignore_ascii_case(name)),
            };
            match found.map(|c| (c.id, c.name.clone())) {
                Some((id, name)) => {
                    led.go(project, id, now);
                    format!("Go {name}")
                }
                None => "no such cue".to_owned(),
            }
        }
        Command::Next => {
            led.step(project, true, now);
            "Next".to_owned()
        }
        Command::Back => {
            led.step(project, false, now);
            "Back".to_owned()
        }
        Command::Current => {
            led.again(project, now);
            "Current".to_owned()
        }
        Command::ChaseRun(n, on) => match project.chases.get_mut(n - 1) {
            Some(c) => {
                c.running = on.unwrap_or(!c.running);
                format!(
                    "{} {}",
                    c.name,
                    if c.running { "running" } else { "stopped" }
                )
            }
            None => "no such chase".to_owned(),
        },
        Command::ChaseTap(n) => match project.chases.get(n - 1).map(|c| c.id) {
            Some(id) => {
                led.tap(project, id, now);
                "Tap".to_owned()
            }
            None => "no such chase".to_owned(),
        },
        Command::Blackout(on) => {
            let on = on.unwrap_or(!project.led.blackout);
            project.led.blackout = on;
            format!("Blackout {}", if on { "on" } else { "off" })
        }
        Command::Master(v) => {
            project.led.master = *v;
            format!("Master {:.0}%", v * 100.0)
        }
    }
}

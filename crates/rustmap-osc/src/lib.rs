//! OSC remote control.
//!
//! A listener thread receives OSC over UDP (any interface) and turns each
//! message into a [`Command`]; the app drains them once per frame. Unknown
//! addresses are reported (so the UI can show what arrived) but ignored.
//!
//! Addresses (case-insensitive, the `/rustmap` prefix is optional):
//!
//! | Address | Arguments | Does |
//! |---|---|---|
//! | `/rustmap/cue/go` | cue number (int/float) or name (string) | go to that cue |
//! | `/rustmap/cue/go/<n>` | none, or 1 (button press) | go to cue `n` |
//! | `/rustmap/cue/next`, `/rustmap/go` | none, or 1 | next cue |
//! | `/rustmap/cue/back`, `/rustmap/cue/prev` | none, or 1 | previous cue |
//! | `/rustmap/cue/current` | none, or 1 | re-apply the current cue |
//! | `/rustmap/chase/<n>/run` | 1 / 0, none toggles | start / stop chase n |
//! | `/rustmap/chase/<n>/tap` | none, or 1 | tap tempo for chase n |
//! | `/rustmap/blackout` | 0 / 1 (none: toggle) | blackout off / on |
//! | `/rustmap/master` | 0..1 | grand master |
//!
//! Cue numbers count from 1 in cue-list order. A trigger with a 0 argument
//! is a button *release* (TouchOSC, Lemur, QLab send 1 then 0) and is ignored.

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

use rosc::{OscMessage, OscPacket, OscType};

/// Default listening port (TouchOSC, Lemur and QLab users usually pick 8000).
pub const DEFAULT_PORT: u16 = 8000;

/// Which cue a message names.
#[derive(Debug, Clone, PartialEq)]
pub enum CueRef {
    /// 1-based position in the cue list.
    Number(usize),
    /// Cue name (matched ignoring case and surrounding spaces).
    Name(String),
}

/// What a message asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    GoCue(CueRef),
    Next,
    Back,
    /// Re-apply the current cue.
    Current,
    /// Start (true), stop (false) or toggle (None) chase `n` (1-based).
    ChaseRun(usize, Option<bool>),
    /// Tap tempo for chase `n`.
    ChaseTap(usize),
    /// `None`: toggle.
    Blackout(Option<bool>),
    Master(f32),
}

/// One message as it arrived, with what it was understood as.
#[derive(Debug, Clone)]
pub struct Received {
    pub address: String,
    pub args: String,
    pub from: SocketAddr,
    /// `None`: not a RUSTMAP address (or a button release).
    pub command: Option<Command>,
}

fn number(arg: &OscType) -> Option<f64> {
    match arg {
        OscType::Int(v) => Some(f64::from(*v)),
        OscType::Long(v) => Some(*v as f64),
        OscType::Float(v) => Some(f64::from(*v)),
        OscType::Double(v) => Some(*v),
        OscType::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        OscType::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// True unless the first argument says "released" (0 / false).
fn pressed(args: &[OscType]) -> bool {
    match args.first() {
        None => true,
        Some(a) => number(a).is_none_or(|v| v != 0.0),
    }
}

fn cue_number(v: f64) -> Option<CueRef> {
    (v >= 1.0 && v.fract() == 0.0).then_some(CueRef::Number(v as usize))
}

/// Understands one message (see the table in the crate docs).
pub fn parse(msg: &OscMessage) -> Option<Command> {
    let address = msg.addr.trim().to_ascii_lowercase();
    let path = address.strip_prefix("/rustmap").unwrap_or(&address);
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let args = &msg.args;
    match parts.as_slice() {
        ["cue", "go"] => match args.first()? {
            OscType::String(s) => match s.trim().parse::<f64>() {
                Ok(v) => cue_number(v).map(Command::GoCue),
                Err(_) => Some(Command::GoCue(CueRef::Name(s.trim().to_owned()))),
            },
            a => cue_number(number(a)?).map(Command::GoCue),
        },
        ["cue", "go", n] if pressed(args) => cue_number(n.parse().ok()?).map(Command::GoCue),
        ["cue", "next"] | ["go"] if pressed(args) => Some(Command::Next),
        ["cue", "back"] | ["cue", "prev"] | ["cue", "previous"] if pressed(args) => {
            Some(Command::Back)
        }
        ["cue", "current"] | ["cue", "again"] if pressed(args) => Some(Command::Current),
        ["chase", n, "run"] => {
            let n: usize = n.parse().ok().filter(|n| *n >= 1)?;
            Some(Command::ChaseRun(
                n,
                args.first().and_then(number).map(|v| v != 0.0),
            ))
        }
        ["chase", n, "tap"] if pressed(args) => {
            Some(Command::ChaseTap(n.parse().ok().filter(|n| *n >= 1)?))
        }
        ["blackout"] => Some(Command::Blackout(
            args.first().and_then(number).map(|v| v != 0.0),
        )),
        ["master"] => Some(Command::Master(
            number(args.first()?)?.clamp(0.0, 1.0) as f32
        )),
        _ => None,
    }
}

fn flatten(packet: OscPacket, out: &mut Vec<OscMessage>) {
    match packet {
        OscPacket::Message(m) => out.push(m),
        OscPacket::Bundle(b) => {
            for p in b.content {
                flatten(p, out);
            }
        }
    }
}

fn describe_args(args: &[OscType]) -> String {
    args.iter()
        .map(|a| match a {
            OscType::Int(v) => v.to_string(),
            OscType::Float(v) => format!("{v}"),
            OscType::Double(v) => format!("{v}"),
            OscType::Long(v) => v.to_string(),
            OscType::String(s) => format!("\"{s}\""),
            OscType::Bool(v) => v.to_string(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A running listener. Dropping it stops the thread and frees the port.
pub struct OscListener {
    port: u16,
    rx: Receiver<Received>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OscListener {
    /// Binds `0.0.0.0:port` and starts listening.
    pub fn start(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        // Wake up now and then to notice `stop`.
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let port = socket.local_addr()?.port();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = std::thread::Builder::new()
            .name("osc-in".into())
            .spawn(move || {
                let mut buf = vec![0u8; rosc::decoder::MTU];
                while !stop_thread.load(Ordering::Relaxed) {
                    let (n, from) = match socket.recv_from(&mut buf) {
                        Ok(r) => r,
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!("OSC receive failed: {e}");
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                    };
                    let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..n]) else {
                        tracing::debug!("ignoring {n} bytes from {from}: not OSC");
                        continue;
                    };
                    let mut messages = Vec::new();
                    flatten(packet, &mut messages);
                    for m in messages {
                        let r = Received {
                            command: parse(&m),
                            args: describe_args(&m.args),
                            address: m.addr,
                            from,
                        };
                        if tx.send(r).is_err() {
                            return;
                        }
                    }
                }
            })?;
        tracing::info!("listening for OSC on UDP port {port}");
        Ok(Self {
            port,
            rx,
            stop,
            thread: Some(thread),
        })
    }

    /// The port actually bound (useful when started on port 0).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Everything received since the last call.
    pub fn drain(&self) -> Vec<Received> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(r) => out.push(r),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

impl Drop for OscListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(addr: &str, args: Vec<OscType>) -> OscMessage {
        OscMessage {
            addr: addr.into(),
            args,
        }
    }

    #[test]
    fn cue_addresses() {
        use Command::*;
        let go = |n| Some(GoCue(CueRef::Number(n)));
        assert_eq!(parse(&msg("/rustmap/cue/go", vec![OscType::Int(3)])), go(3));
        assert_eq!(parse(&msg("/cue/go", vec![OscType::Float(2.0)])), go(2));
        assert_eq!(parse(&msg("/rustmap/cue/go", vec!["4".into()])), go(4));
        assert_eq!(
            parse(&msg("/rustmap/cue/go", vec![" Intro ".into()])),
            Some(GoCue(CueRef::Name("Intro".into())))
        );
        assert_eq!(parse(&msg("/rustmap/cue/go/5", vec![])), go(5));
        assert_eq!(
            parse(&msg("/RUSTMAP/Cue/Go/5", vec![OscType::Float(1.0)])),
            go(5)
        );
        assert_eq!(parse(&msg("/rustmap/cue/next", vec![])), Some(Next));
        assert_eq!(
            parse(&msg("/rustmap/go", vec![OscType::Int(1)])),
            Some(Next)
        );
        assert_eq!(parse(&msg("/rustmap/cue/prev", vec![])), Some(Back));
        assert_eq!(parse(&msg("/rustmap/cue/back", vec![])), Some(Back));
        assert_eq!(parse(&msg("/rustmap/cue/current", vec![])), Some(Current));
        assert_eq!(
            parse(&msg("/rustmap/chase/2/run", vec![OscType::Int(1)])),
            Some(ChaseRun(2, Some(true)))
        );
        assert_eq!(
            parse(&msg("/rustmap/chase/1/run", vec![])),
            Some(ChaseRun(1, None))
        );
        assert_eq!(
            parse(&msg("/rustmap/chase/1/tap", vec![])),
            Some(ChaseTap(1))
        );
        assert_eq!(
            parse(&msg("/rustmap/chase/1/tap", vec![OscType::Int(0)])),
            None
        );
        assert_eq!(parse(&msg("/rustmap/chase/0/run", vec![])), None);
    }

    #[test]
    fn button_releases_and_bad_numbers_are_ignored() {
        assert_eq!(
            parse(&msg("/rustmap/cue/next", vec![OscType::Float(0.0)])),
            None
        );
        assert_eq!(
            parse(&msg("/rustmap/cue/go/2", vec![OscType::Int(0)])),
            None
        );
        assert_eq!(parse(&msg("/rustmap/cue/go", vec![OscType::Int(0)])), None);
        assert_eq!(
            parse(&msg("/rustmap/cue/go", vec![OscType::Float(1.5)])),
            None
        );
        assert_eq!(parse(&msg("/rustmap/cue/go", vec![])), None);
        assert_eq!(parse(&msg("/other/thing", vec![])), None);
    }

    #[test]
    fn blackout_and_master() {
        use Command::*;
        assert_eq!(
            parse(&msg("/rustmap/blackout", vec![])),
            Some(Blackout(None))
        );
        assert_eq!(
            parse(&msg("/rustmap/blackout", vec![OscType::Int(1)])),
            Some(Blackout(Some(true)))
        );
        assert_eq!(
            parse(&msg("/rustmap/blackout", vec![OscType::Bool(false)])),
            Some(Blackout(Some(false)))
        );
        assert_eq!(
            parse(&msg("/rustmap/master", vec![OscType::Float(2.0)])),
            Some(Master(1.0))
        );
    }

    #[test]
    fn listener_receives_over_udp() {
        let l = OscListener::start(0).unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let bundle = OscPacket::Bundle(rosc::OscBundle {
            timetag: (0, 1).into(),
            content: vec![
                OscPacket::Message(msg("/rustmap/cue/go", vec![OscType::Int(2)])),
                OscPacket::Message(msg("/hello", vec![])),
            ],
        });
        let bytes = rosc::encoder::encode(&bundle).unwrap();
        tx.send_to(&bytes, ("127.0.0.1", l.port())).unwrap();
        let mut got = Vec::new();
        for _ in 0..50 {
            got.extend(l.drain());
            if got.len() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].command, Some(Command::GoCue(CueRef::Number(2))));
        assert_eq!(got[0].args, "2");
        assert_eq!(got[1].address, "/hello");
        assert!(got[1].command.is_none());
        drop(l); // stops and joins
    }
}

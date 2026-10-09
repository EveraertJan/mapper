//! RUSTMAP LED output: DMX universes over Art-Net and sACN (E1.31).
//!
//! [`DmxSender`] owns a UDP socket on a background thread. The render loop
//! hands it the latest set of universes; if the network is slower than the
//! frame rate, older frames are dropped rather than queued.

pub mod artnet;
pub mod sacn;

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Protocol {
    ArtNet,
    Sacn,
}

/// One universe to send.
#[derive(Debug, Clone, PartialEq)]
pub struct UniverseData {
    pub protocol: Protocol,
    /// `None`: Art-Net broadcast / sACN multicast.
    pub destination: Option<IpAddr>,
    pub universe: u16,
    pub data: Box<[u8; 512]>,
}

/// Parses a destination as typed by the user: empty means broadcast/multicast.
pub fn parse_destination(s: &str) -> Result<Option<IpAddr>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<IpAddr>()
        .map(Some)
        .map_err(|_| format!("\"{s}\" is not an IP address"))
}

/// Counters for the status line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SenderStats {
    pub packets: u64,
    pub frames: u64,
    /// Frames replaced before they were sent (network slower than the frame rate).
    pub dropped: u64,
    pub errors: u64,
    pub last_error: Option<String>,
}

struct Shared {
    pending: Mutex<Option<Vec<UniverseData>>>,
    wake: Condvar,
    stop: AtomicBool,
    packets: AtomicU64,
    frames: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
    last_error: Mutex<Option<String>>,
}

/// Sends universes from a background thread.
pub struct DmxSender {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// Ports to send to; only tests change them.
#[derive(Debug, Clone, Copy)]
pub struct Ports {
    pub artnet: u16,
    pub sacn: u16,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            artnet: artnet::PORT,
            sacn: sacn::PORT,
        }
    }
}

impl DmxSender {
    /// Opens a UDP socket and starts the sender thread. `source_name` shows up
    /// in sACN receivers.
    pub fn new(source_name: &str) -> std::io::Result<Self> {
        Self::with_ports(source_name, Ports::default())
    }

    pub fn with_ports(source_name: &str, ports: Ports) -> std::io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            packets: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_error: Mutex::new(None),
        });
        let worker = Worker {
            socket,
            shared: shared.clone(),
            ports,
            cid: random_cid(),
            source_name: source_name.to_owned(),
            sequences: HashMap::new(),
        };
        let thread = std::thread::Builder::new()
            .name("dmx send".into())
            .spawn(move || worker.run())?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Queues a frame, replacing one that hasn't gone out yet.
    pub fn send(&self, universes: Vec<UniverseData>) {
        let mut pending = lock(&self.shared.pending);
        if pending.replace(universes).is_some() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.wake.notify_one();
    }

    pub fn stats(&self) -> SenderStats {
        let s = &self.shared;
        SenderStats {
            packets: s.packets.load(Ordering::Relaxed),
            frames: s.frames.load(Ordering::Relaxed),
            dropped: s.dropped.load(Ordering::Relaxed),
            errors: s.errors.load(Ordering::Relaxed),
            last_error: lock(&s.last_error).clone(),
        }
    }
}

impl Drop for DmxSender {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A per-run sACN component id (receivers use it to tell sources apart).
fn random_cid() -> [u8; 16] {
    let state = std::collections::hash_map::RandomState::new();
    let mut cid = [0u8; 16];
    for (i, chunk) in cid.chunks_mut(8).enumerate() {
        let mut h = state.build_hasher();
        h.write_usize(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    // RFC 4122 version 4 bits, so it reads as a UUID.
    cid[6] = (cid[6] & 0x0f) | 0x40;
    cid[8] = (cid[8] & 0x3f) | 0x80;
    cid
}

struct Worker {
    socket: UdpSocket,
    shared: Arc<Shared>,
    ports: Ports,
    cid: [u8; 16],
    source_name: String,
    sequences: HashMap<(Protocol, u16), u8>,
}

impl Worker {
    fn run(mut self) {
        loop {
            let frame = {
                let mut pending = lock(&self.shared.pending);
                loop {
                    if self.shared.stop.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some(f) = pending.take() {
                        break f;
                    }
                    pending = self
                        .shared
                        .wake
                        .wait(pending)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            };
            for u in &frame {
                self.send_one(u);
            }
            self.shared.frames.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn send_one(&mut self, u: &UniverseData) {
        let seq = self.sequences.entry((u.protocol, u.universe)).or_insert(0);
        let (packet, addr) = match u.protocol {
            Protocol::ArtNet => {
                // Art-Net sequence runs 1..=255; 0 would disable reordering.
                *seq = if *seq == 255 { 1 } else { *seq + 1 };
                let ip = u.destination.unwrap_or(IpAddr::V4(Ipv4Addr::BROADCAST));
                (
                    artnet::dmx_packet(u.universe, *seq, &u.data[..]),
                    SocketAddr::new(ip, self.ports.artnet),
                )
            }
            Protocol::Sacn => {
                *seq = seq.wrapping_add(1);
                let ip = u
                    .destination
                    .unwrap_or(IpAddr::V4(sacn::multicast_addr(u.universe)));
                (
                    sacn::data_packet(
                        &self.cid,
                        &self.source_name,
                        sacn::DEFAULT_PRIORITY,
                        u.universe,
                        *seq,
                        &u.data[..],
                    ),
                    SocketAddr::new(ip, self.ports.sacn),
                )
            }
        };
        match self.socket.send_to(&packet, addr) {
            Ok(_) => {
                self.shared.packets.fetch_add(1, Ordering::Relaxed);
            }
            Err(err) => {
                let first = self.shared.errors.fetch_add(1, Ordering::Relaxed) == 0;
                let msg = format!("{addr}: {err}");
                if first {
                    tracing::warn!("LED output: {msg}");
                }
                *lock(&self.shared.last_error) = Some(msg);
            }
        }
    }
}

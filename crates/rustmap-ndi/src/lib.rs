//! NDI input and output for RUSTMAP.
//!
//! The NDI runtime is proprietary, so RUSTMAP never links or ships it: it is
//! loaded at run time from wherever NDI Tools / the NDI SDK installed it (or
//! from `RUSTMAP_NDI_LIB`). Without it, [`Ndi::load`] returns an error and the
//! rest of the app works as usual.
//!
//! All NDI calls happen on worker threads:
//! - [`Discovery`] keeps a live list of sources on the network,
//! - [`Receiver`] pulls frames and keeps only the newest (no latency build-up),
//! - [`Sender`] publishes frames, dropping one rather than stalling the renderer.

mod ffi;

use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{
    Receiver as ChanReceiver, SyncSender, TryRecvError, TrySendError, sync_channel,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ffi::NdiLib;

#[derive(Debug, thiserror::Error)]
pub enum NdiError {
    #[error(
        "NDI runtime not found. Install NDI Tools (ndi.video/tools) or set RUSTMAP_NDI_LIB to libndi. Looked in: {}",
        tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    NotFound { tried: Vec<PathBuf> },
    #[error("NDI runtime at {path} could not be loaded: {reason}")]
    Load { path: PathBuf, reason: String },
    #[error("NDI is not supported on this CPU")]
    UnsupportedCpu,
    #[error("NDI could not create {0}")]
    Create(&'static str),
    #[error("invalid NDI name: {0}")]
    BadName(String),
}

/// Owns the loaded runtime; `NDIlib_destroy` runs when the last user is gone.
struct Runtime {
    lib: NdiLib,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: every finder/receiver/sender holds an `Arc<Runtime>` and
        // destroys its instance first, so nothing uses the library anymore.
        unsafe { (self.lib.destroy)() };
    }
}

// SAFETY: the NDI API is documented as thread-safe for library-level calls,
// and each instance handle is only used from the one thread that owns it.
unsafe impl Send for Runtime {}
// SAFETY: see above.
unsafe impl Sync for Runtime {}

/// A loaded, initialized NDI runtime. Cheap to clone.
#[derive(Clone)]
pub struct Ndi {
    rt: Arc<Runtime>,
    version: String,
}

impl std::fmt::Debug for Ndi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ndi")
            .field("version", &self.version)
            .finish()
    }
}

impl Ndi {
    /// Finds and initializes the NDI runtime.
    pub fn load() -> Result<Self, NdiError> {
        let candidates = ffi::candidate_paths();
        let mut last_error = None;
        for path in &candidates {
            // Relative names go through the OS search path; absolute ones must exist.
            if path.is_absolute() && !path.exists() {
                continue;
            }
            // SAFETY: these are the documented NDI runtime locations or an
            // explicit override from the user.
            match unsafe { NdiLib::open(path) } {
                Ok(lib) => return Self::init(lib),
                Err(err) => last_error = Some((path.clone(), err.to_string())),
            }
        }
        match last_error {
            Some((path, reason)) if path.is_absolute() => Err(NdiError::Load { path, reason }),
            _ => Err(NdiError::NotFound { tried: candidates }),
        }
    }

    fn init(lib: NdiLib) -> Result<Self, NdiError> {
        // SAFETY: plain C calls with no arguments.
        if !unsafe { (lib.initialize)() } {
            return Err(NdiError::UnsupportedCpu);
        }
        // SAFETY: NDIlib_version returns a static NUL-terminated string (or null).
        let version = unsafe {
            let p = (lib.version)();
            if p.is_null() {
                "unknown".to_owned()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        tracing::info!(path = %lib.path.display(), %version, "NDI runtime loaded");
        Ok(Self {
            rt: Arc::new(Runtime { lib }),
            version,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Starts watching the network for NDI sources.
    pub fn discover(&self) -> Discovery {
        Discovery::start(self.rt.clone())
    }

    /// Connects to a source by its full NDI name, e.g. `"STUDIO-PC (Resolume)"`.
    pub fn receive(&self, source: &str) -> Result<Receiver, NdiError> {
        Receiver::start(self.rt.clone(), source)
    }

    /// Publishes a new NDI source. NDI prefixes the machine name, so
    /// `"RUSTMAP Output 1"` shows up as `"MY-MAC (RUSTMAP Output 1)"`.
    pub fn send(&self, name: &str) -> Result<Sender, NdiError> {
        Sender::start(self.rt.clone(), name)
    }
}

/// A stop flag shared with a worker thread; the thread exits soon after.
#[derive(Clone, Default)]
struct Stop(Arc<AtomicBool>);

impl Stop {
    fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.to_owned()).spawn(f) {
        tracing::error!("could not start {name} thread: {err}");
    }
}

// ---------------------------------------------------------------------------
// Discovery

/// Live list of NDI sources. Stops when dropped.
pub struct Discovery {
    sources: Arc<Mutex<Vec<String>>>,
    stop: Stop,
}

impl Discovery {
    fn start(rt: Arc<Runtime>) -> Self {
        let sources = Arc::new(Mutex::new(Vec::new()));
        let stop = Stop::default();
        let (out, flag) = (sources.clone(), stop.clone());
        spawn("ndi discovery", move || {
            let create = ffi::FindCreate {
                show_local_sources: true,
                p_groups: std::ptr::null(),
                p_extra_ips: std::ptr::null(),
            };
            // SAFETY: `create` outlives the call; NDI copies what it needs.
            let finder = unsafe { (rt.lib.find_create_v2)(&create) };
            if finder.is_null() {
                tracing::error!("NDI finder could not be created");
                return;
            }
            while !flag.is_set() {
                // SAFETY: `finder` is valid until destroyed below. The source
                // array stays valid until the next call on this finder; we
                // copy the names out immediately.
                let names = unsafe {
                    (rt.lib.find_wait_for_sources)(finder, 500);
                    let mut count = 0u32;
                    let list = (rt.lib.find_get_current_sources)(finder, &mut count);
                    if list.is_null() {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(list, count as usize)
                            .iter()
                            .filter(|s| !s.p_ndi_name.is_null())
                            .map(|s| CStr::from_ptr(s.p_ndi_name).to_string_lossy().into_owned())
                            .collect()
                    }
                };
                if let Ok(mut list) = out.lock() {
                    *list = names;
                }
            }
            // SAFETY: last use of `finder`.
            unsafe { (rt.lib.find_destroy)(finder) };
        });
        Self { sources, stop }
    }

    /// Sources seen on the network right now, sorted by name.
    pub fn sources(&self) -> Vec<String> {
        let mut list = self.sources.lock().map(|l| l.clone()).unwrap_or_default();
        list.sort();
        list
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.set();
    }
}

// ---------------------------------------------------------------------------
// Video frames

/// One RGBA8 frame, rows tightly packed or padded to `stride`.
///
/// Received frames are **premultiplied** (matching RUSTMAP's textures);
/// frames to send are treated as opaque.
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Bytes per row, at least `width * 4`.
    pub stride: u32,
    pub data: Vec<u8>,
    /// Frames per second as advertised by the sender (0 if unknown).
    pub frame_rate: f32,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VideoFrame({}x{}, stride {})",
            self.width, self.height, self.stride
        )
    }
}

/// Copies an NDI frame into tightly packed premultiplied RGBA.
fn convert_frame(frame: &ffi::VideoFrameV2) -> Option<VideoFrame> {
    let (w, h) = (
        usize::try_from(frame.xres).ok()?,
        usize::try_from(frame.yres).ok()?,
    );
    let stride = usize::try_from(frame.line_stride_in_bytes).ok()?;
    if w == 0 || h == 0 || frame.p_data.is_null() || stride < w * 4 {
        return None;
    }
    let (swap_rb, has_alpha) = match frame.fourcc {
        ffi::FOURCC_RGBA => (false, true),
        ffi::FOURCC_RGBX => (false, false),
        ffi::FOURCC_BGRA => (true, true),
        ffi::FOURCC_BGRX => (true, false),
        other => {
            tracing::warn!("unsupported NDI pixel format {other:#x}");
            return None;
        }
    };
    // SAFETY: NDI guarantees `yres` rows of `line_stride_in_bytes` bytes.
    let src = unsafe { std::slice::from_raw_parts(frame.p_data, stride * h) };
    let mut data = Vec::with_capacity(w * h * 4);
    for row in src.chunks_exact(stride) {
        data.extend_from_slice(&row[..w * 4]);
    }
    for px in data.chunks_exact_mut(4) {
        if swap_rb {
            px.swap(0, 2);
        }
        if !has_alpha {
            px[3] = 255;
        } else if px[3] != 255 {
            let a = u16::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
    }
    let frame_rate = if frame.frame_rate_d > 0 {
        frame.frame_rate_n as f32 / frame.frame_rate_d as f32
    } else {
        0.0
    };
    Some(VideoFrame {
        width: w as u32,
        height: h as u32,
        stride: (w * 4) as u32,
        data,
        frame_rate,
    })
}

// ---------------------------------------------------------------------------
// Receiver

/// Connection state of a [`Receiver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverState {
    /// Looking for the source / waiting for the first frame.
    Connecting,
    /// Frames are arriving.
    Live,
    /// Was live, but no frame for a while (source stopped or network trouble).
    NoSignal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReceiverStatus {
    pub state: ReceiverState,
    pub size: Option<[u32; 2]>,
    pub frame_rate: f32,
    pub frames: u64,
}

struct ReceiverShared {
    latest: Mutex<Option<VideoFrame>>,
    status: Mutex<ReceiverStatus>,
}

/// Receives one NDI source on a worker thread. Stops when dropped.
pub struct Receiver {
    shared: Arc<ReceiverShared>,
    stop: Stop,
    source: String,
}

/// After this long without frames a live source counts as lost.
const NO_SIGNAL_AFTER: Duration = Duration::from_secs(2);

impl Receiver {
    fn start(rt: Arc<Runtime>, source: &str) -> Result<Self, NdiError> {
        let c_source = CString::new(source).map_err(|_| NdiError::BadName(source.to_owned()))?;
        let shared = Arc::new(ReceiverShared {
            latest: Mutex::new(None),
            status: Mutex::new(ReceiverStatus {
                state: ReceiverState::Connecting,
                size: None,
                frame_rate: 0.0,
                frames: 0,
            }),
        });
        let stop = Stop::default();
        let (out, flag) = (shared.clone(), stop.clone());
        let name = source.to_owned();
        spawn("ndi receiver", move || {
            let recv_name = c"RUSTMAP";
            let create = ffi::RecvCreateV3 {
                source_to_connect_to: ffi::Source {
                    p_ndi_name: c_source.as_ptr(),
                    p_url_address: std::ptr::null(),
                },
                color_format: ffi::RECV_COLOR_FORMAT_RGBX_RGBA,
                bandwidth: ffi::RECV_BANDWIDTH_HIGHEST,
                allow_video_fields: false,
                p_ndi_recv_name: recv_name.as_ptr(),
            };
            // SAFETY: all pointers in `create` outlive the call.
            let recv = unsafe { (rt.lib.recv_create_v3)(&create) };
            if recv.is_null() {
                tracing::error!(source = %name, "NDI receiver could not be created");
                return;
            }
            let mut last_frame: Option<Instant> = None;
            while !flag.is_set() {
                let mut frame = ffi::VideoFrameV2::default();
                // SAFETY: `recv` is valid; audio/metadata pointers may be null
                // to skip those streams.
                let kind = unsafe {
                    (rt.lib.recv_capture_v2)(
                        recv,
                        &mut frame,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        100,
                    )
                };
                match kind {
                    ffi::FRAME_TYPE_VIDEO => {
                        let converted = convert_frame(&frame);
                        // SAFETY: frame came from this receiver's capture call.
                        unsafe { (rt.lib.recv_free_video_v2)(recv, &frame) };
                        if let Some(f) = converted {
                            last_frame = Some(Instant::now());
                            if let Ok(mut s) = out.status.lock() {
                                s.state = ReceiverState::Live;
                                s.size = Some([f.width, f.height]);
                                s.frame_rate = f.frame_rate;
                                s.frames += 1;
                            }
                            if let Ok(mut latest) = out.latest.lock() {
                                *latest = Some(f);
                            }
                        }
                    }
                    ffi::FRAME_TYPE_ERROR => {
                        tracing::warn!(source = %name, "NDI connection lost");
                        last_frame = None;
                        if let Ok(mut s) = out.status.lock() {
                            s.state = ReceiverState::NoSignal;
                        }
                    }
                    _ => {}
                }
                if last_frame.is_some_and(|t| t.elapsed() > NO_SIGNAL_AFTER)
                    && let Ok(mut s) = out.status.lock()
                {
                    s.state = ReceiverState::NoSignal;
                }
            }
            // SAFETY: last use of `recv`.
            unsafe { (rt.lib.recv_destroy)(recv) };
        });
        Ok(Self {
            shared,
            stop,
            source: source.to_owned(),
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// The newest frame since the last call, if any. Older frames that were
    /// never picked up are simply dropped.
    pub fn take_frame(&self) -> Option<VideoFrame> {
        self.shared.latest.lock().ok()?.take()
    }

    pub fn status(&self) -> ReceiverStatus {
        *self.shared.status.lock().expect("status lock")
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.set();
    }
}

// ---------------------------------------------------------------------------
// Sender

/// Publishes frames as an NDI source from a worker thread. Stops when dropped.
pub struct Sender {
    frames: Option<SyncSender<VideoFrame>>,
    recycled: ChanReceiver<Vec<u8>>,
    name: String,
    sent: Arc<AtomicU64>,
    dropped: AtomicU64,
}

impl Sender {
    fn start(rt: Arc<Runtime>, name: &str) -> Result<Self, NdiError> {
        let c_name = CString::new(name).map_err(|_| NdiError::BadName(name.to_owned()))?;
        let create = ffi::SendCreate {
            p_ndi_name: c_name.as_ptr(),
            p_groups: std::ptr::null(),
            // RUSTMAP's render loop already paces frames.
            clock_video: false,
            clock_audio: false,
        };
        // SAFETY: `create` and `c_name` outlive the call.
        let send = unsafe { (rt.lib.send_create)(&create) };
        if send.is_null() {
            return Err(NdiError::Create("a sender"));
        }
        let handle = SendHandle(send);

        // One frame in flight: if NDI is still busy with the last one, the
        // newest frame is dropped instead of queueing latency.
        let (tx, rx) = sync_channel::<VideoFrame>(1);
        let (recycle_tx, recycled) = sync_channel::<Vec<u8>>(2);
        let sent = Arc::new(AtomicU64::new(0));
        let sent_count = sent.clone();
        spawn("ndi sender", move || {
            let handle = handle;
            while let Ok(frame) = rx.recv() {
                let (n, d) = frame_rate_fraction(frame.frame_rate);
                let video = ffi::VideoFrameV2 {
                    xres: frame.width as i32,
                    yres: frame.height as i32,
                    fourcc: ffi::FOURCC_RGBX,
                    frame_rate_n: n,
                    frame_rate_d: d,
                    picture_aspect_ratio: frame.width as f32 / frame.height.max(1) as f32,
                    p_data: frame.data.as_ptr().cast_mut(),
                    line_stride_in_bytes: frame.stride as i32,
                    ..Default::default()
                };
                // SAFETY: synchronous send: NDI is done with `p_data` when it returns.
                unsafe { (rt.lib.send_send_video_v2)(handle.0, &video) };
                sent_count.fetch_add(1, Ordering::Relaxed);
                let _ = recycle_tx.try_send(frame.data);
            }
            // SAFETY: last use of the sender; the channel closed because the
            // owning `Sender` was dropped.
            unsafe { (rt.lib.send_destroy)(handle.0) };
        });
        Ok(Self {
            frames: Some(tx),
            recycled,
            name: name.to_owned(),
            sent,
            dropped: AtomicU64::new(0),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// A buffer to fill for the next frame (reused when possible).
    pub fn buffer(&self) -> Vec<u8> {
        match self.recycled.try_recv() {
            Ok(mut v) => {
                v.clear();
                v
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Vec::new(),
        }
    }

    /// Queues a frame. Returns `false` (and drops it) if the previous frame
    /// is still being sent.
    pub fn send(&self, frame: VideoFrame) -> bool {
        let Some(tx) = &self.frames else { return false };
        match tx.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn frames_sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }

    pub fn frames_dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // Closing the channel ends the worker, which destroys the NDI sender.
        self.frames = None;
    }
}

struct SendHandle(ffi::SendInstance);
// SAFETY: the handle moves to the worker thread and is only used there.
unsafe impl Send for SendHandle {}

/// NDI wants frame rates as fractions; use the broadcast ones where they fit.
fn frame_rate_fraction(fps: f32) -> (i32, i32) {
    const KNOWN: [(f32, i32, i32); 8] = [
        (23.976, 24000, 1001),
        (24.0, 24, 1),
        (25.0, 25, 1),
        (29.97, 30000, 1001),
        (30.0, 30, 1),
        (50.0, 50, 1),
        (59.94, 60000, 1001),
        (60.0, 60, 1),
    ];
    if fps <= 0.0 || !fps.is_finite() {
        return (60, 1);
    }
    KNOWN
        .iter()
        .find(|(f, _, _)| (fps - f).abs() < 0.01)
        .map_or(((fps * 1000.0).round() as i32, 1000), |(_, n, d)| (*n, *d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_matches_ndi_header() {
        // Values from NDI SDK 6.1 Processing.NDI.structs.h.
        assert_eq!(ffi::FOURCC_RGBA, 1_094_862_674);
        assert_eq!(ffi::FOURCC_RGBX, 1_480_738_642);
    }

    #[test]
    fn struct_layouts_match_ndi_header() {
        // 64-bit layouts of the C structs.
        assert_eq!(std::mem::size_of::<ffi::Source>(), 16);
        assert_eq!(std::mem::size_of::<ffi::FindCreate>(), 24);
        assert_eq!(std::mem::size_of::<ffi::RecvCreateV3>(), 40);
        assert_eq!(std::mem::size_of::<ffi::SendCreate>(), 24);
        assert_eq!(std::mem::size_of::<ffi::VideoFrameV2>(), 72);
        assert_eq!(std::mem::offset_of!(ffi::VideoFrameV2, timecode), 32);
        assert_eq!(std::mem::offset_of!(ffi::VideoFrameV2, p_data), 40);
        assert_eq!(
            std::mem::offset_of!(ffi::VideoFrameV2, line_stride_in_bytes),
            48
        );
        assert_eq!(std::mem::offset_of!(ffi::VideoFrameV2, p_metadata), 56);
    }

    #[test]
    fn frame_rates() {
        assert_eq!(frame_rate_fraction(59.94), (60000, 1001));
        assert_eq!(frame_rate_fraction(60.0), (60, 1));
        assert_eq!(frame_rate_fraction(0.0), (60, 1));
        assert_eq!(frame_rate_fraction(48.0), (48000, 1000));
    }

    #[test]
    fn converts_bgra_with_stride_to_premultiplied_rgba() {
        // 2x1 BGRA frame, padded to 12 bytes per row.
        let mut bytes = vec![
            10, 20, 200, 255, // B G R A: opaque
            0, 0, 200, 128, // half-transparent red
            9, 9, 9, 9, // padding
        ];
        let frame = ffi::VideoFrameV2 {
            xres: 2,
            yres: 1,
            fourcc: ffi::FOURCC_BGRA,
            frame_rate_n: 30000,
            frame_rate_d: 1001,
            p_data: bytes.as_mut_ptr(),
            line_stride_in_bytes: 12,
            ..Default::default()
        };
        let out = convert_frame(&frame).unwrap();
        assert_eq!(out.stride, 8);
        assert_eq!(out.data, vec![200, 20, 10, 255, 100, 0, 0, 128]);
        assert!((out.frame_rate - 29.97).abs() < 0.01);
    }

    #[test]
    fn finds_libraries_bundled_in_apps() {
        let root = std::env::temp_dir().join(format!("rustmap-apps-{}", std::process::id()));
        let lib = |app: &str, name: &str| {
            let dir = root.join(app).join("Contents/Frameworks");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), b"").unwrap();
            dir.join(name)
        };
        let other = lib("TouchDesigner.app", "libndi.dylib");
        let advanced = lib("NDI Video Monitor.app", "libndi_advanced.dylib");
        let plain = lib("NDI Scan Converter.app", "libndi.dylib");
        std::fs::create_dir_all(root.join("Notes.app")).unwrap();
        assert_eq!(ffi::bundled_in_apps(&root), vec![plain, advanced, other]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_runtime_is_a_clear_error() {
        // SAFETY: (test) only touches this process's environment before any NDI use.
        unsafe { std::env::set_var("RUSTMAP_NDI_LIB", "/nonexistent/libndi.so") };
        if let Err(err) = Ndi::load() {
            assert!(err.to_string().contains("NDI"), "{err}");
        }
        // SAFETY: (test) as above.
        unsafe { std::env::remove_var("RUSTMAP_NDI_LIB") };
    }
}

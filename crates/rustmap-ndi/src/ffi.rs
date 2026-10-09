//! Minimal hand-written bindings for the NDI 6 C API, resolved at run time.
//!
//! Layouts and constants follow `Processing.NDI.*.h` from NDI SDK 6.1. Only
//! the functions RUSTMAP uses are loaded. Every symbol here exists unchanged
//! since NDI 5, so both NDI 5 and 6 runtimes work.

use std::ffi::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use libloading::Library;

pub type FindInstance = *mut c_void;
pub type RecvInstance = *mut c_void;
pub type SendInstance = *mut c_void;

/// `NDIlib_source_t`. The second field is a union of two `const char*`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Source {
    pub p_ndi_name: *const c_char,
    pub p_url_address: *const c_char,
}

/// `NDIlib_find_create_t`
#[repr(C)]
pub struct FindCreate {
    pub show_local_sources: bool,
    pub p_groups: *const c_char,
    pub p_extra_ips: *const c_char,
}

/// `NDIlib_recv_create_v3_t`
#[repr(C)]
pub struct RecvCreateV3 {
    pub source_to_connect_to: Source,
    pub color_format: u32,
    pub bandwidth: i32,
    pub allow_video_fields: bool,
    pub p_ndi_recv_name: *const c_char,
}

/// `NDIlib_send_create_t`
#[repr(C)]
pub struct SendCreate {
    pub p_ndi_name: *const c_char,
    pub p_groups: *const c_char,
    pub clock_video: bool,
    pub clock_audio: bool,
}

/// `NDIlib_video_frame_v2_t`. `line_stride_in_bytes` is a union with
/// `data_size_in_bytes` (same type), used for uncompressed formats.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VideoFrameV2 {
    pub xres: c_int,
    pub yres: c_int,
    pub fourcc: u32,
    pub frame_rate_n: c_int,
    pub frame_rate_d: c_int,
    pub picture_aspect_ratio: f32,
    pub frame_format_type: u32,
    pub timecode: i64,
    pub p_data: *mut u8,
    pub line_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

impl Default for VideoFrameV2 {
    fn default() -> Self {
        Self {
            xres: 0,
            yres: 0,
            fourcc: 0,
            frame_rate_n: 0,
            frame_rate_d: 0,
            picture_aspect_ratio: 0.0,
            frame_format_type: FRAME_FORMAT_PROGRESSIVE,
            timecode: SEND_TIMECODE_SYNTHESIZE,
            p_data: std::ptr::null_mut(),
            line_stride_in_bytes: 0,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}

pub const FOURCC_RGBA: u32 = fourcc(b"RGBA");
pub const FOURCC_RGBX: u32 = fourcc(b"RGBX");
pub const FOURCC_BGRA: u32 = fourcc(b"BGRA");
pub const FOURCC_BGRX: u32 = fourcc(b"BGRX");

pub const FRAME_TYPE_VIDEO: u32 = 1;
pub const FRAME_TYPE_ERROR: u32 = 4;
pub const FRAME_FORMAT_PROGRESSIVE: u32 = 1;
pub const RECV_COLOR_FORMAT_RGBX_RGBA: u32 = 2;
pub const RECV_BANDWIDTH_HIGHEST: i32 = 100;
pub const SEND_TIMECODE_SYNTHESIZE: i64 = i64::MAX;

/// Function table resolved from the NDI runtime library.
pub struct NdiLib {
    pub initialize: unsafe extern "C" fn() -> bool,
    pub destroy: unsafe extern "C" fn(),
    pub version: unsafe extern "C" fn() -> *const c_char,
    pub find_create_v2: unsafe extern "C" fn(*const FindCreate) -> FindInstance,
    pub find_destroy: unsafe extern "C" fn(FindInstance),
    pub find_wait_for_sources: unsafe extern "C" fn(FindInstance, u32) -> bool,
    pub find_get_current_sources: unsafe extern "C" fn(FindInstance, *mut u32) -> *const Source,
    pub recv_create_v3: unsafe extern "C" fn(*const RecvCreateV3) -> RecvInstance,
    pub recv_destroy: unsafe extern "C" fn(RecvInstance),
    pub recv_capture_v2:
        unsafe extern "C" fn(RecvInstance, *mut VideoFrameV2, *mut c_void, *mut c_void, u32) -> u32,
    pub recv_free_video_v2: unsafe extern "C" fn(RecvInstance, *const VideoFrameV2),
    pub send_create: unsafe extern "C" fn(*const SendCreate) -> SendInstance,
    pub send_destroy: unsafe extern "C" fn(SendInstance),
    pub send_send_video_v2: unsafe extern "C" fn(SendInstance, *const VideoFrameV2),
    /// Keeps the function pointers above valid.
    _library: Library,
    pub path: PathBuf,
}

impl NdiLib {
    /// Loads the library at `path` and resolves every function we need.
    ///
    /// # Safety
    /// `path` must be an NDI runtime (or a faithful stand-in); loading runs
    /// its initializers and the symbols are trusted to have the NDI signatures.
    pub unsafe fn open(path: &Path) -> Result<Self, libloading::Error> {
        // SAFETY: the caller vouches for the library (see above).
        let library = unsafe { Library::new(path)? };
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the type comes from the NDI header for this symbol;
                // the pointer stays valid while `_library` is alive.
                unsafe { *library.get(concat!($name, "\0").as_bytes())? }
            };
        }
        Ok(Self {
            initialize: sym!("NDIlib_initialize"),
            destroy: sym!("NDIlib_destroy"),
            version: sym!("NDIlib_version"),
            find_create_v2: sym!("NDIlib_find_create_v2"),
            find_destroy: sym!("NDIlib_find_destroy"),
            find_wait_for_sources: sym!("NDIlib_find_wait_for_sources"),
            find_get_current_sources: sym!("NDIlib_find_get_current_sources"),
            recv_create_v3: sym!("NDIlib_recv_create_v3"),
            recv_destroy: sym!("NDIlib_recv_destroy"),
            recv_capture_v2: sym!("NDIlib_recv_capture_v2"),
            recv_free_video_v2: sym!("NDIlib_recv_free_video_v2"),
            send_create: sym!("NDIlib_send_create"),
            send_destroy: sym!("NDIlib_send_destroy"),
            send_send_video_v2: sym!("NDIlib_send_send_video_v2"),
            _library: library,
            path: path.to_owned(),
        })
    }
}

/// Where NDI runtimes are installed, most specific first.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(p) = std::env::var_os("RUSTMAP_NDI_LIB") {
        paths.push(PathBuf::from(p));
    }
    let lib_name = if cfg!(target_os = "macos") {
        "libndi.dylib"
    } else if cfg!(target_os = "windows") {
        "Processing.NDI.Lib.x64.dll"
    } else {
        "libndi.so.6"
    };
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
        if let Some(dir) = std::env::var_os(var) {
            paths.push(PathBuf::from(dir).join(lib_name));
        }
    }
    if cfg!(target_os = "macos") {
        paths.extend(
            [
                "/usr/local/lib/libndi.dylib",
                "/Library/NDI SDK for Apple/lib/macOS/libndi.dylib",
                "/Library/NDI SDK for Apple/lib/macOS/libndi_advanced.dylib",
                "/opt/homebrew/lib/libndi.dylib",
            ]
            .map(PathBuf::from),
        );
        paths.extend(bundled_in_apps(Path::new("/Applications")));
    } else if cfg!(target_os = "windows") {
        paths.push(PathBuf::from(lib_name));
    } else {
        paths.extend(
            [
                "libndi.so.6",
                "libndi.so.5",
                "libndi.so",
                "/usr/local/lib/libndi.so.6",
                "/usr/lib/libndi.so.6",
            ]
            .map(PathBuf::from),
        );
    }
    paths
}

/// On macOS, NDI Tools installs no shared runtime: every NDI app carries its
/// own `libndi` in `Contents/Frameworks`. Borrow one, preferring NDI's own apps
/// and the plain library over the "advanced" one (same API for our use).
pub fn bundled_in_apps(applications: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(applications) else {
        return Vec::new();
    };
    let mut apps: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .collect();
    apps.sort();
    let is_ndi_app = |p: &PathBuf| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("NDI"))
    };
    let mut found = Vec::new();
    for (ndi_apps_only, lib) in [
        (true, "libndi.dylib"),
        (true, "libndi_advanced.dylib"),
        (false, "libndi.dylib"),
    ] {
        for app in apps.iter().filter(|a| !ndi_apps_only || is_ndi_app(a)) {
            let candidate = app.join("Contents/Frameworks").join(lib);
            if candidate.is_file() && !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

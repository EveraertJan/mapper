//! End-to-end test against `mock_ndi.c`, a stand-in NDI runtime.
//!
//! Exercises loading, discovery, receiving (stride, premultiply, newest-frame
//! semantics) and sending through the same code paths the real runtime uses.
//! Skipped if no C compiler is available.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rustmap_ndi::{Ndi, ReceiverState, VideoFrame};

fn build_mock() -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mock_ndi.c");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(target_os = "macos") {
        "libndi_mock.dylib"
    } else {
        "libndi_mock.so"
    });
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-O2", "-o"])
        .arg(&out)
        .arg(&src)
        .status();
    match status {
        Ok(s) if s.success() => Some(out),
        other => {
            eprintln!("skipping NDI mock test: could not compile mock ({other:?})");
            None
        }
    }
}

fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn receive_and_send_through_mock_runtime() {
    let Some(lib_path) = build_mock() else { return };
    // SAFETY: (test) set before any thread reads the environment.
    unsafe { std::env::set_var("RUSTMAP_NDI_LIB", &lib_path) };

    let ndi = Ndi::load().expect("load mock runtime");
    assert_eq!(ndi.version(), "MOCK NDI 6.1");

    // SAFETY: (test) the mock exports these introspection helpers.
    let probe = unsafe { libloading::Library::new(&lib_path).unwrap() };
    let counter = |which: i32| -> i32 {
        // SAFETY: (test) signature as defined in mock_ndi.c.
        unsafe {
            probe
                .get::<unsafe extern "C" fn(i32) -> i32>(b"mock_counter\0")
                .unwrap()(which)
        }
    };
    let last_send = |i: i32| -> i32 {
        // SAFETY: (test) signature as defined in mock_ndi.c.
        unsafe {
            probe
                .get::<unsafe extern "C" fn(i32) -> i32>(b"mock_last_send\0")
                .unwrap()(i)
        }
    };

    // Discovery.
    let discovery = ndi.discover();
    let sources = wait_for(Duration::from_secs(5), || {
        let s = discovery.sources();
        (!s.is_empty()).then_some(s)
    })
    .expect("mock source discovered");
    assert_eq!(sources, vec!["MOCK (Test Pattern)".to_owned()]);

    // Receiving.
    let receiver = ndi.receive(&sources[0]).expect("receiver");
    let frame = wait_for(Duration::from_secs(5), || receiver.take_frame()).expect("a frame");
    assert_eq!((frame.width, frame.height, frame.stride), (64, 32, 256));
    assert_eq!(frame.data.len(), 64 * 32 * 4, "padding must be stripped");
    // Pixel (3, 2), opaque half: R = x*4, G = y*8, A = 255.
    let px = |x: usize, y: usize| &frame.data[(y * 64 + x) * 4..][..4];
    assert_eq!(&px(3, 2)[..2], &[12, 16]);
    assert_eq!(px(3, 2)[3], 255);
    // Pixel (40, 2), alpha 128: premultiplied R = 160 * 128 / 255 ≈ 80.
    assert_eq!(px(40, 2)[3], 128);
    assert_eq!(px(40, 2)[0], 80);
    assert!((frame.frame_rate - 59.94).abs() < 0.01);

    let status = wait_for(Duration::from_secs(2), || {
        let s = receiver.status();
        (s.frames >= 3).then_some(s)
    })
    .expect("frames keep coming");
    assert_eq!(status.state, ReceiverState::Live);
    assert_eq!(status.size, Some([64, 32]));

    // Unknown sources connect but never go live.
    let ghost = ndi.receive("NOWHERE (Nothing)").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(ghost.status().state, ReceiverState::Connecting);
    assert!(ghost.take_frame().is_none());

    // Sending.
    let sender = ndi.send("RUSTMAP Test").expect("sender");
    let mut data = sender.buffer();
    data.resize(256 * 4 * 2, 0); // 2 rows of a 200-wide frame, padded to 1024-byte rows
    data[..4].copy_from_slice(&[1, 2, 3, 255]);
    let mut accepted = 0;
    for _ in 0..20 {
        if sender.send(VideoFrame {
            width: 200,
            height: 2,
            stride: 1024,
            data: data.clone(),
            frame_rate: 60.0,
        }) {
            accepted += 1;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(accepted > 0);
    wait_for(Duration::from_secs(2), || {
        (sender.frames_sent() as i32 == accepted).then_some(())
    })
    .expect("all accepted frames sent");
    assert_eq!(sender.frames_sent() + sender.frames_dropped(), 20);
    assert_eq!(
        (last_send(0), last_send(1), last_send(2)),
        (200, 2, 1024),
        "size and stride reach NDI"
    );
    assert_eq!(last_send(3), 1_480_738_642, "sent as RGBX");
    assert_eq!(last_send(4), i32::from_le_bytes([1, 2, 3, 255]));

    // Teardown: every NDI instance is destroyed and every frame freed.
    drop((receiver, ghost, sender, discovery));
    wait_for(Duration::from_secs(3), || {
        (counter(1) == counter(0) && counter(4) == counter(3)).then_some(())
    })
    .expect("receivers and senders destroyed");
    assert_eq!(counter(0), 2);
    assert_eq!(counter(3), 1);
    assert!(counter(2) >= 3, "captured frames are freed");
}

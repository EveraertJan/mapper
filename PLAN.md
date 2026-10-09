# RUSTMAP — Project Plan

A native, GPU-first projection-mapping and LED-mapping tool written in Rust, as an open alternative to MadMapper.

---

## 1. Goals and non-goals

**Goals (v1.0)**
- Map video, images and live NDI feeds onto projection surfaces (quads, triangles, circles, mesh warps) on one or more projector outputs.
- Feel like a show tool: stable 60 fps, low latency, survives a 4-hour gig, can't crash because a file went missing.
- Two-view workflow like MadMapper: an **Input view** (which part of the media goes where) and an **Output view** (where it lands on the projector).
- LED pixel mapping over Art-Net / sACN.
- Plain, human-readable project files that diff cleanly in git, plus production files that open straight into presentation.

**Non-goals (for now)**
- 3D object mapping with calibrated camera/projector pose (MadMapper's "3D" / spatial scanner). Revisit after v1.
- A full timeline/NLE. Cues and simple sequences only.
- Laser (ILDA) output. Possible plugin later.
- Windows/Linux-first polish. Design for cross-platform, ship macOS first.

---

## 2. Tech stack

| Concern | Choice | Why |
|---|---|---|
| GPU | **wgpu** | Metal on macOS, Vulkan/DX12 elsewhere; one API, WGSL shaders, raw Metal handle access when needed (Syphon). |
| Windowing | **winit** | Multiple windows (control + one per projector), fullscreen on a chosen monitor. |
| UI | **egui** (+ egui-wgpu, egui-winit) | Immediate mode, fast to iterate, renders on the same wgpu device. Custom-painted canvas for the mapping views. |
| Math | **glam** | Fast, `Vec2/Mat3/Mat4`, plays nicely with bytemuck. |
| Video decode | **ffmpeg-next** (FFmpeg, dynamically linked, LGPL) first; native **AVFoundation/VideoToolbox** path later for hardware decode | FFmpeg reads everything; VT gives zero-copy H.264/HEVC/ProRes on Mac. |
| HAP codec | Own decoder: demux via FFmpeg, Snappy decompress (**snap**), upload as BC1/BC3/BC7 compressed textures | HAP is the VJ standard; GPU does the "decode". Apple Silicon exposes BC texture compression in wgpu. |
| Images | **image** crate | PNG/JPG/TIFF/etc. |
| Cameras | **nokhwa** | Cross-platform webcam / capture cards. |
| Texture sharing | **Syphon** (macOS, via objc2 bindings to the Metal Syphon server/client), **Spout** (Windows, later), **NDI** (runtime-loaded `libndi`, own crate `rustmap-ndi`, M2) | Interop with Resolume, TouchDesigner, VDMX, etc. |
| Generators (dropped for now) | **ISF** (Interactive Shader Format) loader → GLSL → WGSL via **naga** | MadMapper materials are ISF; reuses a huge existing shader library. |
| Audio | **cpal** + **rustfft** | Input capture and FFT bands for audio reactivity. |
| MIDI (dropped for now) | **midir** | MIDI learn, clock. |
| OSC | **rosc** | Remote control, TouchOSC, show control. |
| DMX | Own Art-Net + sACN (E1.31) senders over UDP (std/socket2) | Small protocols; avoid heavy deps. |
| Serialization | **serde** + **ron** (project files) | Readable, supports enums well, diffable. |
| IDs | **slotmap** or `uuid` | Stable handles for surfaces/media across undo and save/load. |
| Errors/logging | **anyhow** (app), **thiserror** (libs), **tracing** | Standard. |
| Profiling | **puffin** (+ egui viewer) | Frame-time visibility from day one. |

Verify crate versions when scaffolding; the wgpu/winit/egui trio must be on matching releases.

---

## 3. Architecture

### 3.1 Workspace layout

```
rustmap/
├─ Cargo.toml                 # workspace
├─ crates/
│  ├─ rustmap-core/           # scene model, commands, undo, no GPU, no UI
│  ├─ rustmap-geom/           # homography, mesh warp, bezier masks, hit-testing
│  ├─ rustmap-render/         # wgpu pipelines, compositor, output windows
│  ├─ rustmap-media/          # decoders, frame queues, texture upload
│  ├─ rustmap-shaders/        # ISF parsing, GLSL→WGSL, built-in generators
│  ├─ rustmap-io/             # MIDI, OSC, audio analysis, Art-Net/sACN
│  ├─ rustmap-interop/        # Syphon / Spout / NDI (feature-gated per OS)
│  └─ rustmap-ui/             # egui panels, mapping canvas, inspectors
├─ app/                       # binary: wires everything, event loop
├─ assets/                    # built-in shaders, test media, icons
└─ docs/
```

Rule: `core` and `geom` stay pure (no wgpu, no egui), so they are fully unit-testable and the scene can be driven headless (tests, future CLI/server mode).

### 3.2 Scene model (rustmap-core)

```
Project
 ├─ Media[]          (Video | Image | Camera | Syphon | Shader(ISF) | Color | Group)
 ├─ Surfaces[]       (Quad | Triangle | Circle | Mesh | Line | Mask)
 │    ├─ input region   (polygon/quad in media UV space)   ← Input view
 │    ├─ output shape   (corners / mesh in output space)   ← Output view
 │    ├─ media_id, opacity, blend mode, color adjust, z-order
 │    └─ masks[], feather
 ├─ Outputs[]        (projector: resolution, monitor, edge blend, test pattern)
 ├─ Fixtures[]       (LED: pixel layout, universe/start address, protocol)
 ├─ Cues[]           (snapshot of parameter values + transition time)
 └─ Mappings[]       (MIDI/OSC/audio → parameter path)
```

Every animatable value is a `Param<T>` addressable by a path (`surfaces/<id>/opacity`). That one design decision makes MIDI learn, OSC, cues, and audio reactivity all the same mechanism.

### 3.3 Edits, undo, threads

- Undo/redo uses project snapshots taken when an edit settles (see M3) instead of a command per edit: projects are small, and it covers every kind of edit for free.
- **Main thread**: winit event loop, egui, applying commands, rendering all windows from one wgpu `Device`.
- **Decode threads**: one per active video; push decoded frames into a small bounded ring (2–3 frames). The render loop picks the frame whose PTS matches the media clock, uploads it, and never blocks.
- **IO threads**: MIDI/OSC/audio listeners send messages over a channel to the main thread; DMX sender runs on its own fixed-rate (≈40 Hz) thread with a snapshot of pixel data.

### 3.4 Render pipeline (per frame)

1. Update media: poll decoders, upload new frames to textures (reuse textures; YUV→RGB in a shader, not on CPU).
2. Evaluate generators (ISF shaders render into offscreen textures at their own resolution).
3. For each **output**: render surfaces back-to-front into an offscreen target at projector resolution:
   - Quad: homography pass (see below), sample media at the input region, apply color adjust + mask + feather.
   - Mesh: draw subdivided grid with per-vertex UVs (bezier/bilinear control points evaluated on CPU, cached).
   - Blend modes via separate pipelines (normal, add, multiply, screen).
4. Post: edge-blend ramps, output gamma, optional test grid overlay.
5. Present to the projector window surface. UI canvas reuses the same output texture as a preview (no double rendering).
6. LED fixtures: sample pixel positions from the output texture → small readback buffer → DMX thread (async `map_async`, one frame of latency is fine).

**Perspective-correct quads**: compute the 3×3 homography from the 4 output corners to the unit square on the CPU (`geom`), pass to the vertex shader, and interpolate UVs as homogeneous `vec3` so the fragment shader divides by `w`. That avoids the classic "bent diagonal" artifact of splitting a quad into two affine triangles.

---

## 4. Milestones

Each milestone ends with something usable. Rough effort assumes one experienced developer part-time; double it if Rust/wgpu are new.

### M0 — Skeleton (1–2 weeks) ✅ done 2026-10-08
- Cargo workspace, CI (fmt, clippy, test) on macOS.
- winit + wgpu + egui control window; second window that can go fullscreen on a chosen monitor.
- puffin profiler panel, tracing logs.
- **Done when**: you can drag a window to the projector, make it fullscreen, and see a test grid at 60 fps.
- Shipped: control window with live previews, per-output render targets, monitor picker, macOS simple fullscreen, Grid / Color bars / Solid test patterns, frame-time graph, VSync toggle, puffin over HTTP (puffin_egui lags egui 0.36), headless GPU render tests.

### M1 — First mapping (2–3 weeks) ✅ done 2026-10-08
- Image media, one quad surface, draggable corners in the Output view, homography rendering.
- Keyboard nudging (arrow keys, shift = coarse), snapping, corner selection.
- **Done when**: an image is corner-pinned perspective-correctly onto a wall.

### M2 — NDI in and out (1 week) ✅ done 2026-10-08
Moved ahead of video: live NDI feeds (Resolume, TouchDesigner, OBS, cameras) cover most show content without RUSTMAP decoding files itself.
- `rustmap-ndi`: the NDI runtime is loaded at run time (NDI Tools / SDK install paths, or `RUSTMAP_NDI_LIB`), never linked or shipped, so the MIT build stays clean and runs without NDI.
- NDI sources on the network show up under Media → NDI source and go on surfaces like images; one receiver thread per source keeps only the newest frame (no latency build-up); straight alpha is premultiplied on receive.
- Per-output "NDI out": the output image is read back with a three-buffer async ring and published as `MACHINE (RUSTMAP <output name>)`; frames are dropped rather than queued when the network is busy.
- Tested end to end against a stand-in C runtime (`crates/rustmap-ndi/tests/mock_ndi.c`) with the struct layouts of NDI SDK 6.1.
- Not yet: NDI audio, NDI metadata/tally, picking NDI groups, YUV (UYVY) fast path on receive.
- **Done when**: a Resolume NDI output is mapped on a wall, and the mapped result is visible in NDI Studio Monitor.

### M3 — Real projects (1 week) ✅ done 2026-10-08
Moved ahead of video (2026-10-08).
- Show files (`.rustmap`, RON): readable and diffable; media paths stored relative to the show so a show folder can move to another machine; written to a temp file and renamed, so a crash mid-save can't corrupt a show; rejects files from newer versions.
- Undo/redo (Cmd+Z / Cmd+Shift+Z) by snapshots taken when an edit settles, so a whole drag is one step. Live state (open windows, fullscreen, monitor, NDI publishing) is kept out of history: undo never closes a projector mid-show.
- **Production file** (File → Save Production File…, saved next to the working file as `Name.production.rustmap`): opens straight into presentation.
- **Presentation mode** (View → Present, Cmd+T, like MadMapper's Ctrl+T): every output fullscreen on its projector, guides hidden. A mode, not an edit: it never enters the file or undo, and Cmd+T or Esc in an output window ends it.
- Unsaved-changes prompt on quit, New and Open; title shows `•` when unsaved; autosave every minute (`Name.rustmap.autosave`, or the data folder for unsaved shows) with File → Recover autosave.
- Missing media: "Missing · Relink…" on the media item; other missing files with the same names in the chosen folder are relinked too.
- `cargo run -p rustmap-core --example demo_show -- Demo.rustmap [--production] [image]` writes a starter show.
- **Done when**: a show can be saved, closed, reopened on another machine and look identical, and its production file opens fullscreen on the projectors.

### M4 — Video and layering (1 week) ✅ done 2026-10-08
- Video media. Backends behind one `Decoder` trait (open / seek / next frame), sharing the clock, loop and seek logic: **AVFoundation on macOS** (`AVAssetReader`, hardware decoding through VideoToolbox, nothing to install or bundle), **FFmpeg** on Linux/Windows and opt-in on macOS (`--features ffmpeg-macos`) for formats AVFoundation can't open. One decoder thread per video, a few frames ahead; the main thread owns the clock (pause, speed 0–8×, scrub/seek).
- Seamless loops: the decoder rewinds itself at the end and keeps the timeline continuous, so there is no hitch at the loop point.
- Transport per video in the Media list (Play/Pause, back to start, Loop, speed, scrub bar with time). Pause state is operating the show, not editing it, so it stays out of undo; loop and speed are saved, so a production file starts playing as set up.
- Shapes: Quad, Triangle, Circle, as anti-aliased masks on the perspective quad (same four handles, warp correctly). Outlines follow the shape in the editor and in the projector guides.
- Blend modes per surface: Normal, Add, Screen, Multiply (premultiplied-correct).
- Input view (Output | Input tabs, or Surface → Input → Edit region): pick which part of the media a surface shows. Any quad, so it can crop, rotate or un-keystone the source; applied on the GPU as a second homography.
- Builds without FFmpeg with `--no-default-features` (video items then say so).
- Not yet (moved to M7): audio, HAP, YUV→RGB on the GPU (conversion currently runs on the decoder thread), playlists/cues.
- **Done when**: three looping videos play on three surfaces without dropped frames.

### M5 — Warping and blending ✅ done 2026-10-09
- Mesh warp per surface: 2–16 × 2–16 control points, Catmull-Rom (smooth, passes through every point) or bilinear. Turning it on reproduces the perspective quad exactly, so nothing jumps; changing the point count resamples the current shape. Tessellated on the CPU (12 steps per cell) and drawn with the same shader; the outer mesh points stay in sync with the surface's corners, so snapping, Match aspect and old code paths keep working.
- Polygon masks (up to 32 points) in surface space, so they follow any warp: even-odd fill, feather inward, invert. Editor **Mask** layer: drag points or the whole mask, double-click an edge to add, Delete to remove. Polygon rather than bezier: simpler to edit, and enough points cover curves.
- Edge blending per output: left/right/top/bottom ramp widths, gamma and curve (S-curve), applied as a multiply pass over the finished output (so NDI out and previews include it). The CPU formula is tested so overlapping ramps sum to full brightness after gamma.
- Color per surface: brightness, contrast, saturation, hue rotation, RGB gain (on unpremultiplied color in the shader).
- Editor and projector guides draw the warped outline, mesh grid lines and handles, and the mask outline.
- Old show files load unchanged (all new fields default).
- Not done: levels/curves, bezier mask handles, per-point mesh weights, black-level compensation in overlaps (moved to M7 if needed).
- **Done when**: two overlapping projectors produce a seamless image on a curved surface. *(Verified headless and in the UI; needs a real two-projector test.)*

### M6 — LED mapping ✅ done 2026-10-09
- Fixtures on an output, like surfaces: **strip** (first and last pixel, N pixels), **matrix** (a quad of cols × rows, any corner as start, rows or columns, zigzag/serpentine) and **free points** (any fixture converted to one handle per pixel). Edited in the Output view (drag handles or the whole fixture, snapping to the frame and surface corners, arrows/Tab/Esc like surfaces), listed under *LED fixtures*, set up in the inspector.
- Sampling on the GPU: one small pass per output writes one texel per LED (bilinear, or by default a 4 × 4 average over half the pixel spacing so detail doesn't flicker), read back asynchronously like NDI. LEDs see the image before edge blending; off-canvas pixels are black.
- Patch per fixture: Art-Net or sACN, destination IP (empty = Art-Net broadcast / sACN multicast 239.255.x.y), universe, start channel. Pixels never split across universes (170 RGB / 128 RGBW per universe). New fixtures are patched right after the previous one. Overlapping channels are flagged.
- Color orders RGB, RBG, GRB, GBR, BRG, BGR, RGBW, GRBW (white = the gray part) and single-channel W; brightness per fixture.
- New crate `rustmap-dmx`: pure-Rust Art-Net 4 (ArtDmx) and E1.31 packets, one UDP socket on a sender thread, latest frame wins, per-universe sequence numbers. Project-wide *Send to lights* switch and rate (default 40 fps). Sending is saved with the show (so a production file starts sending) but kept out of undo.
- **DMX** tab in the editor: every universe's 512 channels with values; hover names the fixture and pixel driving a channel. Status bar shows universes, packets and problems (bad IP, overlaps, network errors).
- Verified: packet layouts against the specs, real UDP loopback, an independent sACN library (python `sacn`) receiving the app's packets, GPU sampling in headless tests, and the UI under Xvfb.
- Not done: ArtPoll discovery, ArtSync, choosing the network interface, per-universe pixel limits other than "no split", gamma/color temperature per fixture, importing pixel layouts (CSV), live preview colors on the fixture dots.
- **Done when**: an LED strip follows the projected content in sync. *(Needs a test with a real controller.)*

### M6B — DMX fixtures and console ✅ done 2026-10-09
- **Fixture types you define yourself**, stored in the show: a name and a list of channels. Each channel has a label, a kind (dimmer, red/green/blue/white/amber/UV, pan, tilt, strobe/shutter, zoom, focus, color wheel, gobo, speed, other), an optional 16-bit mode (coarse + fine channel), a default value, and **value labels**: named ranges like "Open 8–15" or "Gobo 2 16–23". Channels can be added, removed, renamed and reordered; every fixture of the type follows.
- Starter templates: Dimmer, RGB PAR, RGBW PAR + dimmer, Moving head (example). Types can be **exported** to `.rmfixture` files (default folder: the user's RUSTMAP/Fixtures data folder) and **imported** into other shows.
- **DMX fixtures**: an instance of a type with a name and a patch (Art-Net/sACN, destination, universe, start address), patched automatically after the previous one (next universe when it doesn't fit). Duplicate, enable/disable, overlap and past-512 warnings. They share universes with LED fixtures; overlaps between any lights are flagged.
- **Console** tab: one strip per fixture with a fader per channel (colored by kind; drag, Shift-drag for fine, scroll to step, double-click for the default), the current value label under it with a menu of all labels, **Locate** (intensity and colors full) and **Reset**. **Grand master** and **Blackout** work on LED pixels and on DMX intensity channels (the dimmer when a type has one, otherwise its color channels). Values are saved and undoable; blackout is not.
- DMX monitor hovers name the fixture channel ("Par 2 · Red", "Head 1 · Pan fine").
- Verified: unit tests (16-bit layout, master, type edits, overlaps, file round-trip) and the real app sending to a UDP receiver, with every byte checked after fader moves, Locate, value-label picks and Blackout.
- Not done: groups and multi-selection (one fader for many fixtures), presets/cues, effects (chases), importing GDTF / Open Fixture Library definitions, linking a fixture's color to a point of the image.

### M7 — Performance and polish → v1.0 (3+ weeks)
- HAP / HAP Q / HAP Alpha decoding (BC textures).
- Zero-copy video upload (IOSurface → Metal texture) and YUV→RGB in a shader instead of on the CPU; honour the video's rotation metadata.
- Video audio output (and sync to it).
- Frame pacing per output, vsync options, "performance mode" (hide UI previews).
- Sample projects and docs. No signed/notarized app: built from source with `cargo`.

**Dropped from the roadmap (2026-10-08)**: live sources and generators (ISF shaders, cameras, Syphon) and live control (MIDI, OSC, audio reactivity, cues). NDI covers live input for now.

**Later**: Windows (Spout, DX12), Linux, ISF generators / cameras / Syphon, MIDI / OSC / cues, 3D object mapping with projector calibration, plugin API (WASM or dylib), web remote.

---

## 5. Risks and how to handle them

| Risk | Mitigation |
|---|---|
| Video decode performance / sync is the hardest part | Build the media crate behind a `trait VideoSource` so FFmpeg, HAP and VideoToolbox are swappable. Profile from M4. Make HAP the "recommended" codec for shows. |
| FFmpeg licensing and packaging | Solved on macOS: AVFoundation is the default backend, so nothing is bundled. Where FFmpeg is used, link it dynamically and only ship LGPL builds. |
| Syphon needs raw Metal textures from wgpu | Use wgpu's `as_hal` escape hatch to get the `MTLTexture`; isolate in `rustmap-interop` behind `cfg(target_os = "macos")`. Spike this early (if Syphon comes back on the roadmap). |
| Multi-window presentation stutters | One device, one render pass per output, present all in the same frame; test with 2–3 real displays early. |
| egui gets awkward for the canvas | Paint the mapping canvas yourself (egui `Painter` or a wgpu callback) and use egui only for panels. |
| Scope creep toward "everything MadMapper does" | Ship each milestone; keep the non-goals list honest. |

---

## 6. Testing strategy

- **Unit tests** in `geom` (homography round-trips, mesh evaluation, hit-testing) and `core` (command/undo symmetry, save→load→save equality).
- **Golden image tests**: headless wgpu renders of fixture projects compared against reference PNGs with a tolerance.
- **Soak test**: a script that loads a heavy project and runs for hours logging frame times and memory.
- **Real-world rig**: one cheap projector + an LED strip on an Art-Net node, used every milestone.

---

## 7. First week, concretely

1. `cargo new --vcs git` workspace with `app`, `rustmap-core`, `rustmap-geom`, `rustmap-render`, `rustmap-ui`.
2. Get winit + wgpu + egui showing a control window with a "Open output" button.
3. Second window, monitor picker, fullscreen toggle, test grid shader.
4. In `geom`: `fn homography(src: [Vec2; 4], dst: [Vec2; 4]) -> Mat3` with tests.
5. Draw one textured quad with draggable corners. That's M1 half done.

---

## 8. Reference projects worth reading

- **MapMap** (C++/Qt, open source, discontinued) — scene model ideas.
- **Splash** (C++, open source) — multi-projector calibration and blending.
- **ISF spec + library** — generator format and shaders.
- **nannou** — Rust creative-coding framework with wgpu/winit examples.
- **HAP** codec spec — texture formats and chunking.

---

## 9. Decisions

- **License: MIT** (decided 2026-10-08). Consequences:
  - Only depend on crates with MIT/Apache-2/BSD/Zlib-compatible licenses; add `cargo-deny` to CI to enforce it.
  - FFmpeg must stay an LGPL build, dynamically linked and shipped as separate dylibs (users can swap them). Never enable GPL-only FFmpeg components (x264, x265, …) in the bundled build.
  - NDI SDK is proprietary: keep it an optional, runtime-loaded feature, never bundled by default.
  - Syphon (BSD-2) and Spout (BSD-2) are fine.

## 10. Open decisions

- **Platform order**: plan assumes macOS first. If Windows matters early (many venues run Windows media servers), move Spout/DX12 ahead of M5.
- **Name**: "RUSTMAP" works as a working title; check availability before going public.

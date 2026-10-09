# RUSTMAP

Projection mapping in Rust — an open alternative to MadMapper. See [PLAN.md](PLAN.md) for the roadmap.

**Status: M4 done.** Map images, looping videos and live NDI feeds onto projector outputs with correct perspective (quads, triangles, circles, blend modes, input cropping), publish outputs over NDI, save shows with undo, and open production files straight into presentation.

## Run it

Needs Rust 1.95 or newer (`rustup update stable`). On macOS that's all: video plays through AVFoundation, the system's own decoders (hardware H.264, HEVC and ProRes), so there's nothing else to install or bundle.

On Linux/Windows, video uses FFmpeg's libraries (e.g. `apt install libavformat-dev libswscale-dev pkg-config`). On macOS you can add FFmpeg as a fallback for formats AVFoundation can't open: `brew install ffmpeg pkg-config`, then build with `--features ffmpeg-macos`. Builds without any video: `--no-default-features`.

```sh
cargo run --release
cargo run --release -- photo.png clip.mov    # start with images/videos, each on its own quad
cargo run --release -- MyShow.rustmap        # open a show (a production file starts presenting)
```

Release builds are what you want on a projector; `cargo run` (debug) works too, just slower.

To install it as a `rustmap` command (into `~/.cargo/bin`), install the app crate, not the workspace root:

```sh
cargo install --path app
rustmap MyShow.production.rustmap
```

## Using it

- **Outputs** (left, top): one per projector. Tick the checkbox to open its window.
- **Surfaces** (left): quads on the selected output. Top of the list is drawn on top.
- **Media** (left, bottom): images, videos and NDI sources. `+` adds them; you can also drop files on the window. **Use** puts the media on the selected surface. Videos get Play/Pause, back-to-start, Loop, speed and a scrub bar.
- **Editor** (center): **Output** tab: the selected output's live image; drag a corner, or drag inside a surface to move it. **Input** tab: the selected surface's media; drag the region it should show (crop, rotate, un-keystone).
- **Inspector** (right): surface settings (media, opacity, shape: quad/triangle/circle, blend: normal/add/screen/multiply, input region, exact corner pixels, Match media aspect, mesh warp, mask, color) and output settings (resolution, monitor, fullscreen, NDI out, guides, test pattern, edge blend).

In the editor:

| Input | Action |
|---|---|
| Drag a corner / inside a quad | Move the corner / the whole quad |
| Arrow keys | Nudge the selected corner (or the whole quad) 1 output pixel; Shift: 10 px |
| Tab / Shift-Tab | Next / previous corner |
| Esc | Deselect corner, then surface |
| Alt while dragging | No snapping (corners otherwise snap to output edges, center lines and other quads' corners) |

### Mesh warp, masks, color and edge blending

- **Mesh warp** (Surface → Mesh warp): turns the four corners into a grid of points (2–16 each way) for curved or uneven objects. Drag any point; Tab steps through them. *Smooth curves* bends through the points (Catmull-Rom), off gives straight lines between them. Changing the point count keeps the current shape; *Flatten* undoes the bending but keeps the corners.
- **Mask** (Surface → Mask → Add mask): a polygon cut-out that follows the warp. Switch the editor to **Mask** (next to Output/Input, or *Edit points*): drag points or the whole mask, double-click an edge to add a point (up to 32), Delete/Backspace removes the selected one, Esc goes back to Warp. *Feather* softens the edge, *Invert* keeps the outside instead.
- **Color** (Surface → Color): brightness, contrast, saturation, hue and red/green/blue gain, e.g. to match two projectors.
- **Edge blend** (Output → Edge blend): where two projectors overlap, set the overlap width on the touching sides of both outputs (in pixels). *Gamma* compensates for the projector (raise it if the overlap looks bright); *Curve* 1 is a linear ramp, higher is an S-curve that hides small misalignments. Applied to the whole output, after all surfaces, so NDI out and the preview include it.

In an output window:

| Key | Action |
|---|---|
| `F` | Toggle fullscreen |
| `Esc` | Leave fullscreen |
| `G` | Toggle guides (outlines and handles drawn on the projector) |

On macOS, fullscreen uses "simple" fullscreen (no Spaces animation, no separate desktop), which is what you want for a projector.

### Shows, undo and presenting

| Shortcut (Ctrl on Windows/Linux) | Action |
|---|---|
| Cmd+N / Cmd+O / Cmd+S / Cmd+Shift+S | New, Open, Save, Save As |
| Cmd+Z / Cmd+Shift+Z | Undo / Redo (a whole drag is one step) |
| Cmd+T | Present: every output fullscreen on its projector, guides hidden. Esc in an output window also stops. |

- **Show files** (`.rustmap`) are readable text with media paths relative to the show, so a show folder can be moved or put in git.
- **Production file**: File → Save Production File… writes `Name.production.rustmap` next to your working file. Opening it (File → Open, or `cargo run --release -- Name.production.rustmap`) goes straight to presenting. Keep editing the working file; re-save the production file when you're done.
- **Autosave** every minute while there are unsaved changes; File → Recover autosave brings it back after a crash.
- **Missing media** shows "Missing · Relink…"; relinking one file also finds other missing files with the same names in that folder.

### NDI

RUSTMAP uses NDI if it's installed and works fine without it. Install [NDI Tools](https://ndi.video/tools/) (it includes the runtime), restart RUSTMAP, and the status bar shows `NDI ✔`.

- **NDI in**: Media → NDI source (or `+` in the Media list) lists sources on the network. Picking one puts it on the selected empty surface, or on a new quad.
- **NDI out**: Output → NDI out → Publish sends that output to the network as `YOUR-MAC (RUSTMAP Output 1)`.
- On macOS, NDI Tools has no shared runtime; RUSTMAP borrows `libndi` from the NDI apps in /Applications. Somewhere else? Point `RUSTMAP_NDI_LIB` at a `libndi.dylib`.
- No NDI at hand? A stand-in runtime with one test source:
  ```sh
  cc -shared -fPIC -O2 -o /tmp/libndi_mock.dylib crates/rustmap-ndi/tests/mock_ndi.c
  RUSTMAP_NDI_LIB=/tmp/libndi_mock.dylib cargo run --release
  ```

### LED mapping

LED fixtures sit on an output like surfaces and take their colors from that output's image. An output without a projector window works fine as an LED-only canvas.

1. **LED fixtures → +**: add a **Strip** or a **Matrix**. Drag its handles (strip ends, matrix corners) or the whole fixture in the Output view; arrows nudge, Tab steps through handles. The circled pixel is the first one in the chain.
2. In the inspector: pixel count or cols × rows; for a matrix the **wiring** (start corner, rows or columns, zigzag); **color order** (GRB for WS2812 strips, RGBW…); brightness. *Free points* turns every pixel into its own handle for odd layouts.
3. **Patch**: Art-Net or sACN, the controller's IP (empty: Art-Net broadcast / sACN multicast), universe and start channel. A pixel never splits across universes (170 RGB or 128 RGBW per universe); the inspector shows which universes a fixture uses and warns about overlaps.
4. **Send to lights** under the fixture lists (or in the Console) starts sending (default 40 fps). It's saved with the show, so a production file starts sending on open.

The **DMX** tab next to Output/Input shows every universe's channel values as they go out; hover a channel to see which fixture pixel drives it.

### DMX fixtures and the console

For lights that aren't pixels — PARs, dimmers, moving heads, hazers — define the fixture yourself and drive it from faders.

1. **DMX fixtures → +**: start **From template** (Dimmer, RGB PAR, RGBW PAR + dimmer, Moving head) or **New fixture type…**. The fixture gets the next free address.
2. In the inspector, under **Fixture type**: name the type and list its channels, in the order the fixture's manual gives them. Per channel: a label, a kind (sets the fader color and what master/blackout/Locate do), **16-bit** for coarse + fine pairs like pan and tilt, and a default. **Value labels** name ranges of a channel ("Open 8–15", "Gobo 2 16–23"); they show under the fader and as a pick list.
3. Set the **patch** (protocol, IP, universe, start channel). *Duplicate* adds another fixture of the same type right after.
4. The **Console** tab has a strip of faders per fixture: drag (Shift: fine), scroll to step, double-click for the default. **Locate** puts intensity and colors at full; **Reset** returns to defaults. **Grand master** and **Blackout** cover every light, LED pixels included.
5. **Send to lights** (in the Console or under the fixture lists) sends everything.

*Export type…* saves a fixture type as a `.rmfixture` file (by default in your RUSTMAP/Fixtures data folder); *Import fixture types…* in the + menu loads them into another show.

### Groups

At the top of the **Console** tab: *+ Group*, pick its fixtures from the *Members* menu, and it gets one fader per kind of channel (all dimmers together, all reds together, …). The fader shows the average and moves every member.

### Cues

A cue is a stored look of the whole show: DMX fixture values, every surface (visible, opacity, color, blend mode, which media it shows, and its corners / mesh / mask / input region) and whether each video plays.

- The **cue panel** sits at the bottom of every editor tab (half the Console, a strip under the Output view; drag the divider to resize). Click *CUES* to collapse it to one row: *Record all*, *Record selected*, *Back*, *Current*, *Next* and the live cue stay available.
- *Record all* stores the whole show; *Record selected* stores only the selected DMX fixture or surface. *Update* overwrites a cue with the current state (the same items for a cue recorded from a selection). *Current* applies the current cue again after you moved things away from it.
- Per cue, toggles choose what it applies: **Lights**, **Surfaces** (visibility, opacity, color, blend, media), **Geometry** (off by default, so a cue never undoes a calibration; on, the mapping morphs during the fade) and **Media** (play / pause), with **Restart** to rewind its videos.
- Fades move the real values, so faders, inspectors and the outputs show what is happening. Fading: dimmer, colors, pan, tilt, zoom, focus, surface opacity (showing and hiding fade through it), surface color and geometry. Switching at the start: media, blend mode, shape, wheels, gobos, strobe. Geometry with a different mesh size or mask point count switches at once. Touch a fader or a surface mid-fade and it stays where you put it. A whole fade is one undo step.

### OSC remote

Tick **OSC** in the Cues panel (port 8000 by default) and send from TouchOSC, QLab, Bitfocus Companion, a lighting desk or anything else that speaks OSC over UDP:

| Address | Arguments | Does |
|---|---|---|
| `/rustmap/cue/go` | cue number or name | go to that cue |
| `/rustmap/cue/go/<n>` | none, or 1 | go to cue *n* (for buttons) |
| `/rustmap/cue/next` or `/rustmap/go` | none, or 1 | next cue |
| `/rustmap/cue/back` | none, or 1 | previous cue |
| `/rustmap/cue/current` | none, or 1 | apply the current cue again |
| `/rustmap/blackout` | 0 / 1, none toggles | blackout |
| `/rustmap/master` | 0..1 | grand master |

Cue numbers are the numbers in the cue list (counting from 1). A trigger with argument 0 is a button release and is ignored, so TouchOSC buttons fire once. The `/rustmap` prefix is optional and addresses ignore case. *Recent OSC* shows what arrived and what it did, which helps when setting up a controller. Listening is saved with the show (a production file listens from the start) but is not part of undo. To test from a terminal: `oscsend localhost 8000 /rustmap/cue/go i 1` (liblo) or `sendosc 127.0.0.1 8000 /rustmap/cue/next`.

### Test patterns

- **Grid**: grid centered on the image, center cross, diagonals, circles (should look round on the wall), colored corner tags for orientation (top-left red, top-right green, bottom-left blue, bottom-right magenta), output number as yellow dots, white 1–2 px border to spot cropping, and a sweeping bar that makes stutter and tearing obvious.
- **Color bars**: primaries, a 16-step gray ramp (count the steps to check black/white clipping) and a smooth ramp.
- **Solid**: flat color for brightness and blend checks.

### Profiling

```sh
cargo install puffin_viewer
cargo run --release -- --profile     # or tick "Profiler" in the status bar
puffin_viewer                        # connects to 127.0.0.1:8585
```

Logging goes through `RUST_LOG`, e.g. `RUST_LOG=debug cargo run`.

## Layout

```
app/                    binary: event loop, windows, frame loop
crates/rustmap-core/    scene model: outputs, surfaces, media, LED fixtures and DMX layout (no GPU, no UI)
crates/rustmap-dmx/     Art-Net and sACN sender (pure Rust, UDP)
crates/rustmap-geom/    homographies, hit testing, snapping
crates/rustmap-media/   image loading and FFmpeg video playback on worker threads
crates/rustmap-ndi/     NDI in/out, runtime-loaded
crates/rustmap-osc/     OSC listener: cue go/next/back, blackout, master
crates/rustmap-render/  wgpu: output targets, test patterns, quads and meshes, edge blend, LED sampling, guides, readback
crates/rustmap-ui/      egui panels and the output editor
```

The UI edits the `Project`; each frame the app reconciles windows, GPU resources, media and NDI against it. Every output renders into its own texture at projector resolution, which is previewed in the UI, blitted (aspect-preserved) to the projector window, and read back for NDI out.

Quads are perspective-correct: the CPU computes the homography from the unit square to the quad's corners and gives each corner its homogeneous W, and the GPU's perspective-correct interpolation does the rest, so the two triangles of a quad never show a bent diagonal.

## Development

```sh
cargo test --workspace          # includes headless GPU render tests and an NDI mock runtime
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
RUSTMAP_DUMP=/tmp cargo test -p rustmap-render --test test_pattern   # writes the patterns as .ppm
```

## Releases

Downloads live on the repository's **Releases** page. To make one:

1. Set the version in `Cargo.toml` (`[workspace.package] version`) and commit.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. The *Release* workflow builds every platform and publishes the release with:
   - `RUSTMAP-<version>-macos-universal.zip`: `RUSTMAP.app` for Apple Silicon and Intel, ad-hoc signed (not notarized: first launch is right-click > Open).
   - `RUSTMAP-<version>-windows-x86_64.zip`: `rustmap.exe` with the FFmpeg DLLs (LGPL build) for video.
   - `RUSTMAP-<version>-linux-x86_64.tar.gz`: built on Ubuntu 24.04; `RUNNING.txt` lists the libraries it needs.

To try the builds without publishing, run the *Release* workflow by hand (Actions tab > Release > Run workflow); the files appear as artifacts of that run.

To build the Mac app locally: `scripts/macos-app.sh` (writes `dist/RUSTMAP.app` and a zip).

## License

MIT — see [LICENSE](LICENSE).

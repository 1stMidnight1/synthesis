# Synthesis: Decisions Log

Status values: **confirmed** (user agreed or directed), **default** (agent-chosen sensible default, may be changed by user), **proposed** (not yet accepted).
Ask before changing anything marked confirmed.

## Confirmed decisions (from planning)

| Date | Decision |
|---|---|
| 2026-11 | Y axis defaults to semitones; linear and log Hz available (R8). |
| 2026-11 | Time on X, grid from BPM (GRID-02). |
| 2026-11 | Outline plays the perimeter; filled plays the perimeter and everything inside. |
| 2026-11 | Multiple Hz at the same time sound together. |
| 2026-11 | Vertical line plays all frequencies in its span at once, like a very short filled region (R2). |
| 2026-11 | Gaps are silence. |
| 2026-11 | Volume is linear percent, not dB. Overlapping values add. Sum linearly and convert to dB only for display. |
| 2026-11 | Left and right share frequencies and differ only in gain. Purple edits both. |
| 2026-11 | Each voice has one micro shape, one macro track, and one volume lane pair. Shapes in a voice share the micro. |
| 2026-11 | Master output gain is a knob or slider, 0 to 100%. |
| 2026-11 | Constraints and 3D view are ideas, not v1 requirements. |
| 2026-11 | Export WAV files for DAW import. |

## Session decisions

| Date | Status | Decision |
|---|---|---|
| 2026-11-15 | confirmed | Project name is **Synthesis**; repo `synthesis`. Supersedes "Contour". |
| 2026-11-15 | confirmed | **macOS first**, Windows later. Code must stay cross-platform so a Windows build is packaging, not porting. Supersedes "Windows first". |
| 2026-11-15 | confirmed | Focus is the **standalone app**. VST3 plugin (M8) deferred. |
| 2026-11-15 | confirmed | CADCraft is out of scope for now. |
| 2026-11-15 | confirmed | Git: CLI commits on per-milestone branches, visible in GitKraken. Commit messages name requirement IDs. |
| 2026-11-15 | default | Stack: **Rust**, UI **egui via eframe**, audio output **cpal** (CoreAudio on macOS, WASAPI on Windows), WAV **hound**, FFT tests **rustfft**, project files **serde_json**. nih-plug is not used until the plugin milestone. |
| 2026-11-15 | default | Project file extension `.synthesis` (was `.contour`). |
| 2026-11-15 | default | Constraint solver (M6): small in-repo solver behind a trait (CONSTR-07). |
| 2026-11-15 | default | Workspace crates: `engine` (pure, no UI/audio deps), `app` (eframe + cpal). `plugin` added later. |
| 2026-11-15 | default | CI: GitHub Actions builds and tests on macOS and Windows to keep PLAT-02 honest. |

## Resolved open decisions (PLAN.md §12)

All set to the plan's listed default, status **default** (change any by telling the agent).

| # | Question | Resolution |
|---|---|---|
| D-1 | Framework | Rust. egui/eframe + cpal for the standalone; nih-plug later for plugin. |
| D-2 | Name | **Synthesis** (confirmed). |
| D-3 | Overlapping shapes at same frequency | Add. |
| D-4 | Micro depth unit | Semitones, with Hz toggle. |
| D-5 | Micro scale | Separate depth scale and period scale. |
| D-6 | Vertical edge window / sustain | 5 ms window, configurable; sustain = longer window. |
| D-7 | Lattice density default | 5 cents. |
| D-8 | Phase policy default | Random, seeded by lattice index. |
| D-9 | Fill compensation | Off (not in v1). |
| D-10 | Lane max | 200%, clip shown on meter. |
| D-11 | Master default | 70%. |
| D-12 | Export default | 48 kHz, 24-bit. |
| D-13 | Envelope scope | Per voice. |
| D-14 | Max timeline | 60 seconds. |
| D-15 | Primary time display | Seconds, with bars/beats shown. |
| D-16 | Constraint solver | Write a small in-repo solver (CADCraft out of scope). |
| D-17 | v1 shape set | All seven: line, polyline, rectangle, circle, ellipse, arc, polygon. |
| D-18 | Micro moves drawn edges? | Output only. |

## M0 findings (2026-11-15)

- Toolchain: rustup stable, Rust 1.100.0, on macOS 26.6.2 (arm64). Apple Command Line Tools present (no full Xcode needed).
- Versions in use: eframe/egui 0.37, cpal 0.18. Edition 2024, resolver 3.
- `cargo build/test/clippy` all pass on macOS. The app window opens and the cpal stream starts on the default CoreAudio device without errors.
- The `x86_64-pc-windows-msvc` target was added via rustup. `cargo check` passes for **both** `synthesis-engine` and `synthesis-app` for Windows from macOS, so cpal's WASAPI and eframe's Windows paths compile. Linking a Windows binary still needs a Windows machine or CI (the `windows-latest` job).
- Audio thread pattern: the UI talks to the callback only through atomics, and gain is ramped per sample. Later milestones will hand rendered buffers over with a lock-free SPSC queue or a triple buffer.

## M1 engine design (2026-11-17)

- **Why not a pure 5-cent lattice for everything.** A lattice only places a partial at the nearest step, so the error depends on where the target falls relative to the grid. For 450 Hz on this grid the nearest step is 449.94 Hz (-0.06 Hz), which passes the 1 Hz test by luck of alignment. Other pitches can be up to half a step (about 1.3 Hz at 450 Hz) off. Outline pitches must be exact, not grid-dependent, so the engine uses two oscillator kinds. The 1 Hz test is met without weakening it.
  - **Outline tones (exact).** Each non-vertical edge has one sine oscillator that tracks the edge's pitch at each sample. A flat 450 Hz line sounds exactly 450 Hz.
  - **Lattice bands (ENGINE-02).** Filled regions and vertical-edge bursts use a log-spaced lattice from 20 Hz at the chosen density, with persistent phases. A lattice oscillator is on when its pitch lies in an active band.
- **Geometry coordinates.** Shapes are stored as (seconds, fractional MIDI semitone), so lines and curves look straight and round on the default log axis. Hz is derived at synthesis time from A4 (R8).
- **Half-open time intervals.** A shape spans `t0 <= t < t1`, so a shared corner counts once.
- **Gating.** Lattice targets update on a 1 ms control frame (PLAN §5.1). Gains ramp linearly over `ramp_ms` (default 2 ms, ENGINE-05). Outline tones ramp on their own edge-time gate. This is click-free because no gain step is instantaneous.
- **Density compensation (ENGINE-07).** Each lattice oscillator's amplitude is `0.5 * sqrt(density / 100)`. The test checks that band RMS varies by under 1 dB between 5 and 20 cents.
- **Phase policy.** Random (default) is seeded by lattice index with SplitMix64, so renders repeat. Aligned and Schroeder are also available (ENGINE-04).
- **Vertical edges (R2).** A vertical edge sounds its whole span as one band during the burst window (default 5 ms, D-6). It is not a sweep.
- **Export (OUT-03).** WAV via hound. 16-bit and 24-bit are integer with clamping. 32-bit is float. Only 44.1 and 48 kHz are accepted.
- **Real-time caveat.** `synth.rs` allocates per control frame and is only for offline rendering in M1. The audio thread is M3+ and will use prepared buffers (AGENTS rule 3).
- **Rejected or deferred:** no micro (M2), no volume lanes (M2), no envelopes (M5), no UI.

## M1 test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`: pass.
- `cargo build --workspace`: pass.
- `cargo test --workspace`: pass. 26 unit tests in `synthesis-engine` and 7 acceptance tests in `crates/engine/tests/m1_acceptance.rs`:
  - flat 450 Hz line: FFT peak within 1 Hz (PLAN §9.1)
  - rectangle outline 200 to 400 Hz: two steady peaks, no other strong peaks (§9.2)
  - filled rectangle: band energy dominates outside, and is spread mid-band (§9.3)
  - vertical line: burst under 20 ms that contains both span ends (§9.4)
  - gap: silent in the middle (§9.5)
  - determinism: identical renders
  - loudness independent of density within 1 dB (ENGINE-07)
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo check -p synthesis-engine --target x86_64-pc-windows-msvc`: pass.
- `cargo check -p synthesis-app --target x86_64-pc-windows-msvc`: pass.

## M2 engine design (2026-11-17)

- **Micro (R4).** Per voice as `Option<Micro>`. Phase is `frac(t / (period * period_scale))`, locked to the timeline. Offset is `depth * depth_scale * micro(phase)`. Depth defaults to semitones with a Hz toggle (D-4). Depth and period scales are separate (D-5). The micro shifts the sounding frequency only. Gating still uses the macro pitch (D-18).
- **Micro shape.** Points on phase 0..1 with values -1..+1, linearly interpolated with wrap-around, so no per-sample stepping and no restart per shape. Presets (sine, triangle, saw, square) are plain point lists, so they stay editable (MICRO-02).
- **Phase accumulators (M2 change).** M1 computed lattice phase from absolute time, which can't follow a moving or micro-shifted frequency without clicks. Every oscillator is now a phase accumulator. Its phase advances only while it is audible, and it resumes at its previous phase when re-gated. Outline tones and lattice bands both use this. M1 tests are unchanged and still pass.
- **Volume lanes (R5, R6).** Each voice has a left and right lane. Each lane is a set of strokes (piecewise-linear percent over time). **Default, not confirmed in SPEC:** an empty lane is constant 100%. Once a lane has strokes, time outside every stroke is 0%. Overlapping strokes add (VOL-04), and the sum is clamped to 0..=200% (R5, D-10). Record this as a default; ask before changing it.
- **Stereo (R6).** The mono voice signal is multiplied by each lane's gain. Left and right carry identical partials and differ only by gain.
- **Master gain (R7).** Applied after the voice sum, as in M1.
- **Scope.** No UI, no envelopes (M5), no lane tools beyond the data model (M4).

## M2 test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`: pass.
- `cargo build --workspace`: pass.
- `cargo test --workspace`: pass. 38 engine unit tests (new: micro presets, wrap-around interpolation, timeline lock, depth and period scales, unit conversion, validation; lanes: empty default, stroke interpolation, add, clamp, gaps, validation) and 10 acceptance tests.
  - M1 acceptance (7) still passes after the phase-accumulator change.
  - M2 acceptance (3): sine micro with 10 ms period and ±10 Hz depth on a 450 Hz line gives symmetric sidebands near ±100 Hz, well below the carrier (PLAN §9 M2). L and R gains differ as set (power ratio 0.25 for 100% vs 50%). Overlapping lane strokes add, and 300% clamps to 200%.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo check` for `synthesis-engine` and `synthesis-app` on `x86_64-pc-windows-msvc`: pass.

## M3 design (2026-11-17)

- **Block renderer.** `SceneRenderer` keeps oscillator and gate state across calls, so playback pulls consecutive blocks without restarting phase. `render_scene` is one long block. A test confirms blocks equal an offline render exactly, so playback and export agree.
- **Owned voices.** `VoiceSynth` owns its `Voice`, so the renderer doesn't borrow the scene. This is needed for a long-lived background renderer.
- **Background render thread (TRANS-04).** A `std::thread` owns the renderer, receives commands over a channel, and fills an `rtrb` SPSC ring about 0.25 s ahead. The audio callback only pops frames and applies a 5 ms gain fade. It never renders, allocates, or locks. On a new scene or stop, a generation counter makes the audio callback drop stale frames.
- **Transport (TRANS-01 to 03).** Engine math: loop wrap, tempo position (bar, beat, sixteenth, 4/4), grid subdivisions. Playhead is advanced by wall time in the UI.
- **Pitch scale (R8).** Semitone and Log Hz are both log in Hz and differ only in labels. Linear Hz is linear. A4 is configurable. Drawing is not snapped to notes; snap rounds time to the grid and pitch to the nearest semitone only when snap is on.
- **Undo (CANVAS-05).** Snapshot-based: each edit stores the prior scene. An edit that leaves the scene invalid is rolled back and not recorded.
- **Canvas.** Pan (middle-drag), zoom (wheel), and shape tools for line, polyline, rectangle, circle, ellipse, arc, polygon, select, delete. Fill mode for new closed shapes (CANVAS-03). Fill is drawn as a tint over the bounding box for now; exact fill rendering is a later polish item.
- **Export.** WAV from the UI through `rfd` native save dialog, 24-bit, rendered through the same engine path.

**Known gaps in M3 (not hidden):**
- Fill (CANVAS-03): the Fill tool toggles the closed shape under the click (even-odd inside test). The "fill new shapes" checkbox sets the default for new shapes. Implemented.
- Loop region (TRANS-02) is a loop-to-duration toggle, not a user-set range.
- Time is shown in seconds and bars/beats (TRANS-03). The loop and export range picker (OUT-04) are not implemented.
- Fill is drawn as a bounding-box tint, not a true filled-region shape. This is display-only; the engine renders true bands (R1).
- Select/move/resize (CANVAS-07) and a user-set loop range (TRANS-02) are open. Until they are done, the M3 acceptance sentence is only partly met.

## M3 test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`: pass.
- `cargo build --workspace`: pass. `cargo build -p synthesis-app --release`: pass.
- `cargo test --workspace`: pass. Engine 50 unit, 10 acceptance (including block-vs-offline and reset). App 6 unit (screen mapping round-trip in all three scales, axis orientation, snapping, undo/redo, rollback) and 2 render-thread tests (Load produces buffered energy, Stop clears running). Includes fill-tool toggle inside/outside tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo check` for `synthesis-engine` and `synthesis-app` on `x86_64-pc-windows-msvc`: pass.
- App launch: the release binary starts and stays running for 6 seconds with no stderr output. Interactive drawing and audible playback were NOT verified by a person.

## Proposed

_None yet._

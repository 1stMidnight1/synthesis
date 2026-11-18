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
- **Volume lanes (R5, R6).** Each voice has a left and right lane. Each lane is a set of strokes (piecewise-linear percent over time). **Superseded 2026-11-17 by a confirmed decision (see "Roadmap after M3"):** an empty lane is silent, and a new voice gets a default purple stroke. (Old default: an empty lane was constant 100%; time outside every stroke was 0%.) Overlapping strokes add (VOL-04), and the sum is clamped to 0..=200% (R5, D-10). Record this as a default; ask before changing it.
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

**Added in M3 round 2 (2026-11-17):**
- **Select, move, resize (CANVAS-07).** Engine `transform` module: `translate` and `scale` about an anchor, with separate time and pitch factors (radii scale per axis). Results are validated and rejected if invalid. In the app, a click selects the topmost shape. Dragging a corner handle resizes about the opposite corner. Dragging the body moves it. A live preview shows the in-progress shape, and the change commits on release as one undo step. Escape cancels. The preview and the commit use the same `drag_result` function, so they agree.
- **Loop region (TRANS-02).** A user loop stored in the document (undoable), set on the time ruler by shift-dragging. It is clamped to the timeline and has a minimum length. Loop playback wraps inside it. Clear loop removes it. With no region set, loop uses the full timeline, as before.
- **Time ruler (TRANS-01).** Click or drag to scrub. A running playback restarts from the new position.

**Known gaps in M3 (not hidden):**
- Fill (CANVAS-03): the Fill tool toggles the closed shape under the click (even-odd inside test). The "fill new shapes" checkbox sets the default for new shapes. Implemented.
- Loop region (TRANS-02) is a loop-to-duration toggle, not a user-set range.
- Time is shown in seconds and bars/beats (TRANS-03). The loop and export range picker (OUT-04) are not implemented.
- Fill is drawn as a bounding-box tint, not a true filled-region shape. This is display-only; the engine renders true bands (R1).
- Select drag is tested at the geometry level (move, resize, anchor, rejection, pick). Pointer-level behaviour in the running window is not automated and needs a manual check.
- The loop region is not saved to a project yet (PROJ-03 is M7).

## M3 test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`: pass.
- `cargo build --workspace`: pass. `cargo build -p synthesis-app --release`: pass.
- `cargo test --workspace`: pass. Engine 50 unit, 10 acceptance (including block-vs-offline and reset). App 6 unit (screen mapping round-trip in all three scales, axis orientation, snapping, undo/redo, rollback) and 2 render-thread tests (Load produces buffered energy, Stop clears running). Includes fill-tool toggle inside/outside tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo check` for `synthesis-engine` and `synthesis-app` on `x86_64-pc-windows-msvc`: pass.
- App launch: the release binary starts and stays running for 6 seconds with no stderr output. Interactive drawing and audible playback were NOT verified by a person.

## M3 sketch-tool rework (2026-11-17)

Goal: sketch tools that behave like Onshape's. Decisions taken without a user answer; flagged here so they can be changed.

- **Two ways to draw, one result.** Every drawing tool accepts click-click and press-drag-release. Line and Polyline chain. Rectangle is two corners. Circle and Polygon are centre then a radius point. Ellipse is centre then a corner. Arc is a three-point arc (start, end, bend point). Polyline finishes on double-click, Enter, or a click on the last point; Escape cancels.
- **Round at any zoom.** Circles and regular polygons use `k`, the seconds that look as long on screen as one semitone at the pointer. Shapes stay round in the view they were drawn in.
- **Handles.** Lines and polylines: vertices. Rectangles: corners and edge midpoints (an edge drags one side). Circles and ellipses: centre and radius points. Arcs: start, middle, end, centre. Hit tolerance is 9 screen points. Only the Select tool edits existing geometry; drawing tools snap onto existing points.
- **Selection.** Click picks the nearest outline within 7 points, or a filled shape it is inside. Shift adds or removes. Dragging empty space box-selects fully enclosed shapes. Dragging a shape moves the whole selection in one undo step. Delete removes the selection; Cmd+A selects all.
- **Fill.** Fill is drawn from the same even-odd bands the audio uses (`sampling::fill_bands`), so a concave shape's notch is not tinted, and a filled shape sounds exactly the band it shows.
- **Navigation.** Trackpad scroll pans. Pinch, Cmd-scroll, and mouse wheel zoom at the pointer. Shift+zoom changes time only; Alt+zoom changes pitch only. Middle, right, or Space drag pans. Raw wheel events are used to tell a mouse wheel from a trackpad.
- **Snapping.** Points snap to existing vertices, centres, and arc points within 10 points first, then to the grid (subdivision picker). Hold Alt to disable snapping.
- **Playback.** Looping is in the audio. The renderer jumps back at the loop end. The playhead is derived from the frames the device has played, so it shows what is heard. Edits are sent to the render thread live, with a 10 ms crossfade. Playback renders at the device sample rate, so no resampling is needed.
- **Band level.** `BAND_GAIN` changed from 0.5 to 0.29 so a one-octave filled band sits near the level of one outline tone (about 0.707 RMS). The old value made fills about 1.7 times as loud. This is an agent-chosen default and can be changed.

**Testing.** `ui_tests.rs` (24 tests) drives the real app with egui_kittest: real pointer press, move, and release; real modifier state. It covers each tool (click-click and drag), arc bend direction, circle roundness on screen, polyline finish rules, snapping, select (pixel picking, box select, shift, move, handle and edge drags), undo, fill on non-rectangular shapes (including nested), fill display pixels, delete, shortcuts, wheel and trackpad navigation, and ruler scrub and loop. Screenshots are in `SYNTHESIS_SHOTS=<dir> cargo test -p synthesis-app screenshots`.

**Not done / limits.** Screenshots were inspected by eye, not compared to Onshape. Audible playback was not checked by ear. Constraints (CONSTR-*) are still not implemented.

## M3 rework test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`: pass.
- `cargo build --workspace`: pass.
- `cargo test --workspace`: pass. Engine 74 unit and 10 acceptance. App 39 unit, including 24 UI interaction tests and 1 screenshot test.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo check` for `synthesis-engine` and `synthesis-app` on `x86_64-pc-windows-msvc`: pass.
- Release build starts and stays up 6 s with no stderr.

## M3 polish (2026-11-17)

Reported after the M3 merge: curves sound strange, zooming past t = 0 shows a black strip and hides labels, labels are small and sparse, and the grid needs typed X and Y divisions.

- **Curves (CANVAS/R1).** Measured, not guessed. A circle's outline is 256 straight edges, each lasting about 8 ms, shorter than the 2 ms gate ramps at each end, so the outline sounded like a chatter of short glides. `CURVE_SEGMENTS_PER_TURN` is now 64: each edge lasts about 31 ms and changes pitch by at most 0.6 semitones. Measured spectrum and duration are in the commit notes; `circle_outline_has_steady_edges` guards the result. This keeps R1 (each crossing is one frequency). A user choice was asked (coarse tones, band, or tune by ear); the chosen option was "coarse tones".
- **Time never negative.** The view's left edge is t = 0 and `clamp_time` enforces it, so no area before the timeline can appear. The grey strip that drew for t < 0 is removed. Regression tests cover pan, zoom, and wheel.
- **Typed grid divisions.** `Division` (engine, `grid.rs`) is `1/N`, 1 to 256. X is a `1/N` of a beat (`1/16` is a sixteenth). Y is a `1/N` of a semitone in Semitones mode, and of an octave in the Hz modes, which is the same grid on a log axis. Two text fields replace the subdivision dropdown; invalid text is ignored and the last valid value is kept.
- **Pitch snap** follows the Y division (previously it always rounded to whole semitones). It is skipped when the step is under 4 screen points, so it never jumps across a pixel.
- **Labels.** Ruler labels every beat (seconds) when there is room and every bar (bar number and seconds) at a readable spacing, with a background. Pitch labels are 12 px with a background, at every step that has room. Note names in Semitones mode, Hz in Hz modes.
- **Accessibility.** The two division fields have accessible names. The accessibility check is not disabled.

**Testing.** Engine: `grid` unit tests (parse, bounds, steps), `circle_outline_has_steady_edges`. App: `view_never_shows_time_before_zero`, snap tests for X and Y division, UI tests for the wheel past zero, typed X and Y divisions, and invalid text. 46 app tests, 79 engine unit, 11 engine acceptance.

**Not verified.** How the curves sound is still a measurement, not a listening test. The label readability is judged from screenshots. The curve sound needs your ear.

## M3 polish test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo check` for `synthesis-engine` and `synthesis-app` on `x86_64-pc-windows-msvc`: pass.
- Release build starts and stays up 6 s with no stderr.

## M3 polish: curve sound variants (2026-11-17)

Curves no longer sample into straight edges; a circle, ellipse, or arc is split at its time extremes into branches, each a single-valued function of time, with one continuous oscillator per branch (`branch.rs`). Arc pitch error against the analytic curve fell from 11.0 to 6.3 cents median.

Four variants are selectable per voice ("Curve sound" in the sketch panel), so the choice is made by ear:

- **Literal (default):** both branches together, 2 ms fades. R1 as written.
- **Both, long fades:** both branches, 15 ms fades at the turn points.
- **Upper only:** the upper branch alone.
- **Lower only:** the lower branch alone.

"Export curve variants..." writes all four as WAVs into a folder. Listening files for a circle and an arc are in `~/Desktop/synthesis_curve_listening/`, written by `crates/engine/tests/write_listening.rs` (`SYNTHESIS_LISTEN=<dir>`).

**Known caveat for the listening test.** Both-branch variants sum two tones and reach full scale (peak 1.000 in the export); single-branch variants peak at the 0.7 master gain. They are about 3 dB apart, so compare them at matched level. Loudness is not normalised per variant; that is a decision for you.

**Why "Literal" and "Both, long fades" sound the same (measured).** They are not bit-identical overall, but they differ only at the two turn points, the start and end of the circle. Over the middle of the circle (0.5 s to 0.75 s) the two renders are bit-identical. The differing region is about 30 ms of a 1.5 s render, about 0.8% of the energy, so a 15 ms versus 2 ms fade is a subtle change to the attack and release, not to the sustained sound. Judge the curve sound on the sustained middle, or use a longer fade to make the variant audible. An earlier draft of this diagnosis blamed phase advance; a measurement showed that was wrong and it was reverted.

**Why the middle of a circle is chattery and the ends are smooth (measured).** At the middle of a circle the two branches are an octave apart (311 Hz and 622 Hz at t = 0.5 s); the ratio falls to about 1.2 near the ends, where the branches meet. A spectrum window at 0.3 s to 0.6 s shows both families of partials at once (312 to 318 Hz and 606 to 621 Hz). Upper-only removes the lower family. Near the turn points the two pitches are close, so they blend into one sound, which is the smooth part you hear. An arc's tail has only one branch, which is why the end of an arc sounds smooth too.

**A single-branch arc is measurably smooth.** An upper half-ellipse (one branch) has one spectral peak in its middle, at 622.6 Hz against 622.3 Hz expected, and its cycle-to-cycle pitch jitter is 0.002 Hz median, 0.03 Hz at the 95th percentile. The only large jitter is 1.4 Hz at the first cycle (t = 0.50 s), where the onset settles. Peak-count analysis in 10 ms windows is not a valid roughness measure for a fast glide: the window smears a clean sweep over several bins. The two-pitch explanation does not account for the garbling of single-branch arcs. Open question, to be settled by listening: where in the arc it is heard (onset, middle, or end).

**Why a lens of two arcs can sound fine when a circle does not (measured, not heard).** A lens (two arcs sharing both ends) also sounds two pitches at once at its middle, with the same loudness ripple as the circle (1.03 vs 1.04). Its interval is 15 semitones, not the circle's 12. Counting coincident partials (first 12 harmonics, within 20 cents), the circle's octave has 6 shared partials and the lens's 15 semitones has 1. Shared partials make two tones fuse into one harmonic sound; few shared partials keep them as two voices. This is an objective proxy for consonance. Whether it matches what is heard, and whether layered arcs read as separate voices, is not yet verified by ear.

**The curve WAVs were clipping, not the curve treatment (measured).** Live playback and export are bit-identical (max sample difference 0.000000000), so the synth and the render path agree. The problem is the final stage: the circle-plus-arc scene, at master gain 0.7, peaks at 1.40 with 14,639 of 168,000 samples (8.7%) above full scale. The 24-bit WAV clamps those flat at +/-1.0, which adds harmonic distortion and a gritty, buzzy character. Neither the render nor the playback path limits or normalises, so the live app clips the same way. The fix belongs in the output stage: a limiter or a choice of master level. Not yet changed; it is a mix decision.

**Measured, not yet heard.** The two branches of a circle (about 311 Hz and 622 Hz at the middle time, an octave apart) sum to a waveform that is not a steady tone. I have not verified by ear whether that sounds rough. The earlier "beating" explanation was wrong: a 2:1 pair gives a periodic waveform, not a slow beat. The variants exist so the difference can be heard directly.

## M3 polish: curve variants test results (2026-11-17, macOS arm64)

- `cargo fmt --all -- --check`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`: pass. Tests: 46 app, 86 engine unit, 13 engine acceptance.
- New tests: branch geometry (single-valued, on the curve, analytic rate matches finite differences, arc split), `CurveSound` selection, variant branch selection (upper-only and lower-only are each at least 20x weaker on the silent pitch), variants export as distinct WAVs.
- `cargo check` for `synthesis-engine` and `synthesis-app` on `x86_64-pc-windows-msvc`: pass. Release build: pass.

## M3 polish: master gain and the arc pop (2026-11-17)

- **Master gain (OUT-01), adjustable.** A 0 to 100% slider is in the transport bar, next to Export. It writes the scene's master gain as one undoable step (committed on release), and it is shared by live playback and export, since both render from the scene settings. Default stays 70%. A test confirms that a 30% master keeps the circle-plus-arc scene inside full scale while 70% clips it, and that the level scales exactly linearly.
- **Arc start and end clicks (measured; causal link pending listening).** The reported click is at each arc's start and end. Earlier "no step" checks looked for a sample-level jump, which is the wrong test. Measured: at the arc's end the sound is at full level up to the cut and then ramps to zero over the 2 ms fade (envelope 1.00, 0.87, 0.49, 0.00). With the 15 ms fade it ramps over about 15 ms (1.00, 0.94, 0.92, 0.86 ... 0.00). At the start, the 2 ms fade reaches full level in 2 to 3 ms and the 15 ms fade in about 15 ms. A 2 ms fade is a fast edge; at each arc end the pitch is sweeping at its fastest, so this is the most likely click. The earlier statement that the long fade had no effect at the end was wrong: it does change the end envelope. Confirmation requires listening to `arc_ends_literal.wav` and `arc_ends_long_fade.wav` (written to the Desktop folder).
- **In-app vs export (measured).** Live playback through the real ring and player, paced at 48 kHz, is bit-identical to the export (max sample difference 0.000000), and the step at each arc start and end matches. One underrun in 1,312 chunks at startup. The device is the built-in speaker: 48 kHz, stereo, float, buffer range 15 to 4096 frames, default chosen by the OS. So the app and the file produce the same samples; any difference heard in the app comes after the samples leave the app (device processing, a speaker's low-frequency response, or the output chain). Not verified by ear; depends on the output device.
- **The earlier pop search (superseded).** Measured, not guessed: (1) offline, an arc's onset is a smooth 2 ms ramp (first sample 0.0007, largest 5 ms step 0.034); (2) an arc's end fades to exactly zero with a final step of 0.003; (3) through the live player, a sound ringing at the timeline end is cut with a step of 0.0000. So no discontinuity was found at the start or end. A lower master makes any pop quieter but does not remove a real discontinuity. The pop is open: it may come from something the offline render does not capture, or from the audio device. Needs the listener to say when it happens (press, release, loop wrap, or each arc).

**Test results (2026-11-17, macOS arm64).** `cargo test --workspace`: 48 app, 86 engine unit, 14 engine acceptance, 1 render. `cargo clippy --workspace --all-targets -- -D warnings`: pass. Windows type-checks for engine and app: pass. Release build: pass.

## M3 polish: arc-end pops fixed by a longer fade (2026-11-17)

**Confirmed by ear on headphones.** The 2 ms fade on curve branches produces an audible pop at each arc's start and end. The 15 ms fade removes it. Earlier measurements had pointed here, but only a listening test could confirm it, and the in-app and export paths produce identical samples, so the pop is a property of the signal's fade, heard most clearly on headphones and at low frequency.

**Change.** The default `CurveSound` is now `LongFade` (15 ms fades at the turn points). The 2 ms variant is kept as "Both, short fades (2 ms)" for comparison. Regression test `default_curve_sound_has_soft_arc_ends`: the 2 ms fade is over by 3 ms, the 15 ms fade is still ringing at 3 and 10 ms.

**Measurement note.** A first version of the regression test read the wrong 1 ms block and failed. The check was on my side: it measured the block at the fade's tail instead of the block after it. Corrected to whole 1 ms blocks.

**Still open.** The 15 ms default is a fix for the pop, not a tuned sound. The fade length could be shorter (for example 5 to 10 ms) if the start still reads as a pop-free attack too slow; that is a listening choice. Also, the built-in speaker may colour low frequencies, so a fade that is clean on headphones may still sound different there.

**Test results.** `cargo test --workspace`: 48 app, 86 engine unit, 15 engine acceptance, 2 others. `cargo clippy --workspace --all-targets -- -D warnings`: pass. Windows type-checks: pass. Release build: pass.

## Roadmap after M3 (2026-11-17, confirmed)

**Order confirmed by the user:** M2.5 engine performance, then M4 volume canvas, then M5 (5a voices and master view, 5b micro editor and ADSR), then M7 projects, then M6 constraints, then M9 polish. Plugin (M8) stays deferred.

**Volume lane rules (confirmed, for M4).**
- A new voice gets a default purple stroke (both channels) at a reasonable level, shown on the volume canvas. Level to be chosen in M4 and recorded; proposed 80%.
- An empty lane emits no sound (changes the M2 default, where an empty lane was 100%).
- A gap between strokes is silent (unchanged).
- Engine change and tests land in M4 with the lane UI. Consequence to handle there: a voice created before M4, or any lane whose strokes are all deleted, is silent, so the default stroke must exist from voice creation.

**Pitch label gutter (2026-11-17).** Pitch labels moved out of the grid into a 56-point gutter on its left, right-aligned and centred on their lines, with a tick at the grid edge. The ruler reserves the same gutter so its time axis lines up with the grid. Labels that would be clipped at the gutter's top or bottom are skipped. Regression test `pitch_labels_are_outside_the_grid` renders the app and checks that the grid's left edge has no label pixels.

**Curve sound control.** The panel control was a combo box with its label to the side, which made the row wider than the panel and rendered as a tall box. The label is now on its own line, the box fills the panel width, and it shows a short name ("Long fades"). It stays named for accessibility through `labelled_by`.

## M2.5 engine performance (2026-11-18)

Profiled before changing anything. A 5-octave fill spent nearly all its time in `VoiceSynth::process`, mostly in `sin` (libm), and the lattice inner loop was the cost. Changes, in order of effect:

1. **Active lattice ranges.** Only oscillators that can be audible are visited. Stored as ascending contiguous ranges, not an index list (the index list cost about 3x in the inner loop).
2. **Settled ranges.** A range whose gains and targets are all 1 skips the ramp test each sample, and its flag is kept up to date once per control frame.
3. **Fast sine.** `osc::fast_sin`, a degree-15 odd polynomial with symmetry reduction, error below 1e-11 (about -220 dBFS), roughly 3x faster than libm.
4. **Cached edges, scratch buffers.** Shape edges are flattened once per voice; control-frame gating reuses buffers and no longer allocates per frame.
5. **Voices in parallel.** Each voice renders on its own thread for blocks of at least 8192 frames; voice buffers are summed in voice order, so the result does not depend on threading (checked: identical on repeated runs).

**Output contract changed, on purpose.** The fast sine and the reordered per-voice sum change the last bit of some float samples. The golden test (`perf_golden.rs`) now checks the largest difference against the reference is at most one 24-bit quantisation step (2^-23, about -138.5 dBFS). Measured difference: 1.19e-7, one LSB. The export quantises to 24 bits, so this does not change the exported file. Earlier wording that said output was bit-identical is superseded by this.

**Speed (Apple M3, release, 10 s of audio, best of 3):**

| Scene | Before | After |
|---|---|---|
| One line | 8.1x | 538x |
| Filled 1 octave | 4.6x | 32.6x |
| Filled 5 octaves | 1.5x | 7.0x |
| Filled circle, 3 octaves | 2.7x | 10.7x |
| 4 voices, 5-octave fill | 0.4x | 5.7x |
| 8 voices, one line | 1.0x | 229x |

**Accept criteria (from the plan):** a single line at 50x or better (met); a 5-octave fill at 4x or better (met, 7.0x).

**Not yet done.** Live playback has a per-block cost too, which the offline benchmark does not measure; the parallel path only engages for blocks of 8192 frames or more, so live blocks (1024) run on one thread. Whether that is fast enough for 4+ voices of wide fills on a live stream is untested and should be checked in the app.

## Cleanup: fill tool and curve selector removed (2026-11-18)

Confirmed by the user: remove the fill feature, and remove the curve playback selector while keeping the long fade on.

**Removed (app).** The Fill tool and its F shortcut, "New shapes filled", "Toggle fill", fill-by-click, and fill-on-selection. New shapes are always outlines, and the rubber-band preview is drawn as an outline.

**Removed (curve selector).** The `CurveSound` enum (Literal, LongFade, UpperOnly, LowerOnly), the `Voice::curve_sound` field, the Curve sound combo box, and the "Export curve variants" button. The upper-only and lower-only modes and the 2 ms fade are gone with them.

**Kept.** The curve fade is a fixed constant, `CURVE_FADE_MS = 15`, applied to every curve branch. Both branches of a closed curve always sound together. The post-end envelope is unchanged (0.94, 0.92, 0.86 ... declining over about 15 ms).

**Kept on purpose: filled shapes in the engine.** A filled shape still sounds as a band of frequencies. This is the M1 acceptance requirement in PLAN.md section 9 ("a filled rectangle renders a band with energy between the two edges") and is covered by its tests, so removing it would break an accepted milestone. Only the editing feature is removed. The canvas still tints a filled shape so the display matches what sounds. If you want filled shapes removed from the engine as well, that changes an accepted requirement and needs a separate decision.

**Test changes.** Removed the variant-selection, variant-export, default-variant and listening-writer tests, which existed only for the removed modes. Replaced the fill-tool and F-key tests with the rectangle test that checks drawn shapes are outlines. Kept the filled-shape display test. Engine: 87 unit, 12 acceptance, 1 golden. App: 47.

**Checks.** fmt, workspace tests, clippy with warnings denied, Windows type-checks for engine and app, release build: all pass.

## Proposed

_None yet._

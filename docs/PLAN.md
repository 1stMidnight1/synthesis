# Synthesis: Project Plan

> **Overrides (2026-11-15).** This plan was drafted under the working name "Contour". The following supersede the text below; see `docs/DECISIONS.md` for the full log.
> - The project is named **Synthesis** (repo `synthesis`). Read "Contour" as "Synthesis" throughout. Project files use the extension `.synthesis`.
> - **macOS first**, Windows later. All code must stay cross-platform (no platform-specific APIs outside small, isolated modules) so a Windows build is a packaging task, not a port.
> - **Focus is the standalone app.** The VST3 plugin (M8, section 4.13) is deferred. The engine stays UI- and plugin-free so the plugin remains possible.
> - CADCraft is out of scope for now. Constraints (M6) will use a small in-repo solver behind an interface.
> - Open decisions in section 12 are resolved to their listed defaults unless noted in DECISIONS.md.

Status: planning draft v0.1
Platform: macOS first, Windows later. Personal use only.
Deliverables: a standalone app that renders drawings to WAV. A VST3 plugin using the same engine is a later, deferred deliverable.

This document is the handoff plan. It defines what to build, the order to build it in, and the decisions still open. Items marked **[OPEN]** need an answer before the related milestone starts. Items marked **[ASSUMED]** are my working interpretation and should be confirmed or corrected.

---

## 1. Summary

Contour is a drawing-based synthesizer. The user draws shapes on a pitch-versus-time canvas using CAD-style tools. Each shape describes which frequencies sound at each moment. Each *voice* has a micro shape (a user-drawn small-scale waveform that transforms pitch) and a volume lane pair (left and right, drawn as percentages over time). A master view shows all voices together. The app plays the result and exports WAV files.

It is conceptually similar to UPIC (Xenakis, 1977) and MetaSynth, but it works with pitch paths and shaped regions instead of painted spectra.

---

## 2. Glossary

| Term | Meaning |
|---|---|
| **Voice** | One instrument. Owns one micro shape, any number of macro shapes, and one volume lane pair (L, R). Analogy: a synth patch plus a MIDI track plus an automation track. |
| **Macro shape** | A vector shape on a voice's pitch-time canvas: line, polyline, rectangle, circle, ellipse, arc, or polygon. Shapes are outlines only (filled shapes removed 2026-11-18). |
| **Micro shape** | One cycle of a user-drawn waveform. It is applied as a pitch transform to its voice. All macro shapes in a voice share it. |
| **Volume lane** | A curve on a volume-versus-time canvas. Left (red) and right (blue) have their own lanes, and purple edits both at once. |
| **Slice** | The set of frequencies sounding at one instant *t*. |
| **Partial** | One sine oscillator in the output. |
| **Lattice** | A fixed set of persistent oscillators spaced evenly in log-frequency. Each one has its own phase. |
| **Density** | The spacing between lattice oscillators, in cents. |

---

## 3. Core model and rules

These rules are what the engine must implement. Each one should have a test.

**R1. Sounding set.** At each time *t*, the frequencies sounding in a voice are determined by where its macro shapes meet the vertical line at *t*:
- **Outline:** each edge crossing is one frequency. A rectangle sounds two pitches between its left and right sides.
- ~~**Filled:** every frequency between crossings sounds, as a band.~~ Removed 2026-11-18.

Multiple shapes in one voice contribute together. **[OPEN]** If two shapes overlap at the same frequency, do their contributions add, or does the frequency count once?

**R2. Vertical edges.** A vertical line at time *t* produces a slice containing all frequencies in its span, all at once. This is the same as a very short filled region. The slice gets a small time window, default about 5 ms, configurable. It is not a sweep.

**R3. Gaps are silence.** Where no shape crosses the vertical line, the voice is silent.

**R4. Micro transform.** The micro shape offsets the whole slice of its voice:

```
offset(t)   = depth * micro(frac(t / period))      micro() returns -1..+1
f_out(t)    = f_macro(t) * 2^(offset(t) / 12)      when depth is in semitones
```

- `period` is in milliseconds.
- The micro cycle is locked to the timeline. It is not restarted for each shape.
- **[ASSUMED]** Depth is in semitones (or cents) by default. This keeps the transform consistent with the log pitch axis. A fixed-Hz mode is also offered.
- **Worked example:** ±10 Hz at 450 Hz is about ±0.38 semitones, or about ±38 cents.
- **[ASSUMED] "Adjustable scaling"** means a global multiplier on depth, plus an optional multiplier on period. Both are applied to the micro, so the drawn shape does not need to be redrawn.

**R5. Volume.** Each channel's gain is `lane_percent / 100`, applied linearly. Lane values that overlap in time add. **[ASSUMED]** Each lane is clamped to 0 to 200%, and the meter shows clipping.

**R6. Stereo.** Left and right carry identical partials. Only their gains differ, so L/R acts as a balance control.

**R7. Mixing.** Voices sum linearly. A master gain (0 to 100%) is applied after the sum. Summing linearly and converting to dB only for display is the agreed approach.

**R8. Pitch scale.** The Y axis has three modes:
- **Semitones:** log scale with note labels. Default.
- **Log Hz:** log scale with Hz labels.
- **Linear Hz:** linear scale.

Notes are labeled on the scale for reference. Drawing is not forced onto notes. A4 reference is 440 Hz by default and is configurable.

**R9. Units.** The volume canvas uses percent (0 to 100 by default), not dB. Time is in seconds, with the grid tied to BPM.

---

## 4. Requirements by area

IDs are for reference in commits and issues.

### 4.1 Canvas and drawing (CANVAS)
- **CANVAS-01** Pitch-time canvas per voice with pan (middle-drag) and zoom (wheel).
- **CANVAS-02** Tools: Select/Move, Line, Polyline, Rectangle, Circle, Ellipse, Arc, Polygon, Delete.
- ~~**CANVAS-03** Click inside any closed shape with the Fill tool to toggle outline or filled mode.~~ Removed 2026-11-18.
- ~~**CANVAS-04** Each shape shows its mode (outline or filled) visually.~~ Removed 2026-11-18.
- **CANVAS-05** Undo and redo for every edit.
- **CANVAS-06** Shapes are stored as vector geometry, not pixels. Rasterizing is only for display.
- **CANVAS-07** Shapes can be selected, moved, and resized. Handles appear on selection.

### 4.2 Constraints (CONSTR). Staged, see milestones.
- **CONSTR-01** Snap to grid (on or off, toggleable).
- **CONSTR-02** Horizontal and vertical alignment between points and lines.
- **CONSTR-03** Coincident points, equal lengths, and equal radii.
- **CONSTR-04** Dimensions: length, radius, and angle. Dimensions can be in semitones, Hz, or seconds, following the active scale.
- **CONSTR-05** Tangent and perpendicular (normal) relations between lines, circles, and arcs.
- **CONSTR-06** Constraints re-solve after every edit. The app reports conflicts and redundancies.
- **CONSTR-07** The solver is kept behind a small interface so it can be swapped out. See section 7.

### 4.3 Grid and scale (GRID)
- **GRID-01** Grid is toggleable.
- **GRID-02** Time grid is set from BPM. Subdivisions include bar, beat, 1/8, 1/16, and 1/32. Seconds are also available.
- **GRID-03** Y grid spacing is configurable. Units follow the active pitch scale.
- **GRID-04** Note labels appear along the pitch axis (semitone mode and Log Hz mode).
- **GRID-05** BPM is editable in the app. In the plugin, it follows the host, and the field then shows the host value.

### 4.4 Voices (VOICE)
- **VOICE-01** Create, rename, delete, and duplicate voices.
- **VOICE-02** Each voice has a color, a visibility toggle, and a mute toggle.
- **VOICE-03** Each voice contains: one micro shape, zero or more macro shapes, and one volume lane pair.
- **VOICE-04** Multiple voices can be on the timeline at once. Their outputs sum.

### 4.5 Volume lanes (VOL)
- **VOL-01** Volume canvas per voice, with percent on the Y axis (0 to 100 by default) and time on X.
- **VOL-02** Left lane red, right lane blue, and purple to edit both together.
- **VOL-03** A horizontal line is a constant level. Lanes use the same tools as the pitch canvas, but only for line, polyline, and freehand.
- **VOL-04** Overlapping lane values add, as in R5.

### 4.6 Micro shape (MICRO)
- **MICRO-01** Dedicated editor with one cycle drawn on a 0 to 1 horizontal axis and a -1 to +1 vertical axis.
- **MICRO-02** Starter presets: sine, triangle, saw, square. Presets can be edited.
- **MICRO-03** Period in milliseconds, with a numeric field.
- **MICRO-04** Depth with a unit toggle: semitones or Hz.
- **MICRO-05** Global micro scale (multiplier on depth) and optional period scale.
- **MICRO-06** Micro is per voice. All of that voice's macro shapes use it.

### 4.7 Envelopes (ENV)
- **ENV-01** ADSR envelope per voice, applied to the voice's output amplitude.
- **ENV-02** **[OPEN]** Whether envelopes also apply per shape, or to the micro depth.
- **ENV-03** Envelope curves are shown on the volume canvas or in a small panel.

### 4.8 Sound engine (ENGINE)
- **ENGINE-01** Render the scene offline to a stereo buffer at a set sample rate.
- **ENGINE-02** Lattice synthesis (section 5).
- **ENGINE-03** Density is configurable from 1 to 100 cents. Default 5 cents.
- **ENGINE-04** Phase policy is configurable (section 5.3).
- **ENGINE-05** Click-free gating: a short ramp on each lattice oscillator when it turns on or off.
- **ENGINE-06** Offline export is the reference quality. Live preview may use lower density, and it must say so.

### 4.9 Transport and playback (TRANS)
- **TRANS-01** Play, stop, and a playhead. Click or drag the time ruler to scrub.
- **TRANS-02** Loop region.
- **TRANS-03** Time display in seconds and in bars and beats.
- **TRANS-04** Playback runs the engine on a background thread so the UI stays responsive.

### 4.10 Output and export (OUT)
- **OUT-01** Master output gain, 0 to 100%, with a knob or slider. Default **[ASSUMED]** 70%.
- **OUT-02** Stereo output meter with peak and clip indicator.
- **OUT-03** Export WAV: 44.1 or 48 kHz, 16, 24, or 32-bit float. Default **[ASSUMED]** 48 kHz, 24-bit.
- **OUT-04** Export time range can be the whole project or the loop region.
- **OUT-05** Optional normalize on export. Off by default.

### 4.11 Master view (MASTER)
- **MASTER-01** Shows all voices together on one pitch-time canvas. Each voice keeps its color.
- **MASTER-02** Per-voice visibility toggles. Each voice's macro shapes can be shown or hidden here.
- **MASTER-03** Volume is shown in a strip beneath the pitch view.
- **MASTER-04** 2D in v1. A 3D view is a later option, not part of v1.

### 4.12 Projects (PROJ)
- **PROJ-01** Save and load a project as a versioned JSON file with extension `.contour`.
- **PROJ-02** Autosave to a separate recovery file.
- **PROJ-03** Settings (BPM, grid, scale, A4, density, export defaults) are saved with the project.

### 4.13 Plugin (PLUG)
- **PLUG-01** VST3 build from the same engine.
- **PLUG-02** Host transport and tempo drive playback and BPM.
- **PLUG-03** The plugin renders the scene into the host's audio output. It loops with the host's transport, or plays once from a button.
- **PLUG-04** Project state is saved inside the DAW session.
- **PLUG-05** The audio thread never allocates, locks, or does file I/O. Rendering is prepared on a background thread and handed over.

---

## 5. Sound engine design

### 5.1 Slice sampling
The engine converts the drawing into slices at a control rate. Each slice is the set of frequency intervals active at time *t*.
- Control rate default: one slice every 1 ms. Vertical edges use their own short window (R2).
- Each slice is computed by intersecting the vertical line at *t* with every shape in the voice. Outline shapes give points, filled shapes give intervals.
- Shapes can be prepared in advance (bounding boxes, edge lists) so this is fast.

### 5.2 Lattice synthesis (recommended)
The simplest correct approach is one oscillator per partial, with a phase accumulator. Filled regions can require thousands of partials, so the lattice approach keeps this manageable:

1. **Fixed lattice.** Create oscillators at fixed log-frequency spacing across the audible range (for example 20 Hz to 20 kHz at 5 cents, about 2,400 oscillators). Each oscillator has a fixed base frequency and its own persistent phase.
2. **Gating.** For each slice, an oscillator is on if its base frequency lies inside the active set. Its gain ramps toward on or off over about 2 ms (ENGINE-05).
3. **Micro as a global pitch multiplier.** The micro offset from R4 multiplies every oscillator's frequency by the same factor. The gating uses the macro frequency, and the sound uses the transformed frequency.
4. **Accumulate.** Each oscillator advances its phase by `2π · f_actual / sampleRate`, then outputs `gain · sin(phase)`.
5. **Mix.** Sum all oscillators, multiply by voice envelope, then by the voice's L and R lane gains, then sum voices, then apply master gain.

Why a lattice: phases persist across time, so frequency changes don't click, and an oscillator that turns off and back on keeps a consistent phase. Density trades off between smooth, noise-like textures (fine spacing) and resolvable pitches (coarse spacing). Coarse spacing with a fill can also produce audible beating. Expect this and tune it by ear.

**Worked cost estimate:** 2,400 oscillators at 48 kHz for 10 seconds is about 1.2 billion sine evaluations. That is fine offline. Live playback will need either lower density, oscillator skipping (only inactive oscillators with a non-zero ramp are computed), or an FFT-based overlap-add approach later.

### 5.3 Phase policy (partial phase)
Each partial's starting phase matters whenever several partials add together.
- **Aligned (all zero):** evenly spaced partials with zero phase all peak together. With 100 Hz spacing, that is a pulse every 10 ms, which sounds like a buzz at 100 Hz. The peak of N aligned partials is N times the single-partial amplitude.
- **Random (default):** each oscillator gets a fixed pseudo-random starting phase, seeded by its lattice index so renders are repeatable. The sum sounds noise-like, and its peak stays a few times its average, not N times.
- **Schroeder:** a quadratic phase sequence that keeps the sum's envelope fairly flat. Good for dense, even textures.

Phase is never reset per slice. Phase continuity is what keeps moving edges clean.

### 5.4 Loudness
- **[ASSUMED]** Each oscillator is scaled by a factor that depends on density, so changing density does not change loudness. Overall loudness can then still vary with how many partials are active.
- **[OPEN]** Whether to add a "fill compensation" option that scales by the number of active partials in each slice. This would keep loudness steadier but adds a control.
- Final clipping is managed by master gain, the meter, and optional export normalization (OUT-05).

### 5.5 Micro cycle
- The micro is stored as a list of points on 0 to 1 (phase within one cycle) with values -1 to +1.
- Evaluate with linear or cubic interpolation. Interpolation is between drawn points, not per-sample stepping, so the micro does not add aliasing.
- The micro's phase is `frac(t / period)`, with `t` in seconds from the start of the timeline. Period is in ms.

---

## 6. UI design

**Main window layout:**
- Top bar: project name, save, load, export, BPM, grid controls, scale selector, A4 reference, master gain, transport buttons.
- Left: voice list (color swatch, name, visibility, mute, add, delete).
- Center: active view. Tabs for Pitch (macro), Volume, Micro, and Master.
- Bottom: time ruler, playhead, loop region, and meter.

**Pitch canvas:** pan and zoom, time grid, pitch grid, note labels, shape handles, a tool palette, and a snap toggle. Outline and filled shapes are drawn differently.

**Volume canvas:** percent scale on Y, time on X, red and blue lanes, purple for both, with overlap shown when lanes add.

**Micro editor:** a single cycle with a grid. Presets menu, period field, depth field and unit, scale field.

**Master view:** all visible voices together, with their colors and layer toggles. Volume strip beneath.

**Visual rules:** every voice keeps the same color everywhere it appears. Selected objects are highlighted in one consistent color.

---

## 7. Technology and architecture

### 7.1 Recommended stack [OPEN, confirm before M0]
- **Language:** Rust.
- **Plugin framework:** nih-plug. It exports VST3 and CLAP from one codebase. Its editor can use egui, which is the same UI library CADCraft uses.
- **UI:** egui (via nih-plug's egui support for the plugin, and directly for the standalone app).
- **WAV export:** `hound` (WAV writer). Verify it supports 24-bit and float.
- **FFT (tests and analysis):** `rustfft`.
- **Constraint solver (later):** see 7.2.

**Why Rust:** the constraint solver in CADCraft (a separate crate, `cadcraft-constraints`, a weighted damped Gauss-Newton solver with conflict and redundancy detection) is written in Rust and dual-licensed MIT or Apache-2.0. A Rust project could reuse it directly, or adapt it, which saves building a solver from scratch.

**Alternative: JUCE (C++).** Mature for VST3 and standalone builds, with strong audio and custom-drawing support. The trade-off is C++ and no ready-made solver, so the solver would be written or integrated separately. Choose JUCE if you prefer C++ or want the most established plugin path.

**Verification needed in the first spike (M0):**
- nih-plug standalone builds on Windows, and which audio backend it uses.
- nih-plug egui editor works in both the plugin and standalone builds.
- The CADCraft constraints crate can be separated from CADCraft's other crates (it depends on its own geometry types). If it can't, the plan is to write a smaller solver.

### 7.2 Constraint solver options
1. **Adopt** `cadcraft-constraints` from CADCraft, adapting its geometry types. Fastest route to the full feature set, if the crate separates cleanly.
2. **Integrate** a solver through a thin interface.
3. **Write** a small solver for the constraint subset in CONSTR-02 to CONSTR-05. Most work, but the simplest dependency.

Recommended: build M6 with option 1 if the spike shows it's separable. Otherwise use option 3 and keep the interface so the solver can be swapped later.

### 7.3 Architecture
```
engine (pure Rust, no UI)
  geometry    shapes, intersections, slice sampling
  model       voices, shapes, lanes, micro, envelopes, settings
  synth       lattice oscillators, gating, phase policy, micro transform
  render      offline render to buffer, WAV export
  project     JSON load and save, schema versioning

app (standalone, egui)
  canvas, tools, grid, transport, export UI, uses engine

plugin (nih-plug, VST3 and CLAP)
  host sync, parameter exposure, editor, uses engine
```

**Rule:** the engine never depends on egui or nih-plug. The UI and plugin layers can be replaced without changing the engine.

**Threading:**
- UI thread: editing, drawing, and building slice data.
- Render thread: offline render and playback buffer generation.
- Audio thread (plugin): reads prepared buffers only, no allocation, no locks.

---

## 8. Repository layout

```
contour/
  AGENTS.md                  agent instructions (read by opencode and others)
  CLAUDE.md                  imports AGENTS.md for Claude Code
  Cargo.toml                 workspace
  docs/
    PLAN.md                  this file
    SPEC.md                  detailed requirements, kept current
    DECISIONS.md             decisions log with dates
  crates/
    engine/                  geometry, model, synth, render, project
    app/                     standalone egui app
    plugin/                  nih-plug VST3 and CLAP wrapper
    constraints/             constraint solver (added in M6)
  tests/
    fixtures/                sample .contour projects
    audio/                   golden tests (FFT peak checks)
```

---

## 9. Milestones and acceptance criteria

Work one milestone at a time. Each ends with tests passing, a commit or tag, and an updated DECISIONS.md.

**M0. Bootstrap and spike (Windows).**
- Toolchain installed: Rust (MSVC), Visual Studio Build Tools, Git.
- Empty workspace builds.
- nih-plug example builds as VST3, loads in a host, and produces audio.
- nih-plug standalone window opens with an egui panel.
- Constraint crate separability is checked and noted in DECISIONS.md.
- *Accept:* a build produces a loadable VST3 and a runnable standalone app.

**M1. Engine core (no UI).**
- Shape types: line, polyline, rectangle, circle, ellipse, arc, polygon.
- Slice sampling per R1 to R3.
- Simple synth: lattice, phase policy, gating, no micro, constant volume.
- WAV export.
- *Accept (tests):*
  - A flat 450 Hz line for 1 s renders a peak at 450 Hz (FFT within 1 Hz).
  - A rectangle outline between 200 Hz and 400 Hz renders two steady peaks.
  - ~~A filled rectangle renders a band with energy between the two edges.~~ Removed 2026-11-18 with filled shapes.
  - A vertical line at time *t* renders a short burst containing its whole range.
  - A gap renders silence.

**M2. Micro transform and volume lanes.**
- Micro shape with period, depth, and scaling (R4).
- Volume lanes per channel (R5, R6). Overlapping values add.
- Master gain.
- *Accept (tests):* a sine micro with 10 ms period and ±10 Hz depth on a 450 Hz line produces sidebands about 100 Hz from the carrier, with the expected small modulation index. Left and right gains differ as set.

**M3. Standalone canvas and transport.**
- Pitch canvas with pan and zoom, shape tools. (Outline and fill toggle removed 2026-11-18.)
- Grid (BPM-based time grid, Y grid), note labels, scale modes (R8).
- Transport with play, stop, playhead, loop, and time display.
- Export WAV from the UI.
- *Accept:* a user can draw each shape type, play it, and export a WAV that matches the engine test results.

**M4. Volume canvas.**
- Red, blue, and purple lanes, with overlap display (VOL-01 to VOL-04).
- Freehand, line, and polyline tools for lanes.
- *Accept:* a lane edit changes the exported loudness as expected.

**M5. Voices and master view.**
- Multiple voices, each with its own micro, shapes, and lanes (VOICE-01 to 04).
- Micro editor with presets (MICRO-01 to 06).
- Master view with visibility toggles (MASTER-01 to 03).
- Envelopes (ENV-01).
- *Accept:* two voices with different micros play together, can be toggled, and export correctly.

**M6. Constraints.**
- Stage A: snap, horizontal, vertical, coincident (CONSTR-01 to 03).
- Stage B: dimensions and angles (CONSTR-04).
- Stage C: tangent and perpendicular (CONSTR-05).
- Re-solve and conflict reporting (CONSTR-06).
- *Accept:* a constrained rectangle stays rectangular when one corner moves; a conflicting constraint is reported, not silently ignored.

**M7. Projects.**
- Save and load `.contour` files (PROJ-01 to 03).
- Undo and redo across all edits (CANVAS-05).
- Autosave.
- *Accept:* save, close, reopen, and export produce identical WAV output.

**M8. VST3 and host sync.**
- Plugin build with host transport and tempo (PLUG-01 to 05).
- Plugin state saved in the DAW session.
- *Accept:* the plugin plays a drawing in a DAW on loop, follows host tempo, and saves its state with the session.

**M9. Polish.**
- Performance pass on large fills.
- Presets for shapes and micros.
- Keyboard shortcuts.
- *Accept:* a 30-second filled piece exports in under a minute on a typical Windows machine. This is a target, not a hard requirement.

---

## 10. Workflow

**Git (GitKraken):**
- `main` always builds and passes tests.
- One branch per milestone, for example `m1-engine`. Merge into `main` when the milestone's acceptance criteria pass.
- Small commits with messages that name the requirement ID, for example `CANVAS-02: add ellipse tool`.
- Tag each finished milestone, for example `v0.1-m1`.

**Coding harness (Claude Code or opencode):**
- The agent reads `AGENTS.md`, `docs/SPEC.md`, and `docs/DECISIONS.md` at the start of each session.
- The agent works on one milestone at a time and does not start the next without approval.
- The agent runs the test suite before reporting a task done.
- Any ambiguity goes to the user as a question, not an assumption. Any new decision is added to `DECISIONS.md` with a date.

**Agent rules (to put in AGENTS.md):**
1. Read PLAN.md, SPEC.md, and DECISIONS.md first.
2. Do not add features that are not in SPEC.md. Propose them in DECISIONS.md as "proposed" instead.
3. Keep the engine free of UI and plugin dependencies.
4. Keep the audio thread real-time safe.
5. Run tests and report results. Do not claim a test passes without running it.
6. Ask before changing any decision marked as confirmed.

---

## 11. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Filled regions are CPU-heavy | Slow preview, long exports | Lattice gating, density control, offline-first, preview quality setting |
| Fills sound harsh or beating | Poor sound | Phase policy options, density tuning, listening tests in M1 |
| Constraint solver integration | Schedule slip | Staged constraints, solver behind an interface, option to write a smaller solver |
| Rust learning curve | Slower progress | Milestone-by-milestone work, agent writes most code, tests catch regressions |
| nih-plug standalone or egui quirks on Windows | Delayed M0 | Spike first; JUCE is the fallback |
| Audio thread glitches in the plugin | Dropouts in DAW | Prepare buffers off the audio thread (PLUG-05) |
| Scope creep | Never finished | Milestones with acceptance criteria; SPEC.md is the boundary |

---

## 12. Open decisions

| # | Question | Default if no answer |
|---|---|---|
| 1 | Framework: Rust with nih-plug (recommended) or JUCE (C++)? | Rust with nih-plug |
| 2 | Keep the working name "Contour"? | Yes |
| 3 | Overlapping shapes at the same frequency: add or count once? | Add |
| 4 | Micro depth unit: semitones or Hz? | Semitones, with a Hz toggle |
| 5 | Micro scale: depth only, or depth and period? | Both, with separate controls |
| 6 | Vertical edge window length, and what does "sustain" mean? | 5 ms window; sustain holds the slice for a set time |
| 7 | Lattice density default | 5 cents |
| 8 | Phase policy default | Random, seeded |
| 9 | Fill compensation for loudness | Off |
| 10 | Lane max value | 200% with clip meter |
| 11 | Master default | 70% |
| 12 | Export default | 48 kHz, 24-bit |
| 13 | Envelope scope (per voice, per shape, or micro depth) | Per voice |
| 14 | Maximum timeline length | 60 seconds |
| 15 | Time units: seconds or beats as the primary display | Seconds, with beats shown |
| 16 | Constraint solver: adopt CADCraft's crate or write a smaller one | Adopt, if separable |
| 17 | Shape set for v1 | All seven listed in CANVAS-02 |
| 18 | Does the micro move the drawn edges visually, or only the output? | Output only |

---

## 13. Decisions log

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
| 2026-11 | Standalone first, VST3 second. Windows first. |
| 2026-11 | Constraints and 3D view are ideas, not v1 requirements. |
| 2026-11 | Export WAV files for DAW import. |
| 2026-11 | Use CADCraft as a reference for constraints, subject to M0 verification. |

---

## 14. Starter prompt for the coding harness

Paste this into the first session, after putting this file at `docs/PLAN.md`:

> Read `docs/PLAN.md` fully. We are building a Windows-first drawing-based synthesizer called Contour. Before writing code, create `AGENTS.md`, `CLAUDE.md` (which imports AGENTS.md), `docs/SPEC.md` (copy of the requirements from PLAN.md sections 3 and 4, with IDs), and `docs/DECISIONS.md` (copy of section 13 plus the open decisions from section 12). Then run the M0 spike from section 9: confirm the toolchain, build a nih-plug example as VST3, check the standalone build, and check whether the CADCraft constraints crate can be separated. Report findings and ask me to resolve any open decisions that block M1. Do not start M1 until I approve.


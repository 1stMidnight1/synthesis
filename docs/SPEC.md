# Synthesis: Specification

This is the boundary of the project. Features not listed here are not built; propose them in `DECISIONS.md` as "proposed" instead.
Source: `docs/PLAN.md` sections 3 and 4, with open items resolved per `docs/DECISIONS.md` (D-numbers in brackets).

Platform: macOS first, Windows later. Standalone app is the focus; the plugin (PLUG) is deferred.

---

## 1. Core model and rules

Each rule must have a test.

**R1. Sounding set.** At each time *t*, the frequencies sounding in a voice are determined by where its macro shapes meet the vertical line at *t*:
- **Outline:** each edge crossing is one frequency. A rectangle sounds two pitches between its left and right sides.
- **Filled shapes: removed (2026-11-18).** A closed shape sounds only its perimeter. Its interior is not a band.

Multiple shapes in one voice contribute together. Where shapes overlap at the same frequency, their contributions **add** [D-3].

**R2. Vertical edges.** A vertical line at time *t* produces a slice containing all frequencies in its span, all at once, as a short burst. The slice gets a time window, default 5 ms, configurable. It is not a sweep. "Sustain" means the window length can be extended to hold the slice for a set time [D-6].

**R3. Gaps are silence.** Where no shape crosses the vertical line, the voice is silent.

**R4. Micro transform.** The micro shape offsets the whole slice of its voice:

```
offset(t)   = depth * depth_scale * micro(frac(t / (period * period_scale)))   micro() in -1..+1
f_out(t)    = f_macro(t) * 2^(offset(t) / 12)                                  depth in semitones
f_out(t)    = f_macro(t) + offset(t)                                           depth in Hz
```

- `period` is in milliseconds. The micro cycle is locked to the timeline; it does not restart per shape.
- Depth unit defaults to semitones, with a Hz toggle [D-4].
- Depth scale and period scale are separate multipliers [D-5].
- Example: ±10 Hz at 450 Hz ≈ ±0.38 semitones (±38 cents).
- The micro affects audio output only; drawn edges are not moved visually [D-18].

**R5. Volume.** Each channel's gain is `lane_percent / 100`, linear. Lane values overlapping in time add. Each lane's summed value is clamped to 0–200%; the meter shows clipping [D-10].

**R6. Stereo.** Left and right carry identical partials and differ only in gain.

**R7. Mixing.** Voices sum linearly. Master gain (0–100%, default 70% [D-11]) is applied after the sum. dB is for display only.

**R8. Pitch scale.** Y axis modes: Semitones (log, note labels, default), Log Hz (log, Hz labels), Linear Hz. Notes are labeled for reference; drawing is not snapped to notes. A4 = 440 Hz by default, configurable.

**R9. Units.** Volume canvas uses percent (0–100 default view, up to 200). Time is in seconds as the primary display, with bars/beats shown [D-15]; grid tied to BPM.

**R10. Timeline length.** Maximum project length 60 seconds [D-14].

---

## 2. Requirements

### 2.1 Canvas and drawing (CANVAS)
- **CANVAS-01** Pitch-time canvas per voice with pan (middle-drag) and zoom (wheel).
- **CANVAS-02** Tools: Select/Move, Line, Polyline, Rectangle, Circle, Ellipse, Arc, Polygon, Delete [D-17].
- **CANVAS-03** ~~Fill tool: click inside a closed shape to toggle outline/filled.~~ Removed 2026-11-18 (filled shapes removed, see DECISIONS).
- **CANVAS-04** ~~Each shape shows its mode visually.~~ Removed with CANVAS-03; all shapes are outlines.
- **CANVAS-05** Undo and redo for every edit.
- **CANVAS-06** Shapes stored as vector geometry; rasterizing is display-only.
- **CANVAS-07** Shapes can be selected, moved, and resized with handles.

### 2.2 Constraints (CONSTR), staged
- **CONSTR-01** Snap to grid (toggleable).
- **CONSTR-02** Horizontal and vertical alignment between points and lines.
- **CONSTR-03** Coincident points, equal lengths, equal radii.
- **CONSTR-04** Dimensions: length, radius, angle, in semitones/Hz/seconds per active scale.
- **CONSTR-05** Tangent and perpendicular relations between lines, circles, arcs.
- **CONSTR-06** Re-solve after every edit; report conflicts and redundancies.
- **CONSTR-07** Solver behind a small, swappable interface. Implemented in-repo [D-16].

### 2.3 Grid and scale (GRID)
- **GRID-01** Grid toggleable.
- **GRID-02** Time grid from BPM: bar, beat, 1/8, 1/16, 1/32; seconds also available.
- **GRID-03** Y grid spacing configurable, units follow active pitch scale.
- **GRID-04** Note labels along the pitch axis (Semitones and Log Hz modes).
- **GRID-05** BPM editable in the app. (Plugin: follows host — deferred.)

### 2.4 Voices (VOICE)
- **VOICE-01** Create, rename, delete, duplicate voices.
- **VOICE-02** Each voice has a color, visibility toggle, mute toggle.
- **VOICE-03** Each voice: one micro shape, zero or more macro shapes, one volume lane pair.
- **VOICE-04** Multiple voices play at once; outputs sum.

### 2.5 Volume lanes (VOL)
- **VOL-01** Volume canvas per voice, percent on Y, time on X.
- **VOL-02** Left lane red, right lane blue, purple edits both.
- **VOL-03** Horizontal line = constant level. Tools: line, polyline, freehand.
- **VOL-04** Overlapping lane values add (R5).

### 2.6 Micro shape (MICRO)
- **MICRO-01** Editor: one cycle on 0..1 horizontal, -1..+1 vertical.
- **MICRO-02** Editable presets: sine, triangle, saw, square.
- **MICRO-03** Period in ms, numeric field.
- **MICRO-04** Depth with unit toggle: semitones or Hz.
- **MICRO-05** Depth scale and period scale multipliers.
- **MICRO-06** Micro is per voice; all its macro shapes use it.

### 2.7 Envelopes (ENV)
- **ENV-01** ADSR per voice, applied to voice output amplitude.
- **ENV-02** Envelope scope is per voice only in v1 [D-13].
- **ENV-03** Envelope shown on the volume canvas or a small panel.

### 2.8 Sound engine (ENGINE)
- **ENGINE-01** Offline render of the scene to a stereo buffer at a set sample rate.
- **ENGINE-02** Lattice synthesis (PLAN.md §5).
- **ENGINE-03** Density 1–100 cents, default 5 [D-7].
- **ENGINE-04** Phase policy: Aligned, Random (seeded, default) [D-8], Schroeder.
- **ENGINE-05** Click-free gating: ~2 ms ramp on oscillator on/off.
- **ENGINE-06** Offline export is reference quality. Live preview may use lower density and must say so.
- **ENGINE-07** Per-oscillator gain is density-compensated so changing density does not change loudness. No per-slice fill compensation in v1 [D-9].

### 2.9 Transport and playback (TRANS)
- **TRANS-01** Play, stop, playhead; click/drag ruler to scrub.
- **TRANS-02** Loop region.
- **TRANS-03** Time display in seconds and bars/beats.
- **TRANS-04** Engine runs on a background thread; UI stays responsive. Audio callback is real-time safe.

### 2.10 Output and export (OUT)
- **OUT-01** Master gain 0–100%, default 70%.
- **OUT-02** Stereo meter with peak and clip indicator.
- **OUT-03** Export WAV: 44.1/48 kHz; 16, 24-bit int or 32-bit float. Default 48 kHz, 24-bit [D-12].
- **OUT-04** Export range: whole project or loop region.
- **OUT-05** Optional normalize on export, off by default.

### 2.11 Master view (MASTER)
- **MASTER-01** All voices on one pitch-time canvas, each in its color.
- **MASTER-02** Per-voice visibility toggles.
- **MASTER-03** Volume strip beneath the pitch view.
- **MASTER-04** 2D only in v1.

### 2.12 Projects (PROJ)
- **PROJ-01** Save/load versioned JSON, extension `.synthesis`.
- **PROJ-02** Autosave to a separate recovery file.
- **PROJ-03** Settings (BPM, grid, scale, A4, density, export defaults) saved with the project.

### 2.13 Platform (PLAT)
- **PLAT-01** macOS (Apple Silicon) is the primary target.
- **PLAT-02** Windows (x86_64 MSVC) must remain buildable: no platform-specific code outside isolated, cfg-gated modules. CI checks both.
- **PLAT-03** File paths, config, and recovery locations go through a platform-neutral helper (e.g. `directories` crate).

### 2.14 Plugin (PLUG) — deferred
- **PLUG-01..05** As in PLAN.md §4.13. Not in current scope. The engine must remain free of UI/audio-backend dependencies so these stay possible.

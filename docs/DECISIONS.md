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

## Proposed

_None yet._

# Agent instructions: Synthesis

Synthesis is a drawing-based synthesizer: shapes on a pitch-vs-time canvas define which frequencies sound. Standalone app first (macOS, Windows later); a plugin is deferred.

## Start of every session
1. Read `docs/PLAN.md`, `docs/SPEC.md`, and `docs/DECISIONS.md`.
2. Check `git status` and the current branch. Work happens on the current milestone branch (e.g. `m1-engine`), never directly on `main`.

## Rules
1. Do not add features that are not in `docs/SPEC.md`. Propose them in `docs/DECISIONS.md` under "Proposed" instead.
2. Keep `crates/engine` free of UI, audio-backend, and plugin dependencies (no egui, eframe, cpal, nih-plug).
3. Keep the audio callback real-time safe: no allocation, no locks that can block, no file I/O, no logging.
4. Stay cross-platform (PLAT-02). Any OS-specific code goes in a small `cfg`-gated module. Use `std::path` / `directories` for paths, never hard-coded separators or locations.
5. Run tests and report results. Do not claim a test passes without running it.
6. Ask before changing any decision marked **confirmed**. Ambiguity goes to the user as a question, not an assumption. New decisions go in `DECISIONS.md` with a date.
7. Work one milestone at a time. Do not start the next milestone without user approval.

## Commands
- Build: `cargo build --workspace`
- Test: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all`
- Run app: `cargo run -p synthesis-app --release`
- Windows type-check (from macOS): `cargo check -p synthesis-engine --target x86_64-pc-windows-msvc`

## Git
- `main` always builds and passes tests.
- One branch per milestone (`m0-bootstrap`, `m1-engine`, ...). Merge to `main` when acceptance criteria pass; tag `v0.1-mN`.
- Small commits whose messages name the requirement ID, e.g. `ENGINE-02: add lattice oscillator bank`.
- Only commit, merge, tag, or push when the user asks or approves.

## Layout
```
crates/engine   geometry, model, synth, render, project (pure Rust)
crates/app      standalone eframe/egui app + cpal audio output
docs/           PLAN.md, SPEC.md, DECISIONS.md
tests/fixtures  sample .synthesis projects (added in M7)
```

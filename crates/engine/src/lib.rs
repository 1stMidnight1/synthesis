//! Synthesis engine.
//!
//! Pure Rust: no UI, audio-backend, or plugin dependencies (AGENTS.md rule 2).
//!
//! - `geometry`: the seven v1 shapes as vector data (CANVAS-02, CANVAS-06).
//! - `sampling`: slice sampling per R1 to R3.
//! - `model`: scene, voice, and settings (R10, ENGINE-03, ENGINE-04).
//! - `synth`: lattice bands and exact outline tones, with click-free gating.
//! - `render`: offline stereo render (ENGINE-01, R6, R7).
//! - `export`: WAV export at 16-bit, 24-bit, or 32-bit float (OUT-03).

pub mod error;
pub mod export;
pub mod geometry;
pub mod model;
pub mod osc;
pub mod pitch;
pub mod render;
pub mod sampling;
pub mod synth;

pub use error::{EngineError, Result};
pub use export::{BitDepth, write_wav, write_wav_file};
pub use geometry::{Geometry, Point, Segment, Shape};
pub use model::{PhasePolicy, Scene, Settings, Voice};
pub use render::{Rendered, render_scene};

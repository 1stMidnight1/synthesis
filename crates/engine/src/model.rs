//! Scene, voice, and settings model needed for offline rendering (M1).
//!
//! Micro shape, volume lanes, and envelopes arrive in M2 and M5. This model
//! holds only what M1 renders: shapes per voice, a mute flag, and settings.

use crate::error::{EngineError, Result};
use crate::geometry::Shape;
use crate::lanes::{DEFAULT_STROKE_PERCENT, Lane};
use crate::micro::Micro;

/// Maximum project length (R10, D-14).
pub const MAX_DURATION_S: f64 = 60.0;

/// Phase policy for lattice oscillators (ENGINE-04, D-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhasePolicy {
    /// All starting phases zero. Partials peak together and buzz.
    Aligned,
    /// Pseudo-random phases seeded by lattice index (default, D-8).
    #[default]
    Random,
    /// Quadratic (Schroeder) phase sequence for a flatter sum envelope.
    Schroeder,
}

/// Engine settings. Defaults follow docs/DECISIONS.md.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Render sample rate in Hz. Only 44 100 and 48 000 are supported (OUT-03).
    pub sample_rate: u32,
    /// Lattice spacing in cents, 1 to 100 (ENGINE-03, D-7).
    pub density_cents: f64,
    /// Lattice phase policy (ENGINE-04).
    pub phase_policy: PhasePolicy,
    /// A4 reference in Hz (R8).
    pub a4_hz: f64,
    /// Click-free gate ramp in milliseconds (ENGINE-05, about 2 ms).
    pub ramp_ms: f64,
    /// Vertical-edge burst window in milliseconds (R2, D-6).
    pub vertical_window_ms: f64,
    /// Master gain in 0..=1 (OUT-01, D-11).
    pub master_gain: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            density_cents: 5.0,
            phase_policy: PhasePolicy::Random,
            a4_hz: 440.0,
            ramp_ms: 2.0,
            vertical_window_ms: 5.0,
            master_gain: 0.7,
        }
    }
}

impl Settings {
    /// Reject values outside their documented ranges.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(EngineError::InvalidSettings(m.to_string()));
        if self.sample_rate != 44_100 && self.sample_rate != 48_000 {
            return bad("sample rate must be 44100 or 48000");
        }
        if !(1.0..=100.0).contains(&self.density_cents) {
            return bad("density must be 1 to 100 cents");
        }
        if !(self.a4_hz.is_finite() && self.a4_hz > 0.0) {
            return bad("A4 must be a positive frequency");
        }
        if !(self.ramp_ms.is_finite() && self.ramp_ms > 0.0 && self.ramp_ms <= 50.0) {
            return bad("ramp must be in (0, 50] ms");
        }
        if !(self.vertical_window_ms.is_finite()
            && self.vertical_window_ms > 0.0
            && self.vertical_window_ms <= 1000.0)
        {
            return bad("vertical window must be in (0, 1000] ms");
        }
        if !(0.0..=1.0).contains(&self.master_gain) {
            return bad("master gain must be 0 to 100%");
        }
        Ok(())
    }
}

/// One instrument: macro shapes, an optional micro transform, volume lanes,
/// and a mute flag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Voice {
    pub name: String,
    pub shapes: Vec<Shape>,
    /// Micro transform shared by all of this voice's shapes (R4, MICRO-06).
    /// `None` means no micro transform.
    pub micro: Option<Micro>,
    /// Left and right volume lanes (R5, R6). An empty lane is constant 100%.
    pub lane_left: Lane,
    pub lane_right: Lane,
    pub muted: bool,
}

impl Voice {
    /// A new voice. Both volume lanes start with the default purple stroke at
    /// 75% (confirmed 2026-11-18), so the voice sounds from creation. A voice
    /// whose strokes are later removed is silent, as an empty lane is.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            lane_left: Lane::flat(DEFAULT_STROKE_PERCENT, MAX_DURATION_S),
            lane_right: Lane::flat(DEFAULT_STROKE_PERCENT, MAX_DURATION_S),
            ..Self::default()
        }
    }

    pub fn with_shape(mut self, shape: Shape) -> Self {
        self.shapes.push(shape);
        self
    }

    pub fn with_micro(mut self, micro: Micro) -> Self {
        self.micro = Some(micro);
        self
    }
}

/// Everything needed to render: duration, voices, and settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub duration_s: f64,
    pub voices: Vec<Voice>,
    pub settings: Settings,
}

impl Scene {
    pub fn new(duration_s: f64, settings: Settings) -> Self {
        Self {
            duration_s,
            voices: Vec::new(),
            settings,
        }
    }

    /// Check duration (R10), settings, and every shape.
    pub fn validate(&self) -> Result<()> {
        if !(self.duration_s.is_finite() && self.duration_s > 0.0) {
            return Err(EngineError::InvalidScene(
                "duration must be positive".to_string(),
            ));
        }
        if self.duration_s > MAX_DURATION_S {
            return Err(EngineError::InvalidScene(format!(
                "duration {} s exceeds the {} s limit",
                self.duration_s, MAX_DURATION_S
            )));
        }
        self.settings.validate()?;
        for voice in &self.voices {
            for shape in &voice.shapes {
                shape.validate()?;
            }
            if let Some(micro) = &voice.micro {
                micro.validate()?;
            }
            voice.lane_left.validate()?;
            voice.lane_right.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(Settings::default().validate().is_ok());
        assert_eq!(Settings::default().density_cents, 5.0);
        assert_eq!(Settings::default().phase_policy, PhasePolicy::Random);
        assert_eq!(Settings::default().master_gain, 0.7);
    }

    #[test]
    fn rejects_out_of_range_settings() {
        let density = Settings {
            density_cents: 0.5,
            ..Settings::default()
        };
        assert!(density.validate().is_err());
        let rate = Settings {
            sample_rate: 96_000,
            ..Settings::default()
        };
        assert!(rate.validate().is_err());
        let gain = Settings {
            master_gain: 1.5,
            ..Settings::default()
        };
        assert!(gain.validate().is_err());
    }

    #[test]
    fn scene_enforces_sixty_second_limit() {
        assert!(Scene::new(60.0, Settings::default()).validate().is_ok());
        assert!(Scene::new(60.5, Settings::default()).validate().is_err());
        assert!(Scene::new(0.0, Settings::default()).validate().is_err());
    }
}

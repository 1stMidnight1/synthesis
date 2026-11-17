//! Micro shape: a pitch transform locked to the timeline (R4, MICRO-01 to 06).
//!
//! The micro is one cycle of a waveform, given as points on phase 0..1 with
//! values -1..+1, interpolated linearly. Its phase is `frac(t / period)` for
//! timeline time `t`. It is never restarted per shape (R4).
//!
//! `offset(t) = depth * depth_scale * micro(phase(t))`. The offset is in
//! semitones or Hz (D-4). The shift is applied to each frequency in
//! `shift_hz`: a multiplicative shift for semitones, an additive shift for Hz.

use crate::error::{EngineError, Result};

/// Unit of micro depth (MICRO-04, D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthUnit {
    /// Multiplicative shift: `f * 2^(offset / 12)` (default).
    #[default]
    Semitones,
    /// Additive shift: `f + offset`.
    Hz,
}

/// Starter cycles (MICRO-02). Presets are plain point lists and can be edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Sine,
    Triangle,
    Saw,
    Square,
}

/// A micro cycle with its timing and depth controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Micro {
    /// `(phase, value)` points, phase nondecreasing in `0..=1`, value in `-1..=1`.
    pub points: Vec<(f64, f64)>,
    /// Cycle length in milliseconds (MICRO-03).
    pub period_ms: f64,
    /// Depth before scaling, in `depth_unit` (MICRO-04).
    pub depth: f64,
    pub depth_unit: DepthUnit,
    /// Multiplier on depth (MICRO-05, D-5). Must be finite and non-negative.
    pub depth_scale: f64,
    /// Multiplier on period (MICRO-05, D-5). Must be finite and positive.
    pub period_scale: f64,
}

impl Micro {
    /// Build a micro from a starter preset.
    pub fn preset(preset: Preset, period_ms: f64, depth: f64, depth_unit: DepthUnit) -> Self {
        const N: usize = 64;
        let points: Vec<(f64, f64)> = match preset {
            Preset::Sine => (0..N)
                .map(|k| {
                    let x = k as f64 / N as f64;
                    (x, (std::f64::consts::TAU * x).sin())
                })
                .collect(),
            Preset::Triangle => vec![(0.0, -1.0), (0.5, 1.0), (1.0, -1.0)],
            Preset::Saw => vec![(0.0, -1.0), (1.0, 1.0)],
            Preset::Square => vec![(0.0, 1.0), (0.5, 1.0), (0.5, -1.0), (1.0, -1.0)],
        };
        Self {
            points,
            period_ms,
            depth,
            depth_unit,
            depth_scale: 1.0,
            period_scale: 1.0,
        }
    }

    /// Check ranges and point ordering.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(EngineError::InvalidSettings(format!("micro: {m}")));
        if self.points.len() < 2 {
            return bad("needs at least 2 points");
        }
        let mut last_x = 0.0;
        for &(x, y) in &self.points {
            if !(x.is_finite() && (0.0..=1.0).contains(&x) && x >= last_x) {
                return bad("phase must be finite, in 0..=1, and nondecreasing");
            }
            if !(y.is_finite() && (-1.0..=1.0).contains(&y)) {
                return bad("value must be finite and in -1..=1");
            }
            last_x = x;
        }
        if !(self.period_ms.is_finite() && self.period_ms > 0.0) {
            return bad("period must be positive");
        }
        if !(self.period_scale.is_finite() && self.period_scale > 0.0) {
            return bad("period scale must be positive");
        }
        if !self.depth.is_finite() {
            return bad("depth must be finite");
        }
        if !(self.depth_scale.is_finite() && self.depth_scale >= 0.0) {
            return bad("depth scale must be non-negative");
        }
        Ok(())
    }

    /// Point `k` of the wrapped cycle. Index 0 is the wrapped last point at
    /// phase `last - 1`, and index `n + 1` is the wrapped first point at
    /// phase `first + 1`. This lets interpolation cross the cycle boundary.
    fn ext(&self, k: usize) -> (f64, f64) {
        let n = self.points.len();
        if k == 0 {
            let (x, y) = self.points[n - 1];
            (x - 1.0, y)
        } else if k <= n {
            self.points[k - 1]
        } else {
            let (x, y) = self.points[0];
            (x + 1.0, y)
        }
    }

    /// Interpolated cycle value at phase `x` in `[0, 1)`.
    pub fn eval(&self, x: f64) -> f64 {
        let m = self.points.len() + 2;
        // Largest index i with ext(i).0 <= x. ext(0).0 <= 0 <= x and
        // ext(m-1).0 >= 1 > x, so i is in 0..=m-2.
        let (mut lo, mut hi) = (0usize, m);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.ext(mid).0 <= x {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let i = lo - 1;
        let (x0, y0) = self.ext(i);
        let (x1, y1) = self.ext(i + 1);
        let dx = x1 - x0;
        if dx <= 0.0 {
            return y1;
        }
        y0 + (x - x0) / dx * (y1 - y0)
    }

    /// Scaled micro offset at timeline time `t_s` seconds, in `depth_unit`.
    pub fn offset_at(&self, t_s: f64) -> f64 {
        let period_s = self.period_ms * 1e-3 * self.period_scale;
        let x = (t_s / period_s).rem_euclid(1.0);
        self.depth * self.depth_scale * self.eval(x)
    }
}

/// Apply a micro offset to a frequency (R4). `offset` is in `unit`.
pub fn shift_hz(freq_hz: f64, offset: f64, unit: DepthUnit) -> f64 {
    match unit {
        DepthUnit::Semitones => freq_hz * (offset / 12.0).exp2(),
        DepthUnit::Hz => freq_hz + offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} != {b} (tol {tol})");
    }

    #[test]
    fn presets_hit_expected_values() {
        let sine = Micro::preset(Preset::Sine, 10.0, 1.0, DepthUnit::Hz);
        close(sine.eval(0.25), 1.0, 1e-3);
        close(sine.eval(0.75), -1.0, 1e-3);
        let tri = Micro::preset(Preset::Triangle, 10.0, 1.0, DepthUnit::Hz);
        close(tri.eval(0.0), -1.0, 1e-12);
        close(tri.eval(0.5), 1.0, 1e-12);
        close(tri.eval(0.25), 0.0, 1e-12);
        let saw = Micro::preset(Preset::Saw, 10.0, 1.0, DepthUnit::Hz);
        close(saw.eval(0.5), 0.0, 1e-12);
        let sq = Micro::preset(Preset::Square, 10.0, 1.0, DepthUnit::Hz);
        close(sq.eval(0.25), 1.0, 1e-12);
        close(sq.eval(0.75), -1.0, 1e-12);
    }

    #[test]
    fn interpolation_wraps_across_cycle_boundary() {
        // Ramp from +1 at phase 0.9 down to -1 at phase 0.1 (through the wrap).
        let m = Micro {
            points: vec![(0.1, -1.0), (0.9, 1.0)],
            period_ms: 10.0,
            depth: 1.0,
            depth_unit: DepthUnit::Hz,
            depth_scale: 1.0,
            period_scale: 1.0,
        };
        m.validate().unwrap();
        // Between phase 0.9 (+1) and the wrapped 1.1 (-1): halfway is 0.5 at 0.95.
        close(m.eval(0.95), 0.5, 1e-12);
        close(m.eval(0.0), 0.0, 1e-12);
        close(m.eval(0.1), -1.0, 1e-12);
        close(m.eval(0.9), 1.0, 1e-12);
    }

    #[test]
    fn offset_is_locked_to_timeline() {
        let m = Micro::preset(Preset::Sine, 10.0, 10.0, DepthUnit::Hz);
        // Same phase one period later gives the same offset (no per-shape restart).
        close(m.offset_at(0.0025), m.offset_at(0.0125), 1e-9);
        close(m.offset_at(0.0025), m.offset_at(0.0025 + 3.0 * 0.010), 1e-9);
    }

    #[test]
    fn depth_and_period_scales_apply() {
        let base = Micro::preset(Preset::Sine, 10.0, 10.0, DepthUnit::Hz);
        let mut zero = base.clone();
        zero.depth_scale = 0.0;
        close(zero.offset_at(0.0025), 0.0, 1e-12);

        let mut slow = base.clone();
        slow.period_scale = 2.0;
        // A doubled period means the phase at t equals base phase at t/2.
        close(slow.offset_at(0.010), base.offset_at(0.005), 1e-9);

        let mut half = base.clone();
        half.depth_scale = 0.5;
        close(half.offset_at(0.0025), 0.5 * base.offset_at(0.0025), 1e-12);
    }

    #[test]
    fn shift_units() {
        close(shift_hz(450.0, 10.0, DepthUnit::Hz), 460.0, 1e-12);
        close(shift_hz(450.0, 0.0, DepthUnit::Semitones), 450.0, 1e-12);
        // Semitone depth of 1 st is a 2^(1/12) ratio.
        close(
            shift_hz(440.0, 1.0, DepthUnit::Semitones),
            440.0 * 2f64.powf(1.0 / 12.0),
            1e-9,
        );
        // About 0.38 st is ±10 Hz at 450 Hz (PLAN R4 worked example).
        close(shift_hz(450.0, 0.38, DepthUnit::Semitones), 460.0, 0.1);
    }

    #[test]
    fn validation_rejects_bad_micro() {
        let mut m = Micro::preset(Preset::Sine, 10.0, 1.0, DepthUnit::Hz);
        m.validate().unwrap();
        m.period_ms = 0.0;
        assert!(m.validate().is_err());
        m = Micro::preset(Preset::Sine, 10.0, 1.0, DepthUnit::Hz);
        m.points = vec![(0.0, 0.0)];
        assert!(m.validate().is_err());
        m = Micro::preset(Preset::Sine, 10.0, 1.0, DepthUnit::Hz);
        m.points = vec![(0.5, 0.0), (0.2, 0.0)];
        assert!(m.validate().is_err(), "phase must be nondecreasing");
        m = Micro::preset(Preset::Sine, 10.0, 1.0, DepthUnit::Hz);
        m.depth_scale = -1.0;
        assert!(m.validate().is_err());
    }
}

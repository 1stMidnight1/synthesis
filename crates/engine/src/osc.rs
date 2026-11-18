//! Phase-accumulating sine oscillator.
//!
//! The building block of the lattice (ENGINE-02). Real-time safe: no
//! allocation, no locking.

use std::f64::consts::TAU;

/// Sine of `x` for `x` in `[0, TAU)`, by symmetry reduction to
/// `[-pi/2, pi/2]` and a degree-15 odd Taylor polynomial.
///
/// Maximum error against `f64::sin` is below 1e-11 (about -220 dB), far under
/// the 24-bit export's quantisation step (about -144 dB). It is about 3 to 4
/// times faster than the libm call, which dominates fill rendering (M2.5).
#[inline]
pub fn fast_sin(x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    let mut y = if x > PI { x - TAU } else { x };
    if y > FRAC_PI_2 {
        y = PI - y;
    } else if y < -FRAC_PI_2 {
        y = -PI - y;
    }
    let y2 = y * y;
    y * (1.0
        + y2 * (-1.0 / 6.0
            + y2 * (1.0 / 120.0
                + y2 * (-1.0 / 5_040.0
                    + y2 * (1.0 / 362_880.0
                        + y2 * (-1.0 / 39_916_800.0
                            + y2 * (1.0 / 6_227_020_800.0 + y2 * (-1.0 / 1_307_674_368_000.0))))))))
}

/// A sine oscillator with a persistent phase.
#[derive(Debug, Clone, Copy)]
pub struct SineOsc {
    phase: f64,
}

impl SineOsc {
    /// Create an oscillator with the given starting phase in radians.
    pub fn new(phase: f64) -> Self {
        Self {
            phase: phase.rem_euclid(TAU),
        }
    }

    /// Current phase in radians, in `0..TAU`.
    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Output the current sample, then advance by one sample at `freq_hz`.
    #[inline]
    pub fn next(&mut self, freq_hz: f64, sample_rate: f64) -> f64 {
        self.next_step(TAU * freq_hz / sample_rate)
    }

    /// Output the current sample, then advance the phase by `step` radians.
    #[inline]
    pub fn next_step(&mut self, step: f64) -> f64 {
        let out = fast_sin(self.phase);
        self.phase += step;
        if self.phase >= TAU {
            self.phase -= TAU;
        }
        out
    }
}

impl Default for SineOsc {
    fn default() -> Self {
        Self::new(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_sin_matches_libm() {
        let mut worst = 0.0f64;
        for i in 0..1_000_000 {
            let x = TAU * i as f64 / 1_000_000.0;
            worst = worst.max((fast_sin(x) - x.sin()).abs());
        }
        assert!(worst < 1e-11, "max error {worst:e}");
    }

    #[test]
    fn completes_one_cycle_per_period() {
        let sr = 48_000.0;
        let mut osc = SineOsc::default();
        // 480 Hz at 48 kHz = 100 samples per cycle.
        for _ in 0..100 {
            osc.next(480.0, sr);
        }
        let p = osc.phase();
        assert!(p < 1e-9 || (TAU - p) < 1e-9, "phase drifted: {p}");
    }

    #[test]
    fn output_stays_in_range_and_starts_at_phase() {
        let mut osc = SineOsc::new(std::f64::consts::FRAC_PI_2);
        assert!((osc.next(1000.0, 48_000.0) - 1.0).abs() < 1e-11);
        for _ in 0..10_000 {
            let s = osc.next(1234.5, 48_000.0);
            assert!((-1.0..=1.0).contains(&s));
        }
    }
}

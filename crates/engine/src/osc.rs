//! Phase-accumulating sine oscillator.
//!
//! The building block of the lattice (ENGINE-02). Real-time safe: no
//! allocation, no locking.

use std::f64::consts::TAU;

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
        let out = self.phase.sin();
        self.phase += TAU * freq_hz / sample_rate;
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
        assert!((osc.next(1000.0, 48_000.0) - 1.0).abs() < 1e-12);
        for _ in 0..10_000 {
            let s = osc.next(1234.5, 48_000.0);
            assert!((-1.0..=1.0).contains(&s));
        }
    }
}

//! Per-voice synthesis (ENGINE-02 to ENGINE-05, ENGINE-07, R4).
//!
//! Two kinds of oscillator, chosen so that each acceptance test is met exactly
//! rather than approximately (see docs/DECISIONS.md, "M1 engine design"):
//!
//! - **Outline tones.** Each non-vertical edge owns one `SineOsc` whose
//!   frequency follows the edge's pitch at each sample. A flat 450 Hz line
//!   therefore sounds exactly 450 Hz, with no 5-cent quantization.
//! - **Lattice bands.** Filled regions and vertical-edge bursts use a fixed
//!   log-spaced lattice (ENGINE-02). A lattice oscillator is gated on when its
//!   macro pitch lies in an active band.
//!
//! Micro (R4, M2): every sounding frequency is shifted by the voice's micro
//! offset, evaluated per sample. Gating always uses the macro pitch, and only
//! the sound uses the shifted frequency.
//!
//! Phase (M2): every oscillator is a phase accumulator. Its phase advances only
//! while it is audible, so a moving or micro-shifted frequency stays continuous
//! and a re-gated oscillator resumes at its previous phase (ENGINE-05).
//!
//! Gating for the lattice is evaluated once per 1 ms control frame (PLAN §5.1).
//! Gains ramp linearly over `ramp_ms` (ENGINE-05).
//!
//! Level: each lattice oscillator has amplitude `BAND_GAIN * sqrt(d / 100)` for
//! density `d` cents (ENGINE-07).
//!
//! Real-time safety: the lattice is evaluated per sample, and gating allocates
//! at control rate. That is fine offline (M1–M2). Live playback will need
//! prepared buffers (AGENTS rule 3, PLUG-05), added in a later milestone.

use std::f64::consts::{PI, TAU};

use crate::geometry::Segment;
use crate::micro::{DepthUnit, shift_hz};
use crate::model::{PhasePolicy, Settings, Voice};
use crate::osc::SineOsc;
use crate::pitch::{AUDIBLE_MAX_HZ, AUDIBLE_MIN_HZ, hz_to_st, st_to_hz};
use crate::sampling::{is_vertical, slice_at, spans, st_at};

/// Base lattice amplitude before density compensation (tuned by ear later).
pub const BAND_GAIN: f64 = 0.5;

/// Gain of each exact outline tone.
pub const TONE_GAIN: f64 = 1.0;

/// Control-rate frame for lattice gating (PLAN §5.1, 1 ms).
const CONTROL_S: f64 = 0.001;

/// Fixed seed so renders are repeatable (D-8).
const PHASE_SEED: u64 = 0x5EED_CAFE_F00D_1234;

/// Upper limit on lattice frequency as a fraction of the sample rate.
const NYQUIST_FRACTION: f64 = 0.45;

/// SplitMix64 hash, used for seeded random phases.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Deterministic phase in `[0, TAU)` from a lattice index.
fn seeded_phase(index: u64) -> f64 {
    let bits = splitmix64(index ^ PHASE_SEED) >> 11;
    (bits as f64 / (1u64 << 53) as f64) * TAU
}

/// Starting phase for lattice oscillator `i` of `n` under a policy.
pub fn lattice_phase(policy: PhasePolicy, i: usize, n: usize) -> f64 {
    match policy {
        PhasePolicy::Aligned => 0.0,
        PhasePolicy::Random => seeded_phase(i as u64),
        PhasePolicy::Schroeder => {
            let x = i as f64;
            PI * x * x / (n.max(1) as f64)
        }
    }
}

/// Move `gain` toward `target` (0 or 1) by at most `step`.
fn ramp_toward(gain: f64, target: f64, step: f64) -> f64 {
    if target > gain {
        (gain + step).min(target)
    } else {
        (gain - step).max(target)
    }
}

/// Synth state for one voice. Produces a mono sample; L/R gains are applied by the renderer (R6).
pub struct VoiceSynth<'a> {
    voice: &'a Voice,
    sr: f64,
    a4_hz: f64,
    ramp_step: f64,
    control_samples: usize,
    burst_s: f64,
    amp_band: f64,
    // Outline tones: one per non-vertical edge of every shape.
    tone_segs: Vec<Segment>,
    tone_osc: Vec<SineOsc>,
    tone_gain: Vec<f64>,
    // Lattice bands.
    lat_st: Vec<f64>,
    lat_hz: Vec<f64>,
    lat_osc: Vec<SineOsc>,
    lat_gain: Vec<f64>,
    lat_target: Vec<f64>,
}

impl<'a> VoiceSynth<'a> {
    /// Prepare synth state for `voice`. `settings` must already be validated.
    pub fn new(voice: &'a Voice, settings: &Settings) -> Self {
        let sr = f64::from(settings.sample_rate);
        let ramp_samples = (settings.ramp_ms * 1e-3 * sr).max(1.0);

        let mut tone_segs = Vec::new();
        for shape in &voice.shapes {
            for seg in shape.edges() {
                if !is_vertical(&seg) {
                    tone_segs.push(seg);
                }
            }
        }
        let tone_count = tone_segs.len();

        // Lattice from AUDIBLE_MIN_HZ up, spaced by density in cents.
        let step = settings.density_cents / 100.0;
        let st_lo = hz_to_st(AUDIBLE_MIN_HZ, settings.a4_hz);
        let st_hi = hz_to_st(AUDIBLE_MAX_HZ.min(NYQUIST_FRACTION * sr), settings.a4_hz);
        let mut lat_st = Vec::new();
        let mut k = 0usize;
        loop {
            let st = st_lo + step * k as f64;
            if st > st_hi {
                break;
            }
            lat_st.push(st);
            k += 1;
        }
        let n = lat_st.len();
        let lat_hz: Vec<f64> = lat_st
            .iter()
            .map(|&st| st_to_hz(st, settings.a4_hz))
            .collect();
        let lat_osc: Vec<SineOsc> = (0..n)
            .map(|i| SineOsc::new(lattice_phase(settings.phase_policy, i, n)))
            .collect();

        Self {
            voice,
            sr,
            a4_hz: settings.a4_hz,
            ramp_step: 1.0 / ramp_samples,
            control_samples: ((CONTROL_S * sr).round() as usize).max(1),
            burst_s: settings.vertical_window_ms * 1e-3,
            amp_band: BAND_GAIN * (settings.density_cents / 100.0).sqrt(),
            tone_segs,
            tone_osc: vec![SineOsc::default(); tone_count],
            tone_gain: vec![0.0; tone_count],
            lat_st,
            lat_hz,
            lat_osc,
            lat_gain: vec![0.0; n],
            lat_target: vec![0.0; n],
        }
    }

    /// Number of lattice oscillators (exposed for tests and diagnostics).
    pub fn lattice_len(&self) -> usize {
        self.lat_st.len()
    }

    /// Re-evaluate lattice gate targets from the sounding set at `t`.
    fn update_lattice_targets(&mut self, t: f64) {
        let slice = slice_at(&self.voice.shapes, t, self.burst_s);
        for (i, &st) in self.lat_st.iter().enumerate() {
            let on = slice.bands.iter().any(|&(lo, hi)| lo <= st && st <= hi);
            self.lat_target[i] = if on { 1.0 } else { 0.0 };
        }
    }

    /// Mono sample at index `n` (time `n / sr`). Call with increasing `n`.
    pub fn process(&mut self, n: usize) -> f64 {
        let t = n as f64 / self.sr;
        if n.is_multiple_of(self.control_samples) {
            self.update_lattice_targets(t);
        }

        // Micro offset for this sample (R4). No micro means a zero offset.
        let (offset, unit) = match &self.voice.micro {
            Some(m) => (m.offset_at(t), m.depth_unit),
            None => (0.0, DepthUnit::Semitones),
        };

        let mut out = 0.0;

        // Outline tones: exact macro pitch, then the micro shift.
        for k in 0..self.tone_segs.len() {
            let seg = self.tone_segs[k];
            let target = if spans(&seg, t) { 1.0 } else { 0.0 };
            let g = ramp_toward(self.tone_gain[k], target, self.ramp_step);
            self.tone_gain[k] = g;
            if g > 0.0 {
                let macro_hz = st_to_hz(st_at(&seg, t), self.a4_hz);
                let f = shift_hz(macro_hz, offset, unit).max(0.0);
                out += TONE_GAIN * g * self.tone_osc[k].next(f, self.sr);
            }
        }

        // Lattice bands: gated on macro pitch, sounding at the shifted frequency.
        for i in 0..self.lat_st.len() {
            let g = ramp_toward(self.lat_gain[i], self.lat_target[i], self.ramp_step);
            self.lat_gain[i] = g;
            if g > 0.0 {
                let f = shift_hz(self.lat_hz[i], offset, unit).max(0.0);
                out += self.amp_band * g * self.lat_osc[i].next(f, self.sr);
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_monotonic_and_clamped() {
        let mut g = 0.0;
        for _ in 0..10 {
            g = ramp_toward(g, 1.0, 0.25);
        }
        assert_eq!(g, 1.0);
        for _ in 0..10 {
            g = ramp_toward(g, 0.0, 0.25);
        }
        assert_eq!(g, 0.0);
    }

    #[test]
    fn seeded_phases_are_repeatable_and_spread() {
        let a: Vec<f64> = (0..64).map(seeded_phase).collect();
        let b: Vec<f64> = (0..64).map(seeded_phase).collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|p| (0.0..TAU).contains(p)));
        let mean = a.iter().sum::<f64>() / a.len() as f64;
        assert!((mean - PI).abs() < 1.0, "phases not spread: mean {mean}");
    }

    #[test]
    fn aligned_policy_is_zero() {
        assert_eq!(lattice_phase(PhasePolicy::Aligned, 7, 100), 0.0);
    }

    #[test]
    fn lattice_size_follows_density() {
        let voice = Voice::new("v");
        let fine = VoiceSynth::new(
            &voice,
            &Settings {
                density_cents: 5.0,
                ..Settings::default()
            },
        );
        let coarse = VoiceSynth::new(
            &voice,
            &Settings {
                density_cents: 20.0,
                ..Settings::default()
            },
        );
        // About 2 393 oscillators at 5 cents over 20 Hz to 20 kHz.
        assert!(fine.lattice_len() > 2000 && fine.lattice_len() < 2600);
        let ratio = fine.lattice_len() as f64 / coarse.lattice_len() as f64;
        assert!((ratio - 4.0).abs() < 0.1, "ratio {ratio}");
    }
}

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

use crate::branch::{self, Branch};
use crate::geometry::Segment;
use crate::micro::{DepthUnit, shift_hz};
use crate::model::{PhasePolicy, Settings, Voice};
use crate::osc::SineOsc;
use crate::pitch::{AUDIBLE_MAX_HZ, AUDIBLE_MIN_HZ, hz_to_st, st_to_hz};
use crate::sampling::{is_vertical, spans, st_at, vertical_span};

/// Base lattice amplitude before density compensation.
///
/// Power per semitone of band is `BAND_GAIN^2 / 2`, independent of density
/// (ENGINE-07). With 0.29 a one-octave band has about the RMS of one outline
/// tone (0.707), so filled shapes and outlines sit at similar levels. Wider
/// bands are louder; master gain and the meter manage that (PLAN §5.4).
pub const BAND_GAIN: f64 = 0.29;

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
pub struct VoiceSynth {
    voice: Voice,
    sr: f64,
    a4_hz: f64,
    ramp_step: f64,
    /// Gate ramp for curve branches (differs only for `CurveSound::LongFade`).
    branch_ramp_step: f64,
    control_samples: usize,
    burst_s: f64,
    amp_band: f64,
    // Outline tones: one per non-vertical edge of every straight shape.
    tone_segs: Vec<Segment>,
    tone_osc: Vec<SineOsc>,
    tone_gain: Vec<f64>,
    // Curve branches: one continuous oscillator each (see branch.rs).
    curve_branches: Vec<Branch>,
    branch_osc: Vec<SineOsc>,
    branch_gain: Vec<f64>,
    // Lattice bands.
    lat_st: Vec<f64>,
    lat_hz: Vec<f64>,
    /// Phase step per sample at the macro pitch: `TAU * lat_hz / sr`.
    lat_step: Vec<f64>,
    lat_osc: Vec<SineOsc>,
    lat_gain: Vec<f64>,
    lat_target: Vec<f64>,
    /// Lattice indices that may be audible (gain or target above zero), as
    /// ascending, non-overlapping half-open ranges. Only these are processed
    /// per sample; the rest are silent and would contribute nothing. Ranges
    /// (not an index list) keep the inner loop contiguous, which is about 3x
    /// faster; ascending order keeps the sum in the same order as a full scan,
    /// so output is unchanged (M2.5).
    active: Vec<(u32, u32)>,
    /// For each `active` range, whether every gain and target in it is 1.0.
    /// Kept up to date by `process`, so the check is not repeated per sample.
    settled: Vec<bool>,
    /// Each shape's flattened edges and fill flag, computed once instead of
    /// on every control frame.
    shape_edges: Vec<(Vec<Segment>, bool)>,
    // Scratch buffers, reused every control frame so gating does not allocate.
    scratch_cross: Vec<f64>,
    scratch_bands: Vec<(f64, f64)>,
    scratch_active: Vec<(u32, u32)>,
    /// Re-evaluate lattice gates on the next sample even if it is not on a
    /// control-frame boundary (set after a seek).
    force_update: bool,
}

impl VoiceSynth {
    /// Prepare synth state for `voice`, taking ownership. `settings` must already be validated.
    pub fn owned(voice: Voice, settings: &Settings) -> Self {
        let sr = f64::from(settings.sample_rate);
        let ramp_samples = (settings.ramp_ms * 1e-3 * sr).max(1.0);

        // Straight edges (lines, polylines, polygons, rectangles) keep their
        // exact segments. Curves (circles, ellipses) become analytic branches:
        // one continuous oscillator per branch, with no vertices (branch.rs).
        let mut tone_segs = Vec::new();
        let mut curve_branches = Vec::new();
        let curve_sound = voice.curve_sound;
        for shape in &voice.shapes {
            if let Some(branches) = branch::branches(&shape.geometry) {
                curve_branches.extend(
                    branches
                        .into_iter()
                        .filter(|b| curve_sound.selects(b.is_upper())),
                );
                continue;
            }
            for seg in shape.edges() {
                if !is_vertical(&seg) {
                    tone_segs.push(seg);
                }
            }
        }
        let tone_count = tone_segs.len();
        let branch_count = curve_branches.len();
        let branch_ramp_samples = curve_sound
            .fade_ms()
            .map_or(ramp_samples, |ms| (ms * 1e-3 * sr).max(1.0));

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
        let lat_step: Vec<f64> = lat_hz.iter().map(|&f| TAU * f / sr).collect();
        let lat_osc: Vec<SineOsc> = (0..n)
            .map(|i| SineOsc::new(lattice_phase(settings.phase_policy, i, n)))
            .collect();
        let shape_edges: Vec<(Vec<Segment>, bool)> =
            voice.shapes.iter().map(|s| (s.edges(), s.filled)).collect();
        let max_edges = shape_edges.iter().map(|(e, _)| e.len()).max().unwrap_or(0);

        Self {
            voice,
            sr,
            a4_hz: settings.a4_hz,
            ramp_step: 1.0 / ramp_samples,
            branch_ramp_step: 1.0 / branch_ramp_samples,
            control_samples: ((CONTROL_S * sr).round() as usize).max(1),
            burst_s: settings.vertical_window_ms * 1e-3,
            amp_band: BAND_GAIN * (settings.density_cents / 100.0).sqrt(),
            tone_segs,
            tone_osc: vec![SineOsc::default(); tone_count],
            tone_gain: vec![0.0; tone_count],
            curve_branches,
            branch_osc: vec![SineOsc::default(); branch_count],
            branch_gain: vec![0.0; branch_count],
            lat_st,
            lat_hz,
            lat_step,
            lat_osc,
            lat_gain: vec![0.0; n],
            lat_target: vec![0.0; n],
            active: Vec::with_capacity(64),
            settled: Vec::with_capacity(64),
            shape_edges,
            scratch_cross: Vec::with_capacity(max_edges),
            scratch_bands: Vec::with_capacity(64),
            scratch_active: Vec::with_capacity(64),
            force_update: true,
        }
    }

    /// Prepare for a jump in the sample index (seek or loop). Phases and gains
    /// are kept, so audio stays continuous; gates follow the new time on the
    /// next sample and ramp as usual (ENGINE-05).
    pub fn note_jump(&mut self) {
        self.force_update = true;
    }

    /// Number of lattice oscillators (exposed for tests and diagnostics).
    pub fn lattice_len(&self) -> usize {
        self.lat_st.len()
    }

    /// Re-evaluate lattice gate targets from the sounding set at `t`.
    ///
    /// The bands are the same as `sampling::slice_at` computes (vertical-edge
    /// bursts from every shape, even-odd fill bands from filled shapes), but
    /// from cached edges and reused buffers. The lattice is sorted by pitch,
    /// so each band maps to a contiguous index range found by binary search,
    /// with the same `lo <= st <= hi` rule as before.
    fn update_lattice_targets(&mut self, t: f64) {
        let bands = &mut self.scratch_bands;
        bands.clear();
        for (edges, filled) in &self.shape_edges {
            let cross = &mut self.scratch_cross;
            cross.clear();
            for seg in edges {
                if is_vertical(seg) {
                    let t0 = seg.a.t;
                    if t0 <= t && t < t0 + self.burst_s {
                        bands.push(vertical_span(seg));
                    }
                } else if *filled && spans(seg, t) {
                    cross.push(st_at(seg, t));
                }
            }
            if *filled {
                cross
                    .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                for pair in cross.as_chunks::<2>().0 {
                    bands.push((pair[0], pair[1]));
                }
            }
        }

        // Targets: only active oscillators can have a non-zero target, so
        // clearing those clears them all.
        for &(a, b) in &self.active {
            self.lat_target[a as usize..b as usize].fill(0.0);
        }
        let next = &mut self.scratch_active;
        next.clear();
        // Still-ramping oscillators stay active: keep the parts of the old
        // ranges whose gain is above zero.
        for &(a, b) in &self.active {
            let mut run: Option<u32> = None;
            for i in a..b {
                if self.lat_gain[i as usize] > 0.0 {
                    run.get_or_insert(i);
                } else if let Some(r) = run.take() {
                    next.push((r, i));
                }
            }
            if let Some(r) = run {
                next.push((r, b));
            }
        }
        for &(lo, hi) in bands.iter() {
            let a = self.lat_st.partition_point(|&s| s < lo);
            let b = self.lat_st.partition_point(|&s| s <= hi);
            if a < b {
                self.lat_target[a..b].fill(1.0);
                next.push((a as u32, b as u32));
            }
        }
        // Merge into ascending, non-overlapping ranges.
        next.sort_unstable();
        let mut w = 0;
        for k in 0..next.len() {
            if w > 0 && next[k].0 <= next[w - 1].1 {
                next[w - 1].1 = next[w - 1].1.max(next[k].1);
            } else {
                next[w] = next[k];
                w += 1;
            }
        }
        next.truncate(w);
        std::mem::swap(&mut self.active, &mut self.scratch_active);
        // Recompute which ranges are fully settled at gain 1.
        self.settled.clear();
        for &(a, b) in &self.active {
            let (a, b) = (a as usize, b as usize);
            let s = self.lat_gain[a..b]
                .iter()
                .zip(&self.lat_target[a..b])
                .all(|(&g, &t)| g == 1.0 && t == 1.0);
            self.settled.push(s);
        }
    }

    /// Mono sample at index `n` (time `n / sr`). Call with increasing `n`.
    pub fn process(&mut self, n: usize) -> f64 {
        let t = n as f64 / self.sr;
        if self.force_update || n.is_multiple_of(self.control_samples) {
            self.force_update = false;
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

        // Curve branches: the pitch follows the curve continuously. The gate
        // ramps only at the branch's time ends, where the curve turns back.
        for k in 0..self.curve_branches.len() {
            let br = self.curve_branches[k];
            let inside = t >= br.t0 && t <= br.t1;
            let target = if inside { 1.0 } else { 0.0 };
            let g = ramp_toward(self.branch_gain[k], target, self.branch_ramp_step);
            self.branch_gain[k] = g;
            if g > 0.0 {
                let (st, _) = br.at(t.clamp(br.t0, br.t1));
                let macro_hz = st_to_hz(st, self.a4_hz);
                let f = shift_hz(macro_hz, offset, unit).max(0.0);
                out += TONE_GAIN * g * self.branch_osc[k].next(f, self.sr);
            }
        }

        // Lattice bands: gated on macro pitch, sounding at the shifted frequency.
        // Only oscillators that can be audible are visited, in index order.
        //
        // Fast path: with no micro the frequency is the macro pitch, so the
        // phase step is the precomputed `lat_step`; `max(0.0)` is a no-op on
        // positive lattice frequencies. The step is computed with the same
        // expression `SineOsc::next` uses, so output is unchanged.
        if self.voice.micro.is_none() {
            let mut band = 0.0;
            let (amp, ramp) = (self.amp_band, self.ramp_step);
            for (r, &(a, b)) in self.active.iter().enumerate() {
                let (a, b) = (a as usize, b as usize);
                let gains = &mut self.lat_gain[a..b];
                let targets = &self.lat_target[a..b];
                let oscs = &mut self.lat_osc[a..b];
                let steps = &self.lat_step[a..b];
                // Most samples, every oscillator in a band is settled at full
                // gain. Then there is nothing to ramp and nothing to skip, and
                // a branch-free loop runs about 3x faster than one that tests
                // each gain (the test stops the sines from pipelining). It is
                // the same arithmetic in the same order: `amp * 1.0 * x`.
                if self.settled[r] {
                    for j in 0..oscs.len() {
                        band += amp * gains[j] * oscs[j].next_step(steps[j]);
                    }
                    continue;
                }
                let mut all_settled = true;
                for j in 0..gains.len() {
                    let tgt = targets[j];
                    let mut g = gains[j];
                    if g != tgt {
                        g = ramp_toward(g, tgt, ramp);
                        gains[j] = g;
                    }
                    if g > 0.0 {
                        band += amp * g * oscs[j].next_step(steps[j]);
                    }
                    all_settled &= g == 1.0 && tgt == 1.0;
                }
                self.settled[r] = all_settled;
            }
            return out + band;
        }
        // The micro shift is the same for every lattice oscillator, so its
        // factor (semitones) or offset (Hz) is computed once per sample;
        // `shift_hz` computes exactly `f * factor` or `f + offset`.
        let (factor, add) = match unit {
            DepthUnit::Semitones => ((offset / 12.0).exp2(), 0.0),
            DepthUnit::Hz => (1.0, offset),
        };
        let semis = matches!(unit, DepthUnit::Semitones);
        let (amp, ramp, sr) = (self.amp_band, self.ramp_step, self.sr);
        for (r, &(a, b)) in self.active.iter().enumerate() {
            let (a, b) = (a as usize, b as usize);
            let gains = &mut self.lat_gain[a..b];
            let targets = &self.lat_target[a..b];
            let oscs = &mut self.lat_osc[a..b];
            let hz = &self.lat_hz[a..b];
            // Same frequency arithmetic as `shift_hz` and `SineOsc::next`.
            let freq = |h: f64| (if semis { h * factor } else { h + add }).max(0.0);
            if self.settled[r] {
                for j in 0..oscs.len() {
                    out += amp * gains[j] * oscs[j].next(freq(hz[j]), sr);
                }
                continue;
            }
            let mut all_settled = true;
            for j in 0..gains.len() {
                let g = ramp_toward(gains[j], targets[j], ramp);
                gains[j] = g;
                if g > 0.0 {
                    out += amp * g * oscs[j].next(freq(hz[j]), sr);
                }
                all_settled &= g == 1.0 && targets[j] == 1.0;
            }
            self.settled[r] = all_settled;
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
        let fine = VoiceSynth::owned(
            voice.clone(),
            &Settings {
                density_cents: 5.0,
                ..Settings::default()
            },
        );
        let coarse = VoiceSynth::owned(
            voice,
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

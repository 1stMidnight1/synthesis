//! Transport math: playhead, loop region, and time display (TRANS-01 to 03).
//!
//! Pure functions, no state beyond what they are given. The app owns the
//! playhead; this module keeps the arithmetic testable.

/// A loop region in seconds. `start_s < end_s`, both within the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopRegion {
    pub start_s: f64,
    pub end_s: f64,
}

impl LoopRegion {
    /// Create a loop, rejecting empty or non-finite ranges.
    pub fn new(start_s: f64, end_s: f64) -> Option<Self> {
        if start_s.is_finite() && end_s.is_finite() && start_s >= 0.0 && end_s > start_s {
            Some(Self { start_s, end_s })
        } else {
            None
        }
    }

    pub fn len_s(&self) -> f64 {
        self.end_s - self.start_s
    }

    /// Wrap a playhead time into the loop.
    pub fn wrap(&self, t_s: f64) -> f64 {
        if t_s < self.start_s {
            return t_s;
        }
        self.start_s + (t_s - self.start_s).rem_euclid(self.len_s())
    }
}

/// Tempo and meter. Bars are assumed to be 4/4 (TRANS-03).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    pub bpm: f64,
}

impl Tempo {
    pub const BEATS_PER_BAR: f64 = 4.0;

    /// Create a tempo. BPM must be in 20..=300.
    pub fn new(bpm: f64) -> Option<Self> {
        if bpm.is_finite() && (20.0..=300.0).contains(&bpm) {
            Some(Self { bpm })
        } else {
            None
        }
    }

    pub fn beat_s(&self) -> f64 {
        60.0 / self.bpm
    }

    /// Convert seconds to a 1-based `(bar, beat, sixteenth)` position.
    pub fn position(&self, t_s: f64) -> (u32, u32, u32) {
        let beats = (t_s / self.beat_s()).max(0.0);
        let bar = (beats / Self::BEATS_PER_BAR).floor() as u32 + 1;
        let beat_in_bar = (beats % Self::BEATS_PER_BAR).floor() as u32 + 1;
        let frac = beats.fract();
        let sixteenth = (frac * 4.0).floor() as u32 + 1;
        (bar, beat_in_bar, sixteenth)
    }
}

/// Grid subdivisions for the time axis (GRID-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subdivision {
    Bar,
    Beat,
    Eighth,
    Sixteenth,
    ThirtySecond,
    Seconds,
}

impl Subdivision {
    /// Grid step in seconds at a tempo.
    pub fn step_s(&self, tempo: Tempo) -> f64 {
        let beat = tempo.beat_s();
        match self {
            Self::Bar => beat * Tempo::BEATS_PER_BAR,
            Self::Beat => beat,
            Self::Eighth => beat / 2.0,
            Self::Sixteenth => beat / 4.0,
            Self::ThirtySecond => beat / 8.0,
            Self::Seconds => 1.0,
        }
    }
}

/// Playhead time after advancing `dt_s` seconds, looping if a loop is set.
/// Without a loop, the playhead is clamped at `duration_s`.
pub fn advance(t_s: f64, dt_s: f64, duration_s: f64, looping: Option<LoopRegion>) -> f64 {
    let next = t_s + dt_s;
    match looping {
        Some(region) => region.wrap(next),
        None => next.min(duration_s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_wraps_playhead() {
        let l = LoopRegion::new(1.0, 2.0).unwrap();
        assert!((l.wrap(2.25) - 1.25).abs() < 1e-12);
        assert!(
            (l.wrap(0.5) - 0.5).abs() < 1e-12,
            "before loop is untouched"
        );
        assert!((l.wrap(5.5) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn loop_rejects_empty_or_bad_range() {
        assert!(LoopRegion::new(2.0, 2.0).is_none());
        assert!(LoopRegion::new(2.0, 1.0).is_none());
        assert!(LoopRegion::new(-1.0, 1.0).is_none());
        assert!(LoopRegion::new(0.0, f64::NAN).is_none());
    }

    #[test]
    fn tempo_positions() {
        let t = Tempo::new(120.0).unwrap(); // beat = 0.5 s, bar = 2 s
        assert_eq!(t.position(0.0), (1, 1, 1));
        assert_eq!(t.position(0.5), (1, 2, 1));
        assert_eq!(t.position(2.0), (2, 1, 1));
        // 2.25 s is 4.5 beats: bar 2, beat 1, halfway through beat 1 is sixteenth 3.
        assert_eq!(t.position(2.25), (2, 1, 3));
    }

    #[test]
    fn tempo_rejects_out_of_range() {
        assert!(Tempo::new(10.0).is_none());
        assert!(Tempo::new(400.0).is_none());
        assert!(Tempo::new(f64::NAN).is_none());
    }

    #[test]
    fn subdivision_steps() {
        let t = Tempo::new(120.0).unwrap();
        assert!((Subdivision::Bar.step_s(t) - 2.0).abs() < 1e-12);
        assert!((Subdivision::Beat.step_s(t) - 0.5).abs() < 1e-12);
        assert!((Subdivision::Eighth.step_s(t) - 0.25).abs() < 1e-12);
        assert!((Subdivision::Sixteenth.step_s(t) - 0.125).abs() < 1e-12);
        assert!((Subdivision::ThirtySecond.step_s(t) - 0.0625).abs() < 1e-12);
        assert!((Subdivision::Seconds.step_s(t) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn advance_loops_or_clamps() {
        assert!((advance(0.9, 0.2, 10.0, None) - 1.1).abs() < 1e-12);
        assert!(
            (advance(9.9, 0.5, 10.0, None) - 10.0).abs() < 1e-12,
            "clamped"
        );
        let l = LoopRegion::new(1.0, 2.0);
        assert!((advance(1.9, 0.2, 10.0, l) - 1.1).abs() < 1e-9);
    }
}

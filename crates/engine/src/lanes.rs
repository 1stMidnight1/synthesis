//! Volume lanes (R5, R6, VOL-01 to VOL-04).
//!
//! Each voice has one lane per channel (left and right). A lane is a set of
//! strokes. Each stroke is a piecewise-linear curve in percent over time.
//!
//! Defaults (not confirmed in SPEC; recorded in DECISIONS.md):
//! - A lane with no strokes is a constant 100%.
//! - Once a lane has strokes, time outside every stroke contributes 0%, so the
//!   channel is silent there.
//! - Overlapping strokes add (VOL-04), and the sum is clamped to 0..=200% (R5).

use crate::error::{EngineError, Result};

/// Maximum summed lane value in percent (R5, D-10).
pub const LANE_MAX_PERCENT: f64 = 200.0;

/// Value of an empty lane in percent.
pub const LANE_DEFAULT_PERCENT: f64 = 100.0;

/// A point on a lane: time in seconds and value in percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LanePoint {
    pub t_s: f64,
    pub percent: f64,
}

impl LanePoint {
    pub const fn new(t_s: f64, percent: f64) -> Self {
        Self { t_s, percent }
    }
}

/// One stroke: a piecewise-linear curve. Times must be nondecreasing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stroke {
    pub points: Vec<LanePoint>,
}

impl Stroke {
    pub fn new(points: Vec<LanePoint>) -> Result<Self> {
        let s = Self { points };
        s.validate()?;
        Ok(s)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(EngineError::InvalidSettings(format!("lane: {m}")));
        if self.points.len() < 2 {
            return bad("a stroke needs at least 2 points");
        }
        let mut last_t = f64::NEG_INFINITY;
        for p in &self.points {
            if !(p.t_s.is_finite() && p.percent.is_finite()) {
                return bad("non-finite point");
            }
            if !(0.0..=LANE_MAX_PERCENT).contains(&p.percent) {
                return bad("percent must be in 0..=200");
            }
            if p.t_s < last_t {
                return bad("stroke times must be nondecreasing");
            }
            last_t = p.t_s;
        }
        Ok(())
    }

    /// Stroke value in percent at `t`, or `None` outside its time range.
    pub fn value_at(&self, t: f64) -> Option<f64> {
        let p = &self.points;
        let first = p.first()?;
        let last = p.last()?;
        if t < first.t_s || t > last.t_s {
            return None;
        }
        let idx = p.partition_point(|q| q.t_s <= t);
        if idx >= p.len() {
            return Some(last.percent);
        }
        if idx == 0 {
            return Some(first.percent);
        }
        let a = p[idx - 1];
        let b = p[idx];
        let dt = b.t_s - a.t_s;
        if dt <= 0.0 {
            return Some(b.percent);
        }
        Some(a.percent + (t - a.t_s) / dt * (b.percent - a.percent))
    }
}

/// One channel's volume lane: a set of strokes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lane {
    pub strokes: Vec<Stroke>,
}

impl Lane {
    pub fn validate(&self) -> Result<()> {
        self.strokes.iter().try_for_each(Stroke::validate)
    }

    /// Linear gain at `t` seconds (R5): `sum(stroke percent) / 100`, clamped to 0..=2.
    pub fn gain_at(&self, t: f64) -> f64 {
        if self.strokes.is_empty() {
            return LANE_DEFAULT_PERCENT / 100.0;
        }
        let sum: f64 = self.strokes.iter().filter_map(|s| s.value_at(t)).sum();
        (sum / 100.0).clamp(0.0, LANE_MAX_PERCENT / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(t0: f64, t1: f64, percent: f64) -> Stroke {
        Stroke::new(vec![
            LanePoint::new(t0, percent),
            LanePoint::new(t1, percent),
        ])
        .unwrap()
    }

    #[test]
    fn empty_lane_is_constant_unity() {
        let lane = Lane::default();
        assert_eq!(lane.gain_at(0.0), 1.0);
        assert_eq!(lane.gain_at(59.0), 1.0);
    }

    #[test]
    fn stroke_interpolates_linearly_and_is_silent_outside() {
        let s = Stroke::new(vec![LanePoint::new(1.0, 0.0), LanePoint::new(3.0, 100.0)]).unwrap();
        assert_eq!(s.value_at(0.5), None);
        assert_eq!(s.value_at(4.0), None);
        assert!((s.value_at(2.0).unwrap() - 50.0).abs() < 1e-12);
        assert_eq!(s.value_at(1.0), Some(0.0));
        assert_eq!(s.value_at(3.0), Some(100.0));
    }

    #[test]
    fn overlapping_strokes_add() {
        let lane = Lane {
            strokes: vec![flat(0.0, 1.0, 50.0), flat(0.0, 1.0, 50.0)],
        };
        assert!((lane.gain_at(0.5) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn summed_value_is_clamped_to_200_percent() {
        let lane = Lane {
            strokes: vec![
                flat(0.0, 1.0, 100.0),
                flat(0.0, 1.0, 100.0),
                flat(0.0, 1.0, 100.0),
            ],
        };
        assert!((lane.gain_at(0.5) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn gaps_between_strokes_are_silent() {
        let lane = Lane {
            strokes: vec![flat(0.0, 0.4, 100.0), flat(0.6, 1.0, 100.0)],
        };
        assert_eq!(lane.gain_at(0.5), 0.0);
        assert_eq!(lane.gain_at(0.2), 1.0);
    }

    #[test]
    fn validation_rejects_bad_strokes() {
        assert!(Stroke::new(vec![LanePoint::new(0.0, 50.0)]).is_err());
        assert!(Stroke::new(vec![LanePoint::new(0.0, 250.0), LanePoint::new(1.0, 0.0)]).is_err());
        assert!(Stroke::new(vec![LanePoint::new(1.0, 0.0), LanePoint::new(0.0, 0.0)]).is_err());
        assert!(
            Stroke::new(vec![
                LanePoint::new(0.0, f64::NAN),
                LanePoint::new(1.0, 0.0)
            ])
            .is_err()
        );
    }
}

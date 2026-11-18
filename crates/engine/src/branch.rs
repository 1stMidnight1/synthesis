//! Analytic branches of curved outlines (R1, smooth outline tones).
//!
//! A circle, ellipse, or arc is not sampled into straight edges here. It is
//! split at its time extremes into branches, each of which is a single-valued
//! function of time: at any time `t` a branch has at most one pitch. One
//! oscillator per branch then follows the curve continuously, the same way a
//! line's oscillator follows its straight edge. There are no vertices, so no
//! amplitude dips and no pitch corners at sample points (see DECISIONS, M3
//! polish).
//!
//! The fades are only at the branch ends: the leftmost and rightmost times,
//! where the curve turns back in time.

use std::f64::consts::TAU;

use crate::geometry::{Geometry, Point};

/// One single-valued piece of a curve: pitch as a function of time over
/// `[t0, t1]`. `angle` runs from `a0` to `a1` and parameterises the curve;
/// the curve point is `(cx + rx cos a, cy + ry sin a)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Branch {
    pub center: Point,
    pub rx_s: f64,
    pub ry_st: f64,
    pub a0: f64,
    pub a1: f64,
    /// Time range `[t0, t1]` this branch covers.
    pub t0: f64,
    pub t1: f64,
}

/// Fade length at the two turn points of a curve, in milliseconds.
///
/// Fixed at 15 ms. A 2 ms fade is an audible pop at the start and end of an
/// arc on headphones (confirmed by ear); 15 ms removes it. There is no
/// selector for this: the curve always sounds both of its branches together
/// with this fade.
pub const CURVE_FADE_MS: f64 = 15.0;

impl Branch {
    /// True if this branch is on the upper half of its curve (pitch above the centre).
    pub fn is_upper(&self) -> bool {
        (0.5 * (self.a0 + self.a1)).sin() >= 0.0
    }

    /// Pitch (semitones) at time `t`, which must lie in `[t0, t1]`, and the
    /// rate of change of pitch in semitones per second.
    ///
    /// For a branch `t = cx + rx cos a`, so `a = acos((t - cx) / rx)` on the
    /// upper half and `-acos(...)` on the lower. The pitch is
    /// `cy + ry sin a`, and `d(st)/dt` follows from the chain rule.
    pub fn at(&self, t: f64) -> (f64, f64) {
        let u = ((t - self.center.t) / self.rx_s).clamp(-1.0, 1.0);
        let a = u.acos();
        // Branch sign: which half of the ellipse this branch is.
        let mid = 0.5 * (self.a0 + self.a1);
        let sign = if (mid.sin()) >= 0.0 { 1.0 } else { -1.0 };
        let sin_a = (1.0 - u * u).max(0.0).sqrt();
        let st = self.center.st + self.ry_st * sign * sin_a;
        // dt/da = -rx sin a; d(st)/da = ry sign cos a. Clamp the slope where
        // the branch turns (sin a -> 0) so the rate stays finite.
        let dt_da = -self.rx_s * a.sin();
        let dst_da = self.ry_st * sign * a.cos();
        let rate = if dt_da.abs() < 1e-12 {
            0.0
        } else {
            dst_da / dt_da
        };
        (st, rate)
    }
}

/// Split a closed ellipse (or circle) into its upper and lower branches,
/// which meet at the left and right extremes.
pub fn ellipse_branches(center: Point, rx_s: f64, ry_st: f64) -> Vec<Branch> {
    let (t_left, t_right) = (center.t - rx_s, center.t + rx_s);
    vec![
        // Upper half: angles 0..pi (from right to left).
        Branch {
            center,
            rx_s,
            ry_st,
            a0: 0.0,
            a1: std::f64::consts::PI,
            t0: t_left,
            t1: t_right,
        },
        // Lower half: angles pi..2pi (from left to right).
        Branch {
            center,
            rx_s,
            ry_st,
            a0: std::f64::consts::PI,
            a1: TAU,
            t0: t_left,
            t1: t_right,
        },
    ]
}

/// Branches of an open elliptical arc over its sweep. The sweep is split at
/// the angles where the curve reaches its time extremes (cos a = ±1), so each
/// piece is single-valued in time.
pub fn arc_branches(center: Point, rx_s: f64, ry_st: f64, start: f64, sweep: f64) -> Vec<Branch> {
    let (lo, hi) = (start.min(start + sweep), start.max(start + sweep));
    // Extremes in time occur at multiples of pi.
    let mut cuts = vec![lo];
    let mut k = (lo / std::f64::consts::PI).floor() as i64;
    loop {
        let a = k as f64 * std::f64::consts::PI;
        if a > hi {
            break;
        }
        if a > lo {
            cuts.push(a);
        }
        k += 1;
    }
    cuts.push(hi);
    cuts.windows(2)
        .filter(|w| w[1] - w[0] > 1e-12)
        .map(|w| {
            let (a0, a1) = (w[0], w[1]);
            let ta = center.t + rx_s * a0.cos();
            let tb = center.t + rx_s * a1.cos();
            Branch {
                center,
                rx_s,
                ry_st,
                a0,
                a1,
                t0: ta.min(tb),
                t1: ta.max(tb),
            }
        })
        .collect()
}

/// Branches of a curved geometry, or `None` for geometry that is already
/// straight (lines, polylines, polygons, rectangles), which keeps its exact
/// edges.
pub fn branches(g: &Geometry) -> Option<Vec<Branch>> {
    match *g {
        Geometry::Circle {
            center,
            radius_s,
            radius_st,
        } => Some(ellipse_branches(center, radius_s, radius_st)),
        Geometry::Ellipse {
            center,
            rx_s,
            ry_st,
        } => Some(ellipse_branches(center, rx_s, ry_st)),
        Geometry::Arc {
            center,
            rx_s,
            ry_st,
            start_rad,
            sweep_rad,
        } => Some(arc_branches(center, rx_s, ry_st, start_rad, sweep_rad)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;
    use std::f64::consts::PI;

    fn c() -> Point {
        Point::new(1.0, 69.0)
    }

    /// Each branch is single-valued in time: sampling it densely, time is
    /// strictly increasing with the angle, so one time gives one pitch.
    #[test]
    fn ellipse_branches_cover_the_curve() {
        let b = ellipse_branches(c(), 0.5, 6.0);
        assert_eq!(b.len(), 2);
        // Upper branch at the centre time is the top of the ellipse.
        let (st_up, _) = b[0].at(1.0);
        let (st_dn, _) = b[1].at(1.0);
        assert!((st_up - 75.0).abs() < 1e-9, "top {st_up}");
        assert!((st_dn - 63.0).abs() < 1e-9, "bottom {st_dn}");
    }

    /// The pitch of a branch matches the curve point it came from, everywhere.
    #[test]
    fn branch_pitch_lies_on_the_ellipse() {
        for br in ellipse_branches(c(), 0.5, 6.0) {
            for i in 1..50 {
                let t = br.t0 + (br.t1 - br.t0) * i as f64 / 50.0;
                let (st, _) = br.at(t);
                let u = (t - 1.0) / 0.5;
                let on_curve = 69.0 + 6.0 * (1.0 - u * u).sqrt();
                let dev = (st - 69.0).abs() - (on_curve - 69.0).abs();
                assert!(dev.abs() < 1e-9, "t={t}: {st} vs {on_curve}");
            }
        }
    }

    /// Slope check: the analytic rate matches a finite difference.
    #[test]
    fn rate_matches_finite_difference() {
        for br in ellipse_branches(c(), 0.5, 6.0) {
            for i in 2..48 {
                let t = br.t0 + (br.t1 - br.t0) * i as f64 / 50.0;
                let h = 1e-6;
                let (s0, _) = br.at(t - h);
                let (s1, _) = br.at(t + h);
                let (_, r) = br.at(t);
                let fd = (s1 - s0) / (2.0 * h);
                assert!(
                    (r - fd).abs() < 1e-3 * (1.0 + fd.abs()),
                    "t={t}: {r} vs {fd}"
                );
            }
        }
    }

    /// An arc splits into pieces that each cover a time range without folding back.
    #[test]
    fn arc_branches_are_single_valued_and_cover_the_sweep() {
        // A half-ellipse from the right end, over the top, to the left end.
        let b = arc_branches(c(), 0.5, 6.0, 0.0, PI);
        assert_eq!(b.len(), 1, "a half-turn needs no split");
        assert!((b[0].t0 - 0.5).abs() < 1e-9 && (b[0].t1 - 1.5).abs() < 1e-9);
        // A three-quarter arc crosses one time extreme and splits in two.
        let b = arc_branches(c(), 0.5, 6.0, 0.0, 1.5 * PI);
        assert!(b.len() >= 2, "{} pieces", b.len());
        for br in &b {
            for i in 1..20 {
                let t = br.t0 + (br.t1 - br.t0) * i as f64 / 20.0;
                let (st, _) = br.at(t);
                let u = (t - 1.0) / 0.5;
                let on = 69.0 + 6.0 * (1.0 - u * u).sqrt();
                // The lower half of a piece sits below the centre.
                assert!(
                    ((st - 69.0).abs() - (on - 69.0).abs()).abs() < 1e-6,
                    "t={t}"
                );
            }
        }
    }

    #[test]
    fn upper_and_lower_branches_are_identified() {
        let b = ellipse_branches(c(), 0.5, 6.0);
        assert!(b[0].is_upper() && !b[1].is_upper());
        let arc = arc_branches(c(), 0.5, 6.0, 0.0, 1.5 * PI);
        assert!(arc.iter().any(|x| x.is_upper()) && arc.iter().any(|x| !x.is_upper()));
    }

    #[test]
    fn curve_fade_is_fixed_at_15_ms() {
        assert_eq!(CURVE_FADE_MS, 15.0);
    }

    #[test]
    fn straight_geometry_has_no_branches() {
        let line = Geometry::Line {
            a: c(),
            b: Point::new(2.0, 70.0),
        };
        assert!(branches(&line).is_none());
    }
}

//! Slice sampling: which frequencies sound at time `t` (R1 to R3).
//!
//! - Outline crossings: every non-vertical edge spanning `t` contributes one
//!   tone at its crossing pitch. Contributions from different shapes add,
//!   even at the same pitch (D-3), so duplicate pitches are kept.
//! - Filled regions: crossings of a filled shape are paired even-odd into
//!   bands; everything between a pair sounds (R1). The perimeter still sounds
//!   as tones.
//! - Vertical edges: a vertical edge at `t0` sounds its whole pitch span as a
//!   band during `[t0, t0 + window)` (R2). This is a burst, not a sweep.
//! - Anywhere no shape is crossed, the slice is empty (R3).
//!
//! Half-open intervals (`t0 <= t < t1`) make shared corners count once.

use crate::geometry::{Segment, Shape};

/// Two times this close are treated as the same instant for vertical edges.
const VERTICAL_EPS_S: f64 = 1e-12;

/// The sounding set at one instant. Pitches are fractional MIDI semitones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slice {
    /// Tone contributions, one entry each (duplicates add).
    pub tones: Vec<f64>,
    /// Filled intervals `(lo, hi)` with `lo <= hi`.
    pub bands: Vec<(f64, f64)>,
}

impl Slice {
    /// True when nothing sounds (R3).
    pub fn is_silent(&self) -> bool {
        self.tones.is_empty() && self.bands.is_empty()
    }
}

/// True when the segment has no time extent and is a vertical edge.
pub fn is_vertical(seg: &Segment) -> bool {
    (seg.b.t - seg.a.t).abs() <= VERTICAL_EPS_S
}

fn time_range(seg: &Segment) -> (f64, f64) {
    if seg.a.t <= seg.b.t {
        (seg.a.t, seg.b.t)
    } else {
        (seg.b.t, seg.a.t)
    }
}

/// Whether a non-vertical segment spans `t` (half-open).
pub fn spans(seg: &Segment, t: f64) -> bool {
    let (t0, t1) = time_range(seg);
    t0 <= t && t < t1
}

/// Pitch of a non-vertical segment at `t`, clamped to the segment's time range.
pub fn st_at(seg: &Segment, t: f64) -> f64 {
    let (lo, hi) = if seg.a.t <= seg.b.t {
        (seg.a, seg.b)
    } else {
        (seg.b, seg.a)
    };
    let u = ((t - lo.t) / (hi.t - lo.t)).clamp(0.0, 1.0);
    lo.st + u * (hi.st - lo.st)
}

/// Lowest and highest pitch of a vertical segment.
pub fn vertical_span(seg: &Segment) -> (f64, f64) {
    (seg.a.st.min(seg.b.st), seg.a.st.max(seg.b.st))
}

/// Sounding set of a voice's shapes at time `t` (R1 to R3).
///
/// `burst_window_s` is the vertical-edge window (R2, default 5 ms).
pub fn slice_at(shapes: &[Shape], t: f64, burst_window_s: f64) -> Slice {
    let mut slice = Slice::default();
    for shape in shapes {
        let edges = shape.edges();
        let mut crossings: Vec<f64> = Vec::new();
        for seg in &edges {
            if is_vertical(seg) {
                let t0 = seg.a.t;
                if t0 <= t && t < t0 + burst_window_s {
                    let (lo, hi) = vertical_span(seg);
                    slice.bands.push((lo, hi));
                }
            } else if spans(seg, t) {
                crossings.push(st_at(seg, t));
            }
        }
        slice.tones.extend_from_slice(&crossings);
        if shape.filled {
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in crossings.as_chunks::<2>().0 {
                slice.bands.push((pair[0], pair[1]));
            }
        }
    }
    slice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Geometry, Point};
    use crate::pitch::hz_to_st;

    const A4: f64 = 440.0;
    const W: f64 = 0.005;

    fn st(hz: f64) -> f64 {
        hz_to_st(hz, A4)
    }

    fn rect(t0: f64, t1: f64, lo_hz: f64, hi_hz: f64, filled: bool) -> Shape {
        let g = Geometry::Rectangle {
            a: Point::new(t0, st(lo_hz)),
            b: Point::new(t1, st(hi_hz)),
        };
        Shape::new(g, filled).unwrap()
    }

    fn line(t0: f64, t1: f64, hz: f64) -> Shape {
        Shape::outline(Geometry::Line {
            a: Point::new(t0, st(hz)),
            b: Point::new(t1, st(hz)),
        })
        .unwrap()
    }

    fn vline(t: f64, lo_hz: f64, hi_hz: f64) -> Shape {
        Shape::outline(Geometry::Line {
            a: Point::new(t, st(lo_hz)),
            b: Point::new(t, st(hi_hz)),
        })
        .unwrap()
    }

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    /// R1: an outline rectangle sounds two pitches between its sides.
    #[test]
    fn r1_outline_rectangle_sounds_two_tones() {
        let shapes = [rect(0.0, 1.0, 200.0, 400.0, false)];
        let s = slice_at(&shapes, 0.5, W);
        assert_eq!(s.tones.len(), 2);
        assert!(s.bands.is_empty());
        let mut tones = s.tones.clone();
        tones.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_close(tones[0], st(200.0));
        assert_close(tones[1], st(400.0));
    }

    /// R1: a filled rectangle sounds the band between its edges.
    #[test]
    fn r1_filled_rectangle_sounds_band() {
        let shapes = [rect(0.0, 1.0, 200.0, 400.0, true)];
        let s = slice_at(&shapes, 0.5, W);
        assert_eq!(s.bands.len(), 1);
        let (lo, hi) = s.bands[0];
        assert_close(lo, st(200.0));
        assert_close(hi, st(400.0));
        // Perimeter still sounds as tones.
        assert_eq!(s.tones.len(), 2);
    }

    /// R1: overlapping contributions add, so the same pitch counts twice (D-3).
    #[test]
    fn r1_overlapping_contributions_add() {
        let shapes = [line(0.0, 1.0, 450.0), line(0.0, 1.0, 450.0)];
        let s = slice_at(&shapes, 0.5, W);
        assert_eq!(s.tones.len(), 2);
        assert_close(s.tones[0], s.tones[1]);
    }

    /// R1: a sloped line sounds the pitch at its crossing time.
    #[test]
    fn r1_sloped_line_pitch_follows_crossing() {
        let g = Geometry::Line {
            a: Point::new(0.0, st(200.0)),
            b: Point::new(1.0, st(400.0)),
        };
        let shapes = [Shape::outline(g).unwrap()];
        let s = slice_at(&shapes, 0.5, W);
        assert_eq!(s.tones.len(), 1);
        assert_close(s.tones[0], (st(200.0) + st(400.0)) / 2.0);
    }

    /// R2: a vertical line sounds its whole span at once, during a short window.
    #[test]
    fn r2_vertical_line_is_a_short_burst() {
        let shapes = [vline(0.5, 200.0, 400.0)];
        let inside = slice_at(&shapes, 0.5 + 0.002, W);
        assert_eq!(inside.bands.len(), 1);
        let (lo, hi) = inside.bands[0];
        assert_close(lo, st(200.0));
        assert_close(hi, st(400.0));
        assert!(inside.tones.is_empty(), "vertical edge is not a sweep");

        let after = slice_at(&shapes, 0.5 + W + 1e-6, W);
        assert!(after.is_silent(), "burst must end after the window");
        let before = slice_at(&shapes, 0.5 - 1e-6, W);
        assert!(before.is_silent(), "burst must not start early");
    }

    /// R3: where no shape is crossed, the slice is silent.
    #[test]
    fn r3_gap_is_silent() {
        let shapes = [line(0.0, 0.4, 450.0), line(0.6, 0.8, 300.0)];
        assert!(slice_at(&shapes, 0.5, W).is_silent());
        assert!(!slice_at(&shapes, 0.1, W).is_silent());
        assert!(slice_at(&[], 0.1, W).is_silent());
    }

    /// Shared corners count once: a line ending where another begins has one tone.
    #[test]
    fn half_open_endpoints_count_once() {
        let shapes = [line(0.0, 0.5, 450.0), line(0.5, 1.0, 300.0)];
        assert_eq!(slice_at(&shapes, 0.5, W).tones.len(), 1);
    }
}

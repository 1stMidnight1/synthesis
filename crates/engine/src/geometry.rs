//! Vector geometry for the seven v1 shapes (CANVAS-02, CANVAS-06).
//!
//! Coordinates are `Point { t, st }`: `t` is seconds on the timeline and `st`
//! is a fractional MIDI note number (see `pitch.rs`). Shapes are stored as
//! vectors and only flattened to edges here, for slice sampling (R1 to R3).
//!
//! Curves (circle, ellipse, arc) are sampled as polylines with
//! `CURVE_SEGMENTS_PER_TURN` segments per full turn. Their radii are given per
//! axis: `*_s` in seconds, `*_st` in semitones. The UI supplies radii that look
//! round at the current view. The engine does not assume an aspect ratio.

use std::f64::consts::TAU;

use crate::error::{EngineError, Result};

/// Polyline segments used to approximate a full curve turn.
/// 64 segments: each edge is audible for about 31 ms and changes pitch by at
/// most about 0.6 semitones, so a curve sounds as a smooth outline of steady
/// pitches. 256 segments (the M1 value) gave 8 ms edges that were too short to
/// hold a pitch, and the 2 ms gate ramps then dominated, which sounded grainy.
pub const CURVE_SEGMENTS_PER_TURN: usize = 64;

/// A point in (time seconds, fractional MIDI semitone) space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub t: f64,
    pub st: f64,
}

impl Point {
    pub const fn new(t: f64, st: f64) -> Self {
        Self { t, st }
    }
}

/// One straight edge of a shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub a: Point,
    pub b: Point,
}

/// The geometry of a shape, before any fill mode is applied.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// Open straight line from `a` to `b`.
    Line { a: Point, b: Point },
    /// Open chain of straight segments through `points` (at least 2).
    Polyline { points: Vec<Point> },
    /// Axis-aligned rectangle between opposite corners `a` and `b`.
    Rectangle { a: Point, b: Point },
    /// Circle with a per-axis radius (time seconds, pitch semitones).
    Circle {
        center: Point,
        radius_s: f64,
        radius_st: f64,
    },
    /// Axis-aligned ellipse with per-axis radii.
    Ellipse {
        center: Point,
        rx_s: f64,
        ry_st: f64,
    },
    /// Open elliptical arc. Angles are in radians and counterclockwise.
    Arc {
        center: Point,
        rx_s: f64,
        ry_st: f64,
        start_rad: f64,
        sweep_rad: f64,
    },
    /// Closed polygon through `points` (at least 3). The last point connects to the first.
    Polygon { points: Vec<Point> },
}

/// A drawn shape: its geometry. Every shape is an outline; a closed shape
/// sounds only its perimeter (R1). Filled shapes were removed (M-remove-filled).
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub geometry: Geometry,
}

fn finite(p: &Point) -> bool {
    p.t.is_finite() && p.st.is_finite()
}

fn bad(msg: &str) -> EngineError {
    EngineError::InvalidShape(msg.to_string())
}

impl Shape {
    /// Create a shape and validate it.
    pub fn outline(geometry: Geometry) -> Result<Self> {
        let shape = Self { geometry };
        shape.validate()?;
        Ok(shape)
    }

    /// Whether the geometry encloses a region (and so can be filled).
    pub fn is_closed(&self) -> bool {
        matches!(
            self.geometry,
            Geometry::Rectangle { .. }
                | Geometry::Circle { .. }
                | Geometry::Ellipse { .. }
                | Geometry::Polygon { .. }
        )
    }

    /// Check finiteness, counts, radii, and sweep.
    pub fn validate(&self) -> Result<()> {
        match &self.geometry {
            Geometry::Line { a, b } | Geometry::Rectangle { a, b } => {
                if !finite(a) || !finite(b) {
                    return Err(bad("non-finite coordinate"));
                }
            }
            Geometry::Polyline { points } => {
                if points.len() < 2 {
                    return Err(bad("polyline needs at least 2 points"));
                }
                if !points.iter().all(finite) {
                    return Err(bad("non-finite coordinate"));
                }
            }
            Geometry::Polygon { points } => {
                if points.len() < 3 {
                    return Err(bad("polygon needs at least 3 points"));
                }
                if !points.iter().all(finite) {
                    return Err(bad("non-finite coordinate"));
                }
            }
            Geometry::Circle {
                center,
                radius_s,
                radius_st,
            } => {
                check_radii(center, *radius_s, *radius_st)?;
            }
            Geometry::Ellipse {
                center,
                rx_s,
                ry_st,
            } => {
                check_radii(center, *rx_s, *ry_st)?;
            }
            Geometry::Arc {
                center,
                rx_s,
                ry_st,
                start_rad,
                sweep_rad,
            } => {
                check_radii(center, *rx_s, *ry_st)?;
                if !start_rad.is_finite() || !sweep_rad.is_finite() {
                    return Err(bad("non-finite arc angle"));
                }
                if *sweep_rad == 0.0 || sweep_rad.abs() > TAU {
                    return Err(bad("arc sweep must be non-zero and at most one turn"));
                }
            }
        }
        Ok(())
    }

    /// The vertices of the shape, and whether the last connects back to the first.
    pub fn vertices(&self) -> (Vec<Point>, bool) {
        match &self.geometry {
            Geometry::Line { a, b } => (vec![*a, *b], false),
            Geometry::Polyline { points } => (points.clone(), false),
            Geometry::Rectangle { a, b } => (
                vec![*a, Point::new(b.t, a.st), *b, Point::new(a.t, b.st)],
                true,
            ),
            Geometry::Circle {
                center,
                radius_s,
                radius_st,
            } => (
                ellipse_points(*center, *radius_s, *radius_st, 0.0, TAU, true),
                true,
            ),
            Geometry::Ellipse {
                center,
                rx_s,
                ry_st,
            } => (ellipse_points(*center, *rx_s, *ry_st, 0.0, TAU, true), true),
            Geometry::Arc {
                center,
                rx_s,
                ry_st,
                start_rad,
                sweep_rad,
            } => (
                ellipse_points(*center, *rx_s, *ry_st, *start_rad, *sweep_rad, false),
                false,
            ),
            Geometry::Polygon { points } => (points.clone(), true),
        }
    }

    /// Flatten the shape into straight edges.
    pub fn edges(&self) -> Vec<Segment> {
        let (pts, closed) = self.vertices();
        let mut out: Vec<Segment> = pts
            .windows(2)
            .map(|w| Segment { a: w[0], b: w[1] })
            .collect();
        if closed && pts.len() >= 3 {
            out.push(Segment {
                a: pts[pts.len() - 1],
                b: pts[0],
            });
        }
        out
    }
}

fn check_radii(center: &Point, r_s: f64, r_st: f64) -> Result<()> {
    if !finite(center) {
        return Err(bad("non-finite center"));
    }
    if !(r_s.is_finite() && r_st.is_finite()) || r_s <= 0.0 || r_st <= 0.0 {
        return Err(bad("radii must be positive and finite"));
    }
    Ok(())
}

/// Sample an elliptical curve with `CURVE_SEGMENTS_PER_TURN` segments per turn.
///
/// Closed curves return `n` points with no repeated endpoint. Open arcs return
/// `n + 1` points, including both ends.
fn ellipse_points(
    center: Point,
    rx: f64,
    ry: f64,
    start: f64,
    sweep: f64,
    closed: bool,
) -> Vec<Point> {
    let per_turn = CURVE_SEGMENTS_PER_TURN as f64;
    if closed {
        let n = CURVE_SEGMENTS_PER_TURN;
        (0..n)
            .map(|i| {
                let th = start + sweep * (i as f64) / (n as f64);
                Point::new(center.t + rx * th.cos(), center.st + ry * th.sin())
            })
            .collect()
    } else {
        let n = ((per_turn * sweep.abs() / TAU).ceil() as usize).max(8);
        (0..=n)
            .map(|i| {
                let th = start + sweep * (i as f64) / (n as f64);
                Point::new(center.t + rx * th.cos(), center.st + ry * th.sin())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: f64, st: f64) -> Point {
        Point::new(t, st)
    }

    #[test]
    fn line_has_one_edge_and_is_open() {
        let s = Shape::outline(Geometry::Line {
            a: p(0.0, 69.0),
            b: p(1.0, 69.0),
        })
        .unwrap();
        assert!(!s.is_closed());
        assert_eq!(s.edges().len(), 1);
    }

    #[test]
    fn polyline_edges_follow_points() {
        let s = Shape::outline(Geometry::Polyline {
            points: vec![p(0.0, 60.0), p(1.0, 62.0), p(2.0, 64.0)],
        })
        .unwrap();
        assert_eq!(s.edges().len(), 2);
    }

    #[test]
    fn rectangle_is_closed_with_four_edges() {
        let s = Shape::outline(Geometry::Rectangle {
            a: p(0.0, 60.0),
            b: p(1.0, 64.0),
        })
        .unwrap();
        assert!(s.is_closed());
        assert_eq!(s.edges().len(), 4);
    }

    #[test]
    fn circle_ellipse_and_polygon_are_closed() {
        let c = Shape::outline(Geometry::Circle {
            center: p(1.0, 69.0),
            radius_s: 0.5,
            radius_st: 3.0,
        })
        .unwrap();
        assert_eq!(c.edges().len(), CURVE_SEGMENTS_PER_TURN);
        let e = Shape::outline(Geometry::Ellipse {
            center: p(1.0, 69.0),
            rx_s: 0.5,
            ry_st: 3.0,
        })
        .unwrap();
        assert_eq!(e.edges().len(), CURVE_SEGMENTS_PER_TURN);
        let g = Shape::outline(Geometry::Polygon {
            points: vec![p(0.0, 60.0), p(1.0, 60.0), p(0.5, 65.0)],
        })
        .unwrap();
        assert_eq!(g.edges().len(), 3);
    }

    #[test]
    fn arc_is_open_and_zero_sweep_is_rejected() {
        let arc = Shape::outline(Geometry::Arc {
            center: p(1.0, 69.0),
            rx_s: 0.5,
            ry_st: 3.0,
            start_rad: 0.0,
            sweep_rad: std::f64::consts::PI,
        })
        .unwrap();
        assert!(!arc.is_closed());
        assert!(!arc.edges().is_empty());

        // A short arc is a valid outline; a zero sweep is not.
        let short = Shape::outline(Geometry::Arc {
            center: p(1.0, 69.0),
            rx_s: 0.5,
            ry_st: 3.0,
            start_rad: 0.0,
            sweep_rad: 1.0,
        });
        assert!(short.is_ok(), "a short arc is a valid outline");
        let zero = Shape::outline(Geometry::Arc {
            center: p(1.0, 69.0),
            rx_s: 0.5,
            ry_st: 3.0,
            start_rad: 0.0,
            sweep_rad: 0.0,
        });
        assert!(zero.is_err(), "a zero-sweep arc is rejected");
    }

    #[test]
    fn degenerate_shapes_are_rejected() {
        assert!(
            Shape::outline(Geometry::Polyline {
                points: vec![p(0.0, 60.0)]
            })
            .is_err()
        );
        assert!(
            Shape::outline(Geometry::Polygon {
                points: vec![p(0.0, 60.0), p(1.0, 61.0)]
            })
            .is_err()
        );
        assert!(
            Shape::outline(Geometry::Circle {
                center: p(0.0, 60.0),
                radius_s: 0.0,
                radius_st: 1.0
            })
            .is_err()
        );
        assert!(
            Shape::outline(Geometry::Line {
                a: p(f64::NAN, 60.0),
                b: p(1.0, 60.0)
            })
            .is_err()
        );
    }
}

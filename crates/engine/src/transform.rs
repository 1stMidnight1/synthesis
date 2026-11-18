//! Geometric edits for the Select tool (CANVAS-07).
//!
//! Move translates a shape. Resize scales it about an anchor, with separate
//! factors for time and pitch, because the two axes have different units.
//! Both return a new shape and never mutate the input, so an invalid result
//! can be rejected and the original kept.

use crate::error::Result;
use crate::geometry::{Geometry, Point, Shape};

fn map_points(g: &Geometry, f: &impl Fn(Point) -> Point) -> Geometry {
    match g {
        Geometry::Line { a, b } => Geometry::Line { a: f(*a), b: f(*b) },
        Geometry::Polyline { points } => Geometry::Polyline {
            points: points.iter().map(|p| f(*p)).collect(),
        },
        Geometry::Rectangle { a, b } => Geometry::Rectangle { a: f(*a), b: f(*b) },
        Geometry::Polygon { points } => Geometry::Polygon {
            points: points.iter().map(|p| f(*p)).collect(),
        },
        Geometry::Circle {
            center,
            radius_s,
            radius_st,
        } => Geometry::Circle {
            center: f(*center),
            radius_s: *radius_s,
            radius_st: *radius_st,
        },
        Geometry::Ellipse {
            center,
            rx_s,
            ry_st,
        } => Geometry::Ellipse {
            center: f(*center),
            rx_s: *rx_s,
            ry_st: *ry_st,
        },
        Geometry::Arc {
            center,
            rx_s,
            ry_st,
            start_rad,
            sweep_rad,
        } => Geometry::Arc {
            center: f(*center),
            rx_s: *rx_s,
            ry_st: *ry_st,
            start_rad: *start_rad,
            sweep_rad: *sweep_rad,
        },
    }
}

/// Translate a shape by `(dt_s, dst)`. Times are clamped at zero so the shape
/// cannot leave the timeline on the left.
pub fn translate(shape: &Shape, dt_s: f64, dst: f64) -> Result<Shape> {
    let f = |p: Point| Point::new((p.t + dt_s).max(0.0), p.st + dst);
    let geometry = map_points(&shape.geometry, &f);
    Shape::outline(geometry)
}

/// Scale a shape about `anchor` by `(sx, sy)` on time and pitch.
/// Radii scale with their axes. Factors must be finite and positive.
pub fn scale(shape: &Shape, anchor: Point, sx: f64, sy: f64) -> Result<Shape> {
    let f = |p: Point| {
        Point::new(
            anchor.t + (p.t - anchor.t) * sx,
            anchor.st + (p.st - anchor.st) * sy,
        )
    };
    let mapped = map_points(&shape.geometry, &f);
    let geometry = match mapped {
        Geometry::Circle {
            center,
            radius_s,
            radius_st,
        } => Geometry::Circle {
            center,
            radius_s: radius_s * sx.abs(),
            radius_st: radius_st * sy.abs(),
        },
        Geometry::Ellipse {
            center,
            rx_s,
            ry_st,
        } => Geometry::Ellipse {
            center,
            rx_s: rx_s * sx.abs(),
            ry_st: ry_st * sy.abs(),
        },
        Geometry::Arc {
            center,
            rx_s,
            ry_st,
            start_rad,
            sweep_rad,
        } => Geometry::Arc {
            center,
            rx_s: rx_s * sx.abs(),
            ry_st: ry_st * sy.abs(),
            start_rad,
            sweep_rad,
        },
        other => other,
    };
    Shape::outline(geometry)
}

/// Distance in semitone-or-second space used for hit testing. Times are
/// scaled by `time_per_st` so that one semitone and one time unit are
/// comparable on screen.
pub fn distance_to_segment(p: Point, a: Point, b: Point, time_per_st: f64) -> f64 {
    let (ax, ay) = (a.t / time_per_st, a.st);
    let (bx, by) = (b.t / time_per_st, b.st);
    let (px, py) = (p.t / time_per_st, p.st);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let u = if len2 == 0.0 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (ax + u * dx, ay + u * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Shape {
        Shape::outline(Geometry::Rectangle {
            a: Point::new(1.0, 60.0),
            b: Point::new(3.0, 64.0),
        })
        .unwrap()
    }

    #[test]
    fn translate_moves_every_vertex() {
        let moved = translate(&rect(), 0.5, 2.0).unwrap();
        let (v, _) = moved.vertices();
        let (orig, _) = rect().vertices();
        for (a, b) in v.iter().zip(orig.iter()) {
            assert!((a.t - (b.t + 0.5)).abs() < 1e-12);
            assert!((a.st - (b.st + 2.0)).abs() < 1e-12);
        }
    }

    #[test]
    fn translate_keeps_shape_on_timeline() {
        let moved = translate(&rect(), -5.0, 0.0).unwrap();
        let (v, _) = moved.vertices();
        assert!(v.iter().all(|p| p.t >= 0.0), "left edge clamped at zero");
    }

    #[test]
    fn scale_about_anchor_keeps_anchor_fixed() {
        let anchor = Point::new(1.0, 60.0);
        let scaled = scale(&rect(), anchor, 2.0, 0.5).unwrap();
        let (v, _) = scaled.vertices();
        // The corner at the anchor does not move.
        assert!((v[0].t - anchor.t).abs() < 1e-12);
        assert!((v[0].st - anchor.st).abs() < 1e-12);
        // The far corner (3, 64) moves by the factors: t 1 + 2*2 = 5, st 60 + 4*0.5 = 62.
        assert!((v[2].t - 5.0).abs() < 1e-12);
        assert!((v[2].st - 62.0).abs() < 1e-12);
    }

    #[test]
    fn scale_scales_radii_per_axis() {
        let c = Shape::outline(Geometry::Circle {
            center: Point::new(2.0, 60.0),
            radius_s: 1.0,
            radius_st: 4.0,
        })
        .unwrap();
        let s = scale(&c, Point::new(2.0, 60.0), 3.0, 0.5).unwrap();
        match s.geometry {
            Geometry::Circle {
                radius_s,
                radius_st,
                ..
            } => {
                assert!((radius_s - 3.0).abs() < 1e-12);
                assert!((radius_st - 2.0).abs() < 1e-12);
            }
            _ => panic!("expected circle"),
        }
    }

    #[test]
    fn invalid_scale_is_rejected_by_validation() {
        // A zero factor collapses a rectangle to a line. A zero-radius circle is invalid.
        let c = Shape::outline(Geometry::Circle {
            center: Point::new(2.0, 60.0),
            radius_s: 1.0,
            radius_st: 4.0,
        })
        .unwrap();
        assert!(scale(&c, Point::new(2.0, 60.0), 0.0, 1.0).is_err());
    }

    #[test]
    fn distance_to_segment_is_zero_on_the_line() {
        let a = Point::new(0.0, 60.0);
        let b = Point::new(2.0, 60.0);
        assert!(distance_to_segment(Point::new(1.0, 60.0), a, b, 0.1) < 1e-12);
        assert!(distance_to_segment(Point::new(1.0, 61.0), a, b, 0.1) > 0.5);
    }
}

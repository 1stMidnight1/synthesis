//! Sketch construction and handle editing (CANVAS-02, CANVAS-07).
//!
//! The canvas axes have different units (seconds and semitones), so "round"
//! only means something relative to the current view. Every function that
//! builds or edits a round shape takes `k`, the number of seconds that looks
//! as long on screen as one semitone. Points are mapped to normalised
//! coordinates `(u, v) = (t / k, st)`, where circles are circles, and mapped
//! back. A circle drawn at one zoom stays the same data afterwards; it only
//! looks elliptical if the view's aspect changes later, as on any time-pitch
//! canvas.
//!
//! All constructors return `None` for degenerate input (zero size, collinear
//! arc points) instead of producing shapes that cannot be seen or heard.

use std::f64::consts::TAU;

use crate::error::{EngineError, Result};
use crate::geometry::{Geometry, Point, Shape};

/// Smallest accepted size in normalised units (about 1/1000 of a semitone).
const MIN_SIZE: f64 = 1e-3;

/// Default number of sides for the regular polygon tool.
pub const DEFAULT_POLYGON_SIDES: usize = 6;

/// Accepted side counts for regular polygons.
pub const POLYGON_SIDES: std::ops::RangeInclusive<usize> = 3..=64;

fn n(p: Point, k: f64) -> (f64, f64) {
    (p.t / k, p.st)
}

fn from_n(u: f64, v: f64, k: f64) -> Point {
    Point::new(u * k, v)
}

fn ndist(a: Point, b: Point, k: f64) -> f64 {
    let (au, av) = n(a, k);
    let (bu, bv) = n(b, k);
    (bu - au).hypot(bv - av)
}

fn valid_k(k: f64) -> bool {
    k.is_finite() && k > 0.0
}

/// Straight line from `a` to `b`, or `None` if the points coincide.
pub fn line(a: Point, b: Point) -> Option<Geometry> {
    let same = (a.t - b.t).abs() < 1e-9 && (a.st - b.st).abs() < 1e-9;
    (!same).then_some(Geometry::Line { a, b })
}

/// Corner rectangle between opposite corners, or `None` if it has no area.
pub fn rectangle(a: Point, b: Point) -> Option<Geometry> {
    let flat = (a.t - b.t).abs() < 1e-9 || (a.st - b.st).abs() < 1e-9;
    (!flat).then_some(Geometry::Rectangle { a, b })
}

/// Centre-point circle through `through`, round in the view described by `k`.
pub fn circle(center: Point, through: Point, k: f64) -> Option<Geometry> {
    if !valid_k(k) {
        return None;
    }
    let r = ndist(center, through, k);
    (r >= MIN_SIZE).then_some(Geometry::Circle {
        center,
        radius_s: r * k,
        radius_st: r,
    })
}

/// Axis-aligned ellipse from its centre and a corner of its bounding box.
pub fn ellipse(center: Point, corner: Point) -> Option<Geometry> {
    let rx = (corner.t - center.t).abs();
    let ry = (corner.st - center.st).abs();
    (rx >= 1e-6 && ry >= MIN_SIZE).then_some(Geometry::Ellipse {
        center,
        rx_s: rx,
        ry_st: ry,
    })
}

/// Regular polygon inscribed in a circle: `vertex` is one of its corners.
pub fn regular_polygon(center: Point, vertex: Point, sides: usize, k: f64) -> Option<Geometry> {
    if !valid_k(k) || !POLYGON_SIDES.contains(&sides) {
        return None;
    }
    let (cu, cv) = n(center, k);
    let (vu, vv) = n(vertex, k);
    let r = (vu - cu).hypot(vv - cv);
    if r < MIN_SIZE {
        return None;
    }
    let a0 = (vv - cv).atan2(vu - cu);
    let points = (0..sides)
        .map(|i| {
            let a = a0 + TAU * i as f64 / sides as f64;
            from_n(cu + r * a.cos(), cv + r * a.sin(), k)
        })
        .collect();
    Some(Geometry::Polygon { points })
}

/// Three-point arc: from `start` to `end`, passing through `through`.
/// Returns `None` if the three points are (nearly) collinear or coincide.
pub fn arc_3pt(start: Point, end: Point, through: Point, k: f64) -> Option<Geometry> {
    if !valid_k(k) {
        return None;
    }
    let (ax, ay) = n(start, k);
    let (bx, by) = n(end, k);
    let (mx, my) = n(through, k);
    let d = 2.0 * (ax * (by - my) + bx * (my - ay) + mx * (ay - by));
    let scale = (bx - ax).hypot(by - ay).max(MIN_SIZE);
    if d.abs() < 1e-9 * scale * scale {
        return None;
    }
    let (a2, b2, m2) = (ax * ax + ay * ay, bx * bx + by * by, mx * mx + my * my);
    let cx = (a2 * (by - my) + b2 * (my - ay) + m2 * (ay - by)) / d;
    let cy = (a2 * (mx - bx) + b2 * (ax - mx) + m2 * (bx - ax)) / d;
    let r = (ax - cx).hypot(ay - cy);
    if !(r.is_finite() && r >= MIN_SIZE) || (bx - ax).hypot(by - ay) < MIN_SIZE {
        return None;
    }
    let ta = (ay - cy).atan2(ax - cx);
    let tb = (by - cy).atan2(bx - cx);
    let tm = (my - cy).atan2(mx - cx);
    let ccw = (tb - ta).rem_euclid(TAU);
    let m_off = (tm - ta).rem_euclid(TAU);
    let sweep = if m_off < ccw { ccw } else { ccw - TAU };
    if sweep.abs() < 1e-9 {
        return None;
    }
    Some(Geometry::Arc {
        center: from_n(cx, cy, k),
        rx_s: r * k,
        ry_st: r,
        start_rad: ta,
        sweep_rad: sweep,
    })
}

/// Point on an elliptical curve at angle `a`.
fn on_ellipse(center: Point, rx: f64, ry: f64, a: f64) -> Point {
    Point::new(center.t + rx * a.cos(), center.st + ry * a.sin())
}

/// Start, middle, and end points of an arc.
pub fn arc_points(g: &Geometry) -> Option<(Point, Point, Point)> {
    match *g {
        Geometry::Arc {
            center,
            rx_s,
            ry_st,
            start_rad,
            sweep_rad,
        } => Some((
            on_ellipse(center, rx_s, ry_st, start_rad),
            on_ellipse(center, rx_s, ry_st, start_rad + sweep_rad / 2.0),
            on_ellipse(center, rx_s, ry_st, start_rad + sweep_rad),
        )),
        _ => None,
    }
}

/// What a handle edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleKind {
    /// Endpoint of a line (0, 1) or vertex of a polyline or polygon.
    Vertex(usize),
    /// Rectangle corner, in the order a, (b.t, a.st), b, (a.t, b.st).
    Corner(usize),
    /// Rectangle edge midpoint, in [`Shape::edges`] order: 0 the side at
    /// `a.st`, 1 at `b.t`, 2 at `b.st`, 3 at `a.t`. Dragging moves that side only.
    Edge(usize),
    /// Centre of a circle, ellipse, or arc. Dragging it moves the shape.
    Center,
    /// Point on a circle or ellipse: 0 right, 1 top, 2 left, 3 bottom.
    Radius(usize),
    ArcStart,
    ArcMid,
    ArcEnd,
}

/// A draggable point on a shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle {
    pub pos: Point,
    pub kind: HandleKind,
}

fn rect_corners(a: Point, b: Point) -> [Point; 4] {
    [a, Point::new(b.t, a.st), b, Point::new(a.t, b.st)]
}

/// The handles of a shape, as Onshape shows them: endpoints and vertices,
/// rectangle corners, centres, radius points, and arc start/middle/end.
pub fn handles(shape: &Shape) -> Vec<Handle> {
    let h = |pos, kind| Handle { pos, kind };
    match &shape.geometry {
        Geometry::Line { a, b } => vec![h(*a, HandleKind::Vertex(0)), h(*b, HandleKind::Vertex(1))],
        Geometry::Polyline { points } | Geometry::Polygon { points } => points
            .iter()
            .enumerate()
            .map(|(i, p)| h(*p, HandleKind::Vertex(i)))
            .collect(),
        Geometry::Rectangle { a, b } => {
            let c = rect_corners(*a, *b);
            let mut v: Vec<Handle> = c
                .iter()
                .enumerate()
                .map(|(i, p)| h(*p, HandleKind::Corner(i)))
                .collect();
            for i in 0..4 {
                let (p, q) = (c[i], c[(i + 1) % 4]);
                v.push(h(
                    Point::new((p.t + q.t) / 2.0, (p.st + q.st) / 2.0),
                    HandleKind::Edge(i),
                ));
            }
            v
        }
        Geometry::Circle {
            center,
            radius_s,
            radius_st,
        } => radius_handles(*center, *radius_s, *radius_st),
        Geometry::Ellipse {
            center,
            rx_s,
            ry_st,
        } => radius_handles(*center, *rx_s, *ry_st),
        Geometry::Arc { center, .. } => {
            let (s, m, e) = arc_points(&shape.geometry).expect("arc geometry");
            vec![
                h(s, HandleKind::ArcStart),
                h(m, HandleKind::ArcMid),
                h(e, HandleKind::ArcEnd),
                h(*center, HandleKind::Center),
            ]
        }
    }
}

fn radius_handles(c: Point, rx: f64, ry: f64) -> Vec<Handle> {
    let mut v = vec![Handle {
        pos: c,
        kind: HandleKind::Center,
    }];
    for (i, (dt, dst)) in [(rx, 0.0), (0.0, ry), (-rx, 0.0), (0.0, -ry)]
        .iter()
        .enumerate()
    {
        v.push(Handle {
            pos: Point::new(c.t + dt, c.st + dst),
            kind: HandleKind::Radius(i),
        });
    }
    v
}

fn bad(msg: &str) -> EngineError {
    EngineError::InvalidShape(msg.to_string())
}

/// Move one handle of `shape` to `to` and return the edited shape.
///
/// Endpoints and vertices move alone; a rectangle corner moves with the
/// opposite corner fixed; a centre moves the whole shape; a circle radius
/// point scales the circle uniformly; an ellipse radius point changes that
/// axis only; an arc point is re-solved as a three-point arc with the other
/// two points fixed. Degenerate results are rejected, never applied.
pub fn drag_handle(shape: &Shape, kind: HandleKind, to: Point, k: f64) -> Result<Shape> {
    if !(to.t.is_finite() && to.st.is_finite()) {
        return Err(bad("non-finite handle position"));
    }
    let g = &shape.geometry;
    let geometry = match (g, kind) {
        (Geometry::Line { a, b }, HandleKind::Vertex(i)) => {
            let (a, b) = if i == 0 { (to, *b) } else { (*a, to) };
            line(a, b).ok_or_else(|| bad("line would have zero length"))?
        }
        (Geometry::Polyline { points }, HandleKind::Vertex(i)) if i < points.len() => {
            let mut points = points.clone();
            points[i] = to;
            Geometry::Polyline { points }
        }
        (Geometry::Polygon { points }, HandleKind::Vertex(i)) if i < points.len() => {
            let mut points = points.clone();
            points[i] = to;
            Geometry::Polygon { points }
        }
        (Geometry::Rectangle { a, b }, HandleKind::Corner(i)) if i < 4 => {
            let opposite = rect_corners(*a, *b)[(i + 2) % 4];
            rectangle(opposite, to).ok_or_else(|| bad("rectangle would have no area"))?
        }
        (Geometry::Rectangle { a, b }, HandleKind::Edge(i)) if i < 4 => {
            let (mut a, mut b) = (*a, *b);
            match i {
                0 => a.st = to.st,
                1 => b.t = to.t,
                2 => b.st = to.st,
                _ => a.t = to.t,
            }
            rectangle(a, b).ok_or_else(|| bad("rectangle would have no area"))?
        }
        (Geometry::Circle { center, .. }, HandleKind::Center)
        | (Geometry::Ellipse { center, .. }, HandleKind::Center)
        | (Geometry::Arc { center, .. }, HandleKind::Center) => {
            let (dt, dst) = (to.t - center.t, to.st - center.st);
            return crate::transform::translate(shape, dt, dst);
        }
        (
            Geometry::Circle {
                center,
                radius_s,
                radius_st,
            },
            HandleKind::Radius(i),
        ) => {
            if !valid_k(k) {
                return Err(bad("invalid view aspect"));
            }
            // Uniform scale: ratio of the pointer's distance from the centre to
            // the dragged handle's distance, both in normalised view units.
            let old = if i % 2 == 0 { radius_s / k } else { *radius_st };
            let f = ndist(*center, to, k) / old.max(MIN_SIZE);
            if !(f.is_finite() && f * radius_st >= MIN_SIZE) {
                return Err(bad("circle would have zero radius"));
            }
            Geometry::Circle {
                center: *center,
                radius_s: radius_s * f,
                radius_st: radius_st * f,
            }
        }
        (
            Geometry::Ellipse {
                center,
                rx_s,
                ry_st,
            },
            HandleKind::Radius(i),
        ) => {
            let (rx, ry) = if i % 2 == 0 {
                ((to.t - center.t).abs(), *ry_st)
            } else {
                (*rx_s, (to.st - center.st).abs())
            };
            ellipse(*center, Point::new(center.t + rx, center.st + ry))
                .ok_or_else(|| bad("ellipse would have zero radius"))?
        }
        (Geometry::Arc { .. }, HandleKind::ArcStart | HandleKind::ArcMid | HandleKind::ArcEnd) => {
            let (s, m, e) = arc_points(g).expect("arc geometry");
            let (s, m, e) = match kind {
                HandleKind::ArcStart => (to, m, e),
                HandleKind::ArcMid => (s, to, e),
                _ => (s, m, to),
            };
            arc_3pt(s, e, m, k).ok_or_else(|| bad("arc points are collinear"))?
        }
        _ => return Err(bad("handle does not belong to this shape")),
    };
    Shape::outline(geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: f64 = 0.1; // 0.1 s looks as long as 1 semitone

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    fn on_curve(g: &Geometry, p: Point) -> bool {
        // Distance from p to the shape's edges, in normalised units.
        let s = Shape::outline(g.clone()).unwrap();
        s.edges().iter().any(|e| {
            let (ax, ay) = n(e.a, K);
            let (bx, by) = n(e.b, K);
            let (px, py) = n(p, K);
            let (dx, dy) = (bx - ax, by - ay);
            let u = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            (px - ax - u * dx).hypot(py - ay - u * dy) < 0.02
        })
    }

    #[test]
    fn circle_is_round_in_the_view() {
        let g = circle(Point::new(1.0, 60.0), Point::new(1.3, 60.0), K).unwrap();
        match g {
            Geometry::Circle {
                radius_s,
                radius_st,
                ..
            } => {
                close(radius_s, 0.3);
                close(radius_st, 3.0); // 0.3 s / 0.1 s per st
            }
            _ => panic!(),
        }
        // A purely horizontal drag still gives a round circle, not a sliver.
        assert!(circle(Point::new(1.0, 60.0), Point::new(1.0, 60.0), K).is_none());
    }

    #[test]
    fn three_point_arc_passes_through_all_points() {
        let (s, e, m) = (
            Point::new(1.0, 60.0),
            Point::new(2.0, 60.0),
            Point::new(1.5, 63.0),
        );
        let g = arc_3pt(s, e, m, K).unwrap();
        let (gs, gm, ge) = arc_points(&g).unwrap();
        close(gs.t, s.t);
        close(gs.st, s.st);
        close(ge.t, e.t);
        close(ge.st, e.st);
        assert!(on_curve(&g, m), "arc must pass through the third point");
        // The arc bulges toward the third point (up), so its middle is above the chord.
        assert!(gm.st > 60.0);
    }

    #[test]
    fn three_point_arc_bulges_either_way() {
        let (s, e) = (Point::new(1.0, 60.0), Point::new(2.0, 60.0));
        let down = arc_3pt(s, e, Point::new(1.5, 57.0), K).unwrap();
        let (_, mid, _) = arc_points(&down).unwrap();
        assert!(mid.st < 60.0, "bulges down toward the third point");
        // A large arc (more than half a circle) is also reachable.
        let big = arc_3pt(s, e, Point::new(1.5, 70.0), K).unwrap();
        assert!(on_curve(&big, Point::new(1.5, 70.0)));
    }

    #[test]
    fn collinear_arc_is_rejected() {
        let (s, e) = (Point::new(1.0, 60.0), Point::new(2.0, 60.0));
        assert!(arc_3pt(s, e, Point::new(1.5, 60.0), K).is_none());
        assert!(arc_3pt(s, s, Point::new(1.5, 62.0), K).is_none());
    }

    #[test]
    fn regular_polygon_has_equal_sides_in_the_view() {
        let g = regular_polygon(Point::new(1.0, 60.0), Point::new(1.0, 64.0), 6, K).unwrap();
        let Geometry::Polygon { points } = &g else {
            panic!()
        };
        assert_eq!(points.len(), 6);
        close(points[0].t, 1.0);
        close(points[0].st, 64.0);
        let sides: Vec<f64> = (0..6)
            .map(|i| ndist(points[i], points[(i + 1) % 6], K))
            .collect();
        for s in &sides {
            close(*s, sides[0]);
        }
        assert!(regular_polygon(Point::new(1.0, 60.0), Point::new(1.0, 64.0), 2, K).is_none());
    }

    #[test]
    fn degenerate_rectangles_and_lines_are_rejected() {
        assert!(rectangle(Point::new(0.0, 60.0), Point::new(1.0, 60.0)).is_none());
        assert!(rectangle(Point::new(1.0, 60.0), Point::new(1.0, 62.0)).is_none());
        assert!(line(Point::new(1.0, 60.0), Point::new(1.0, 60.0)).is_none());
        assert!(ellipse(Point::new(1.0, 60.0), Point::new(1.5, 60.0)).is_none());
    }

    fn drag(shape: &Shape, kind: HandleKind, to: Point) -> Shape {
        drag_handle(shape, kind, to, K).unwrap()
    }

    #[test]
    fn line_endpoint_moves_alone() {
        let s =
            Shape::outline(line(Point::new(0.0, 60.0), Point::new(1.0, 60.0)).unwrap()).unwrap();
        let d = drag(&s, HandleKind::Vertex(1), Point::new(2.0, 65.0));
        assert_eq!(
            d.geometry,
            Geometry::Line {
                a: Point::new(0.0, 60.0),
                b: Point::new(2.0, 65.0)
            }
        );
        assert!(drag_handle(&s, HandleKind::Vertex(1), Point::new(0.0, 60.0), K).is_err());
    }

    #[test]
    fn polygon_vertex_moves_and_fill_is_kept() {
        let s = Shape::outline(Geometry::Polygon {
            points: vec![
                Point::new(0.0, 60.0),
                Point::new(1.0, 60.0),
                Point::new(0.5, 64.0),
            ],
        })
        .unwrap();
        let d = drag(&s, HandleKind::Vertex(2), Point::new(0.5, 70.0));

        let Geometry::Polygon { points } = &d.geometry else {
            panic!()
        };
        assert_eq!(points[2], Point::new(0.5, 70.0));
        assert_eq!(points[0], Point::new(0.0, 60.0));
    }

    #[test]
    fn rectangle_corner_keeps_opposite_corner() {
        let s = Shape::outline(rectangle(Point::new(1.0, 60.0), Point::new(3.0, 64.0)).unwrap())
            .unwrap();
        // Corner 2 is b = (3, 64); the opposite corner a = (1, 60) stays.
        let d = drag(&s, HandleKind::Corner(2), Point::new(4.0, 66.0));
        assert_eq!(
            d.geometry,
            Geometry::Rectangle {
                a: Point::new(1.0, 60.0),
                b: Point::new(4.0, 66.0)
            }
        );
        // Corner 1 is (3, 60); its opposite (1, 64) stays.
        let d = drag(&s, HandleKind::Corner(1), Point::new(5.0, 58.0));
        assert_eq!(
            d.geometry,
            Geometry::Rectangle {
                a: Point::new(1.0, 64.0),
                b: Point::new(5.0, 58.0)
            }
        );
        assert!(drag_handle(&s, HandleKind::Corner(2), Point::new(1.0, 66.0), K).is_err());
    }

    #[test]
    fn rectangle_edge_moves_only_that_edge() {
        let s = Shape::outline(rectangle(Point::new(1.0, 60.0), Point::new(3.0, 64.0)).unwrap())
            .unwrap();
        // Edge 1 is the side at b.t = 3: dragging it changes the end time only.
        let d = drag(&s, HandleKind::Edge(1), Point::new(5.0, 99.0));
        assert_eq!(
            d.geometry,
            Geometry::Rectangle {
                a: Point::new(1.0, 60.0),
                b: Point::new(5.0, 64.0)
            }
        );

        // Edge 0 is at a.st = 60: dragging it changes that pitch only.
        let d = drag(&s, HandleKind::Edge(0), Point::new(-7.0, 57.0));
        assert_eq!(
            d.geometry,
            Geometry::Rectangle {
                a: Point::new(1.0, 57.0),
                b: Point::new(3.0, 64.0)
            }
        );
        // Collapsing the rectangle is rejected.
        assert!(drag_handle(&s, HandleKind::Edge(3), Point::new(3.0, 62.0), K).is_err());
    }

    #[test]
    fn circle_radius_handle_scales_uniformly_and_center_moves() {
        let s = Shape::outline(circle(Point::new(1.0, 60.0), Point::new(1.2, 60.0), K).unwrap())
            .unwrap();
        let d = drag(&s, HandleKind::Radius(1), Point::new(1.0, 64.0));
        let Geometry::Circle {
            center,
            radius_s,
            radius_st,
        } = d.geometry
        else {
            panic!()
        };
        close(radius_st, 4.0);
        close(radius_s, 0.4);
        assert_eq!(center, Point::new(1.0, 60.0));
        let m = drag(&s, HandleKind::Center, Point::new(2.0, 62.0));
        let Geometry::Circle { center, .. } = m.geometry else {
            panic!()
        };
        assert_eq!(center, Point::new(2.0, 62.0));
    }

    #[test]
    fn ellipse_radius_handles_change_one_axis() {
        let s =
            Shape::outline(ellipse(Point::new(1.0, 60.0), Point::new(1.5, 63.0)).unwrap()).unwrap();
        let d = drag(&s, HandleKind::Radius(0), Point::new(2.0, 61.0));
        let Geometry::Ellipse { rx_s, ry_st, .. } = d.geometry else {
            panic!()
        };
        close(rx_s, 1.0);
        close(ry_st, 3.0);
        let d = drag(&s, HandleKind::Radius(3), Point::new(1.2, 55.0));
        let Geometry::Ellipse { rx_s, ry_st, .. } = d.geometry else {
            panic!()
        };
        close(rx_s, 0.5);
        close(ry_st, 5.0);
    }

    #[test]
    fn arc_endpoint_drag_keeps_other_points() {
        let g = arc_3pt(
            Point::new(1.0, 60.0),
            Point::new(2.0, 60.0),
            Point::new(1.5, 63.0),
            K,
        )
        .unwrap();
        let s = Shape::outline(g).unwrap();
        let (_, m0, e0) = arc_points(&s.geometry).unwrap();
        let d = drag(&s, HandleKind::ArcStart, Point::new(0.8, 58.0));
        let (s1, _, e1) = arc_points(&d.geometry).unwrap();
        close(s1.t, 0.8);
        close(s1.st, 58.0);
        close(e1.t, e0.t);
        close(e1.st, e0.st);
        assert!(
            on_curve(&d.geometry, m0),
            "old middle point still on the arc"
        );
    }

    #[test]
    fn handles_cover_each_shape_kind() {
        let arc = Shape::outline(
            arc_3pt(
                Point::new(1.0, 60.0),
                Point::new(2.0, 60.0),
                Point::new(1.5, 63.0),
                K,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(handles(&arc).len(), 4);
        let c = Shape::outline(circle(Point::new(1.0, 60.0), Point::new(1.2, 60.0), K).unwrap())
            .unwrap();
        assert_eq!(handles(&c).len(), 5);
        let r = Shape::outline(rectangle(Point::new(1.0, 60.0), Point::new(2.0, 62.0)).unwrap())
            .unwrap();
        assert_eq!(handles(&r).len(), 8, "4 corners and 4 edge midpoints");
        let mids: Vec<_> = handles(&r)
            .into_iter()
            .filter(|h| matches!(h.kind, HandleKind::Edge(_)))
            .collect();
        assert!(
            mids.iter()
                .any(|h| h.pos == Point::new(2.0, 61.0) && h.kind == HandleKind::Edge(1))
        );
    }
}

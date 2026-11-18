//! Volume canvas (M4, VOL-01 to VOL-04, R5, R9).
//!
//! Time runs left to right on the shared timeline axis; the vertical axis is
//! level in percent, 0 at the bottom to 200 at the top (R5, D-10). The left and
//! right lanes of the selected voice are drawn here, the left in red and the
//! right in blue (VOL-02).
//!
//! Tools are the pitch canvas's line and polyline tools, plus Select and Delete
//! (the same tools as the pitch canvas, per the M4 scope). Freehand is deferred.
//! A stroke is a piecewise-linear curve in (time, percent); its points are
//! stored on the lane, so undo, saving and export all see the same data.

use eframe::egui::{Color32, Pos2};
use synthesis_engine::{LanePoint, Stroke};

use crate::view::View;
use synthesis_engine::Point;

/// Snap step for levels, in percent. Levels snap to the grid when snapping is on.
pub const LEVEL_STEP: f64 = 25.0;

pub const LEFT_COLOR: Color32 = Color32::from_rgb(235, 90, 90);
pub const RIGHT_COLOR: Color32 = Color32::from_rgb(90, 140, 240);
/// A stroke present in both lanes (a `Both` stroke, VOL-02) is purple.
pub const BOTH_COLOR: Color32 = Color32::from_rgb(170, 105, 235);

/// Which lane a stroke belongs to. Purple edits both at once (VOL-02); that
/// is `Both`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Left,
    Right,
    Both,
}

/// Snap a level to the nearest grid line, if snapping is on.
pub fn snap_level(percent: f64, snap: bool) -> f64 {
    if snap {
        (percent / LEVEL_STEP).round() * LEVEL_STEP
    } else {
        percent
    }
}

/// Finish a polyline from its pending points (two or more) into one stroke
/// edit on `channel`. The pending list is cleared. Points are (time, level).
pub fn finish_points(pending: &mut Vec<Point>, channel: Channel) -> Option<Edit> {
    let pts = std::mem::take(pending);
    if pts.len() < 2 {
        return None;
    }
    let points: Vec<LanePoint> = pts.iter().map(|q| LanePoint::new(q.t, q.st)).collect();
    Stroke::new(points)
        .ok()
        .map(|s| Edit::AddStroke(channel, s))
}

/// Build a point list for a line between two lane points (level already snapped).
pub fn line_stroke(a: (f64, f64), b: (f64, f64)) -> Option<Stroke> {
    Stroke::new(vec![LanePoint::new(a.0, a.1), LanePoint::new(b.0, b.1)]).ok()
}

/// What a lane edit does to the selected voice. Each edit is one undo step.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Add a stroke to the lanes named by `channel`.
    AddStroke(Channel, Stroke),
    /// Remove stroke `index` from the lanes named by `channel`. For `Both`,
    /// the same index is removed from both lanes.
    RemoveStroke(Channel, usize),
}

/// Apply an edit to a voice. Returns false if it would be invalid.
pub fn apply(voice: &mut synthesis_engine::Voice, edit: &Edit) -> bool {
    match edit {
        Edit::AddStroke(ch, stroke) => {
            if matches!(ch, Channel::Left | Channel::Both) {
                voice.lane_left.strokes.push(stroke.clone());
            }
            if matches!(ch, Channel::Right | Channel::Both) {
                voice.lane_right.strokes.push(stroke.clone());
            }
            true
        }
        Edit::RemoveStroke(ch, i) => {
            if matches!(ch, Channel::Left | Channel::Both) && *i < voice.lane_left.strokes.len() {
                voice.lane_left.strokes.remove(*i);
            }
            if matches!(ch, Channel::Right | Channel::Both) && *i < voice.lane_right.strokes.len() {
                voice.lane_right.strokes.remove(*i);
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synthesis_engine::Voice;

    #[test]
    fn levels_snap_to_the_grid_when_on() {
        assert_eq!(snap_level(77.0, true), 75.0);
        assert_eq!(snap_level(77.0, false), 77.0);
    }

    #[test]
    fn line_stroke_is_two_points() {
        let s = line_stroke((0.0, 75.0), (1.0, 50.0)).unwrap();
        assert_eq!(s.points.len(), 2);
    }

    #[test]
    fn add_and_remove_strokes_by_channel() {
        let mut v = Voice::new("v");
        let base_l = v.lane_left.strokes.len();
        let base_r = v.lane_right.strokes.len();
        let s = line_stroke((0.0, 50.0), (1.0, 50.0)).unwrap();
        assert!(apply(&mut v, &Edit::AddStroke(Channel::Left, s.clone())));
        assert_eq!(v.lane_left.strokes.len(), base_l + 1);
        assert_eq!(v.lane_right.strokes.len(), base_r, "left only");
        assert!(apply(&mut v, &Edit::AddStroke(Channel::Both, s)));
        assert_eq!(v.lane_left.strokes.len(), base_l + 2);
        assert_eq!(v.lane_right.strokes.len(), base_r + 1);
        assert!(apply(&mut v, &Edit::RemoveStroke(Channel::Both, base_l)));
        assert_eq!(v.lane_left.strokes.len(), base_l + 1);
    }
}

/// Distance in screen points from `p` to a stroke's curve on the lane canvas.
fn stroke_distance(stroke: &Stroke, v: &View, p: Pos2) -> f32 {
    let pts: Vec<Pos2> = stroke
        .points
        .iter()
        .map(|q| v.point_to_screen(Point::new(q.t_s, q.percent)))
        .collect();
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let len2 = ab.length_sq();
            let u = if len2 <= f32::EPSILON {
                0.0
            } else {
                ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
            };
            (p - (a + ab * u)).length()
        })
        .fold(f32::INFINITY, f32::min)
}

/// The stroke index nearest to `p` among the lanes named by `channel`, within
/// `pick_px` screen points. Returns the index into the first matching lane.
pub fn pick_stroke(lane_strokes: &[Stroke], v: &View, p: Pos2, pick_px: f32) -> Option<usize> {
    lane_strokes
        .iter()
        .enumerate()
        .map(|(i, s)| (i, stroke_distance(s, v, p)))
        .filter(|(_, d)| *d <= pick_px)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod pick_tests {
    use super::*;
    use crate::document::TimeView;
    use crate::view::VerticalMode;
    use eframe::egui::{Rect, Vec2};
    use synthesis_engine::Voice;

    fn lane_view() -> View {
        View {
            rect: Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(400.0, 200.0)),
            time: TimeView {
                start_s: 0.0,
                width_s: 4.0,
            },
            pitch: crate::document::PitchView::default(),
            a4: 440.0,
            mode: VerticalMode::Volume,
        }
    }

    #[test]
    fn pick_finds_the_stroke_under_the_pointer_and_delete_removes_it() {
        let v = lane_view();
        let mut voice = Voice::new("v");
        // Two flat strokes: one at 50%, one at 150%.
        voice.lane_left.strokes = vec![
            line_stroke((0.0, 50.0), (4.0, 50.0)).unwrap(),
            line_stroke((0.0, 150.0), (4.0, 150.0)).unwrap(),
        ];
        let on_low = v.point_to_screen(Point::new(2.0, 50.0));
        assert_eq!(
            pick_stroke(&voice.lane_left.strokes, &v, on_low, 6.0),
            Some(0)
        );
        // 100% is between the two strokes: nothing is picked there.
        let between = v.point_to_screen(Point::new(2.0, 100.0));
        assert_eq!(
            pick_stroke(&voice.lane_left.strokes, &v, between, 6.0),
            None
        );
        assert!(apply(&mut voice, &Edit::RemoveStroke(Channel::Left, 0)));
        assert_eq!(voice.lane_left.strokes.len(), 1);
    }
}

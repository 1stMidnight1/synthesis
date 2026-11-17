//! Pitch-time canvas (CANVAS-01 to 04, GRID-01 to 04, R8).
//!
//! Pan with middle-drag, zoom with the wheel, and draw with the tool palette.
//! Shapes are stored as vectors in the document; this module only converts
//! between screen and (time, pitch) coordinates and draws them (CANVAS-06).

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use synthesis_engine::pitchscale::{PitchScale, from_fraction, note_name, to_fraction};
use synthesis_engine::{Geometry, Point, Shape, Subdivision, Tempo};

use crate::document::{Document, PitchView, TimeView};

/// Drawing tools (CANVAS-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Line,
    Polyline,
    Rectangle,
    Circle,
    Ellipse,
    Arc,
    Polygon,
    Fill,
    Delete,
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Tool::Select,
        Tool::Line,
        Tool::Polyline,
        Tool::Rectangle,
        Tool::Circle,
        Tool::Ellipse,
        Tool::Arc,
        Tool::Polygon,
        Tool::Fill,
        Tool::Delete,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Polyline => "Polyline",
            Tool::Rectangle => "Rect",
            Tool::Circle => "Circle",
            Tool::Ellipse => "Ellipse",
            Tool::Arc => "Arc",
            Tool::Polygon => "Polygon",
            Tool::Fill => "Fill",
            Tool::Delete => "Delete",
        }
    }
}

/// Canvas-local UI state (not part of the scene).
#[derive(Default)]
pub struct CanvasState {
    pub tool: Tool,
    /// Fill mode for new closed shapes (CANVAS-03).
    pub fill_new: bool,
    pub snap: bool,
    pub show_grid: bool,
    pub bpm: f64,
    pub subdivision: Option<Subdivision>,
    /// Pending points while a multi-click tool is in progress (polyline, polygon).
    pub pending: Vec<Point>,
    /// Drag start for two-point tools (line, rectangle, circle, ellipse).
    pub drag_start: Option<Point>,
    pub status: String,
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            show_grid: true,
            snap: true,
            bpm: 120.0,
            subdivision: Some(Subdivision::Beat),
            ..Self::default()
        }
    }
}

/// Map a screen position to (seconds, pitch in semitones) for the given view.
pub fn screen_to_point(pos: Pos2, rect: Rect, time: TimeView, pitch: PitchView, a4: f64) -> Point {
    let fx = f64::from((pos.x - rect.left()) / rect.width());
    let fy = f64::from((rect.bottom() - pos.y) / rect.height());
    let t = time.start_s + fx * time.width_s;
    let st = from_fraction(pitch.scale, fy, pitch.lo_hz, pitch.hi_hz, a4);
    Point::new(t, st)
}

/// Map (seconds, pitch) to a screen position.
pub fn point_to_screen(p: Point, rect: Rect, time: TimeView, pitch: PitchView, a4: f64) -> Pos2 {
    let fx = ((p.t - time.start_s) / time.width_s) as f32;
    let fy = to_fraction(pitch.scale, p.st, pitch.lo_hz, pitch.hi_hz, a4) as f32;
    Pos2::new(
        rect.left() + fx * rect.width(),
        rect.bottom() - fy * rect.height(),
    )
}

/// Snap a point to the time subdivision and nearest semitone if snapping is on.
fn maybe_snap(p: Point, state: &CanvasState) -> Point {
    if !state.snap {
        return p;
    }
    let t = match (state.subdivision, Tempo::new(state.bpm)) {
        (Some(sub), Some(tempo)) => {
            let step = sub.step_s(tempo);
            (p.t / step).round() * step
        }
        _ => p.t,
    };
    Point::new(t.max(0.0), p.st.round())
}

/// Draw the canvas and handle its input. Returns the response for focus checks.
pub fn show(ui: &mut Ui, doc: &mut Document, state: &mut CanvasState, a4: f64) -> Response {
    let size = Vec2::new(ui.available_width(), ui.available_height().max(200.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(22));

    // Pan with middle-drag, zoom with the wheel (CANVAS-01).
    if response.dragged_by(egui::PointerButton::Middle) {
        let d = response.drag_delta();
        let dt = -f64::from(d.x) / f64::from(rect.width()) * doc.time.width_s;
        doc.time.start_s = (doc.time.start_s + dt).max(0.0);
        let dy = f64::from(d.y) / f64::from(rect.height());
        let log_lo = doc.pitch.lo_hz.ln();
        let log_hi = doc.pitch.hi_hz.ln();
        let shift = dy * (log_hi - log_lo);
        if matches!(doc.pitch.scale, PitchScale::Semitones | PitchScale::LogHz) {
            doc.pitch.lo_hz = (log_lo + shift).exp();
            doc.pitch.hi_hz = (log_hi + shift).exp();
        }
    }
    if response.hovered() {
        let zoom = ui.input(|i| i.zoom_delta());
        if (zoom - 1.0).abs() > f32::EPSILON {
            doc.time.width_s = (doc.time.width_s / f64::from(zoom)).clamp(0.05, 60.0);
        }
    }

    draw_grid(&painter, rect, state, doc);
    draw_shapes(&painter, rect, doc, a4);
    draw_pending(&painter, rect, doc, state, a4);

    // Tool input (left button).
    if let Some(pos) = response.interact_pointer_pos() {
        let p = maybe_snap(screen_to_point(pos, rect, doc.time, doc.pitch, a4), state);
        if response.clicked_by(egui::PointerButton::Primary) {
            handle_click(doc, state, p);
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && matches!(
                state.tool,
                Tool::Line | Tool::Rectangle | Tool::Circle | Tool::Ellipse | Tool::Arc
            )
        {
            state.drag_start = Some(p);
        }
        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some(start) = state.drag_start.take()
        {
            handle_drag(doc, state, start, p);
        }
    }
    // Escape cancels a pending multi-click shape.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.pending.clear();
        state.drag_start = None;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        finish_pending(doc, state);
    }

    response
}

fn draw_grid(painter: &egui::Painter, rect: Rect, state: &CanvasState, doc: &Document) {
    if !state.show_grid {
        return;
    }
    let stroke = Stroke::new(1.0, Color32::from_gray(40));
    if let (Some(sub), Some(tempo)) = (state.subdivision, Tempo::new(state.bpm)) {
        let step = sub.step_s(tempo);
        if step > 0.0 && doc.time.width_s / step < 400.0 {
            let mut t = (doc.time.start_s / step).floor() * step;
            while t <= doc.time.start_s + doc.time.width_s {
                let x = point_to_screen(Point::new(t, 0.0), rect, doc.time, doc.pitch, 440.0).x;
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    stroke,
                );
                t += step;
            }
        }
    }
    // Pitch grid at each semitone, labelled in semitone mode (GRID-04).
    let lo_st = synthesis_engine::pitch::hz_to_st(doc.pitch.lo_hz, 440.0).floor() as i32;
    let hi_st = synthesis_engine::pitch::hz_to_st(doc.pitch.hi_hz, 440.0).ceil() as i32;
    for midi in lo_st..=hi_st {
        let p = point_to_screen(
            Point::new(0.0, f64::from(midi)),
            rect,
            doc.time,
            doc.pitch,
            440.0,
        );
        if p.y < rect.top() || p.y > rect.bottom() {
            continue;
        }
        painter.line_segment(
            [Pos2::new(rect.left(), p.y), Pos2::new(rect.right(), p.y)],
            stroke,
        );
        if doc.pitch.scale == PitchScale::Semitones && midi % 12 == 0 {
            painter.text(
                Pos2::new(rect.left() + 4.0, p.y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                note_name(midi),
                egui::FontId::monospace(11.0),
                Color32::from_gray(140),
            );
        }
    }
}

fn draw_shapes(painter: &egui::Painter, rect: Rect, doc: &Document, a4: f64) {
    for (vi, voice) in doc.scene.voices.iter().enumerate() {
        let color = voice_color(vi);
        for shape in &voice.shapes {
            for seg in shape.edges() {
                let a = point_to_screen(seg.a, rect, doc.time, doc.pitch, a4);
                let b = point_to_screen(seg.b, rect, doc.time, doc.pitch, a4);
                painter.line_segment([a, b], Stroke::new(2.0, color));
            }
            if shape.filled {
                draw_fill(painter, rect, shape, doc, a4, color);
            }
        }
    }
}

/// Fill visualisation (CANVAS-04): a light tint over the bounding box of a closed shape.
fn draw_fill(
    painter: &egui::Painter,
    rect: Rect,
    shape: &Shape,
    doc: &Document,
    a4: f64,
    color: Color32,
) {
    let (verts, _) = shape.vertices();
    if verts.is_empty() {
        return;
    }
    let mut min = Pos2::new(f32::MAX, f32::MAX);
    let mut max = Pos2::new(f32::MIN, f32::MIN);
    for v in &verts {
        let p = point_to_screen(*v, rect, doc.time, doc.pitch, a4);
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    let tint = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 36);
    painter.rect_filled(Rect::from_min_max(min, max), 0.0, tint);
}

fn draw_pending(painter: &egui::Painter, rect: Rect, doc: &Document, state: &CanvasState, a4: f64) {
    let stroke = Stroke::new(1.5, Color32::WHITE);
    let pts: Vec<Pos2> = state
        .pending
        .iter()
        .map(|p| point_to_screen(*p, rect, doc.time, doc.pitch, a4))
        .collect();
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], stroke);
    }
    for p in &pts {
        painter.circle_stroke(*p, 3.0, stroke);
    }
    // A selected-highlight rect for the drag preview.
    if let Some(start) = state.drag_start {
        let p = point_to_screen(start, rect, doc.time, doc.pitch, a4);
        painter.rect_stroke(
            Rect::from_center_size(p, Vec2::splat(6.0)),
            0.0,
            stroke,
            StrokeKind::Inside,
        );
    }
}

fn voice_color(index: usize) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(230, 110, 90),
        Color32::from_rgb(90, 170, 230),
        Color32::from_rgb(120, 210, 120),
        Color32::from_rgb(230, 200, 90),
        Color32::from_rgb(180, 120, 230),
        Color32::from_rgb(90, 220, 200),
    ];
    PALETTE[index % PALETTE.len()]
}

/// A single click: adds a point for polyline and polygon, or runs the
/// one-click tools (delete, fill toggle).
fn handle_click(doc: &mut Document, state: &mut CanvasState, p: Point) {
    match state.tool {
        Tool::Polyline | Tool::Polygon => state.pending.push(p),
        Tool::Delete => {
            let idx = doc.selected_voice;
            let hit = doc
                .scene
                .voices
                .get(idx)
                .and_then(|v| v.shapes.iter().position(|s| hit_shape(s, p)));
            if let Some(si) = hit {
                doc.edit(|s| {
                    s.voices[idx].shapes.remove(si);
                });
                state.status = "Deleted shape".into();
            }
        }
        Tool::Fill => {
            let idx = doc.selected_voice;
            let hit = doc.scene.voices.get(idx).and_then(|v| {
                v.shapes
                    .iter()
                    .position(|s| s.is_closed() && contains_point(s, p))
            });
            match hit {
                Some(si) => {
                    doc.edit(|s| {
                        let shape = &mut s.voices[idx].shapes[si];
                        shape.filled = !shape.filled;
                    });
                    state.status = "Toggled fill".into();
                }
                None => state.status = "Click inside a closed shape to toggle fill".into(),
            }
        }
        _ => {}
    }
}

/// Whether `p` lies inside a closed shape (even-odd rule over its edges).
fn contains_point(shape: &Shape, p: Point) -> bool {
    let mut inside = false;
    for seg in shape.edges() {
        let (a, b) = (seg.a, seg.b);
        if (a.st > p.st) != (b.st > p.st) {
            let t_cross = a.t + (p.st - a.st) / (b.st - a.st) * (b.t - a.t);
            if p.t < t_cross {
                inside = !inside;
            }
        }
    }
    inside
}

/// Whether a point is near a shape's edges (or inside a closed shape). Used for delete hit-testing.
fn hit_shape(shape: &Shape, p: Point) -> bool {
    let tol_t = 0.02;
    let tol_st = 0.5;
    for seg in shape.edges() {
        let t0 = seg.a.t.min(seg.b.t) - tol_t;
        let t1 = seg.a.t.max(seg.b.t) + tol_t;
        let s0 = seg.a.st.min(seg.b.st) - tol_st;
        let s1 = seg.a.st.max(seg.b.st) + tol_st;
        if p.t >= t0 && p.t <= t1 && p.st >= s0 && p.st <= s1 {
            return true;
        }
    }
    false
}

/// Finish a drag-defined shape (line, rectangle, circle, ellipse, arc).
fn handle_drag(doc: &mut Document, state: &mut CanvasState, a: Point, b: Point) {
    let fill = state.fill_new;
    let geom = match state.tool {
        Tool::Line => Geometry::Line { a, b },
        Tool::Rectangle => Geometry::Rectangle { a, b },
        Tool::Circle => {
            let r_s = ((b.t - a.t).abs()).max(1e-3);
            let r_st = ((b.st - a.st).abs()).max(1e-3);
            Geometry::Circle {
                center: a,
                radius_s: r_s,
                radius_st: r_st,
            }
        }
        Tool::Ellipse => Geometry::Ellipse {
            center: a,
            rx_s: ((b.t - a.t).abs()).max(1e-3),
            ry_st: ((b.st - a.st).abs()).max(1e-3),
        },
        Tool::Arc => Geometry::Arc {
            center: a,
            rx_s: ((b.t - a.t).abs()).max(1e-3),
            ry_st: ((b.st - a.st).abs()).max(1e-3),
            start_rad: 0.0,
            sweep_rad: std::f64::consts::PI,
        },
        _ => return,
    };
    add_geometry(doc, state, geom, fill);
}

/// Commit pending points for polyline (open) or polygon (closed) on Enter.
fn finish_pending(doc: &mut Document, state: &mut CanvasState) {
    let pts = std::mem::take(&mut state.pending);
    let fill = state.fill_new;
    let geom = match state.tool {
        Tool::Polyline if pts.len() >= 2 => Geometry::Polyline { points: pts },
        Tool::Polygon if pts.len() >= 3 => Geometry::Polygon { points: pts },
        _ => return,
    };
    add_geometry(doc, state, geom, fill);
}

fn add_geometry(doc: &mut Document, state: &mut CanvasState, geom: Geometry, fill: bool) {
    match Shape::new(geom, fill) {
        Ok(shape) => {
            if doc.add_shape(shape) {
                state.status = "Shape added".into();
            } else {
                state.status = "Shape rejected".into();
            }
        }
        Err(e) => state.status = format!("Invalid shape: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Pos2, Rect};
    use synthesis_engine::PitchScale;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::new(40.0, 20.0), Vec2::new(800.0, 400.0))
    }

    /// Screen to point and back must return the same point in every scale mode.
    #[test]
    fn screen_mapping_round_trips_in_every_scale() {
        let time = TimeView {
            start_s: 1.5,
            width_s: 4.0,
        };
        for scale in [
            PitchScale::Semitones,
            PitchScale::LogHz,
            PitchScale::LinearHz,
        ] {
            let pitch = PitchView {
                lo_hz: 100.0,
                hi_hz: 1600.0,
                scale,
            };
            let p = Point::new(2.25, 64.0);
            let screen = point_to_screen(p, rect(), time, pitch, 440.0);
            let back = screen_to_point(screen, rect(), time, pitch, 440.0);
            assert!((back.t - p.t).abs() < 1e-3, "{scale:?} time {}", back.t);
            assert!((back.st - p.st).abs() < 1e-3, "{scale:?} pitch {}", back.st);
        }
    }

    #[test]
    fn time_axis_is_left_to_right_and_pitch_is_bottom_to_top() {
        let time = TimeView {
            start_s: 0.0,
            width_s: 2.0,
        };
        let pitch = PitchView::default();
        let lo = point_to_screen(Point::new(0.0, 60.0), rect(), time, pitch, 440.0);
        let hi = point_to_screen(Point::new(2.0, 80.0), rect(), time, pitch, 440.0);
        assert!(hi.x > lo.x, "later time is further right");
        assert!(hi.y < lo.y, "higher pitch is further up");
    }

    #[test]
    fn contains_point_detects_inside_and_outside() {
        let rect = Shape::outline(synthesis_engine::Geometry::Rectangle {
            a: Point::new(0.0, 60.0),
            b: Point::new(2.0, 64.0),
        })
        .unwrap();
        assert!(contains_point(&rect, Point::new(1.0, 62.0)));
        assert!(!contains_point(&rect, Point::new(3.0, 62.0)));
        assert!(!contains_point(&rect, Point::new(1.0, 70.0)));
    }

    #[test]
    fn fill_tool_toggles_closed_shape_under_click() {
        let mut doc = Document::default();
        let shape = Shape::outline(synthesis_engine::Geometry::Rectangle {
            a: Point::new(0.0, 60.0),
            b: Point::new(2.0, 64.0),
        })
        .unwrap();
        assert!(doc.add_shape(shape));
        let mut state = CanvasState {
            tool: Tool::Fill,
            ..CanvasState::default()
        };
        handle_click(&mut doc, &mut state, Point::new(1.0, 62.0));
        assert!(
            doc.scene.voices[0].shapes[0].filled,
            "click inside should fill"
        );
        handle_click(&mut doc, &mut state, Point::new(1.0, 62.0));
        assert!(
            !doc.scene.voices[0].shapes[0].filled,
            "second click should unfill"
        );
        // Click outside any shape changes nothing.
        handle_click(&mut doc, &mut state, Point::new(9.0, 62.0));
        assert!(!doc.scene.voices[0].shapes[0].filled);
    }

    #[test]
    fn snap_rounds_time_to_grid_and_pitch_to_semitone() {
        let state = CanvasState {
            snap: true,
            bpm: 120.0,
            subdivision: Some(Subdivision::Beat),
            ..CanvasState::default()
        };
        // Beat is 0.5 s at 120 BPM.
        let p = maybe_snap(Point::new(0.74, 60.4), &state);
        assert!((p.t - 0.5).abs() < 1e-12);
        assert!((p.st - 60.0).abs() < 1e-12);
    }
}

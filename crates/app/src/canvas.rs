//! Pitch-time sketch canvas (CANVAS-01 to 07, GRID-01 to 04, R8).
//!
//! Tools behave like Onshape's sketch tools:
//! - Every drawing tool works either click-click or press-drag-release, with a
//!   rubber-band preview that follows the pointer.
//! - Line and Polyline chain segments: each click adds a point. Double-click,
//!   Enter, or clicking the last point finishes; Escape cancels.
//! - Rectangle: two opposite corners. Circle and Polygon: centre, then a point
//!   on the circle. Ellipse: centre, then a bounding-box corner.
//! - Arc is a three-point arc: start, end, then a point it passes through.
//! - Fill toggles a closed shape between outline and filled (CANVAS-03).
//! - Select: click picks the nearest shape (within a few pixels), Shift-click
//!   adds or removes, dragging empty space draws a selection box, dragging a
//!   shape moves it (and anything else selected), and dragging a handle edits
//!   that point. Rectangles have corner and edge-midpoint handles; an edge
//!   handle moves that side only. Delete removes. With a drawing tool, pressing
//!   on an existing point starts the new shape there (it snaps).
//!
//! Hit tolerances are in screen pixels, so they feel the same at every zoom.
//! Snapping prefers existing points within a few pixels, then the grid.
//! The fill display comes from the same band computation as the audio, so
//! what looks filled is exactly what sounds as a band.

use std::collections::BTreeSet;

use eframe::egui::{
    self, Color32, CursorIcon, Mesh, Pos2, Rect, Response, Sense, Shape as EShape, Stroke,
    StrokeKind, Ui, Vec2,
};
use synthesis_engine::pitchscale::{PitchScale, note_name};
use synthesis_engine::sampling::fill_bands_edges;
use synthesis_engine::sketch::{self, Handle, HandleKind};
use synthesis_engine::{Division, Geometry, Point, Shape, Tempo};

use crate::document::Document;
use crate::view::{self, View};

/// Pick radius for shapes, in screen points.
pub const PICK_PX: f32 = 7.0;
/// Pick radius for handles, in screen points.
pub const HANDLE_PX: f32 = 9.0;
/// Snap radius for existing points, in screen points.
pub const SNAP_PX: f32 = 10.0;

/// Drawing and editing tools (CANVAS-02).
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
            Tool::Rectangle => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Ellipse => "Ellipse",
            Tool::Arc => "3-point arc",
            Tool::Polygon => "Polygon",
            Tool::Fill => "Fill",
            Tool::Delete => "Delete",
        }
    }

    /// Keyboard shortcut, as in Onshape where one exists.
    pub fn shortcut(self) -> Option<egui::Key> {
        use egui::Key;
        match self {
            Tool::Select => Some(Key::S),
            Tool::Line => Some(Key::L),
            Tool::Polyline => Some(Key::P),
            Tool::Rectangle => Some(Key::G),
            Tool::Circle => Some(Key::C),
            Tool::Ellipse => Some(Key::E),
            Tool::Arc => Some(Key::A),
            Tool::Polygon => Some(Key::O),
            Tool::Fill => Some(Key::F),
            Tool::Delete => None,
        }
    }

    /// What the tool expects next, shown in the status bar.
    pub fn hint(self, clicks: usize) -> &'static str {
        match (self, clicks) {
            (Tool::Select, _) => {
                "Click a shape to select (Shift adds). Drag a shape to move, a handle to edit, empty space to box-select. Delete removes."
            }
            (Tool::Line, 0) => "Click or drag: start of line",
            (Tool::Line, _) => "Click: end of line",
            (Tool::Polyline, 0) => "Click: first point",
            (Tool::Polyline, _) => {
                "Click: next point. Double-click, Enter, or click the last point to finish. Esc cancels."
            }
            (Tool::Rectangle, 0) => "Click or drag: first corner",
            (Tool::Rectangle, _) => "Click: opposite corner",
            (Tool::Circle, 0) => "Click or drag: centre",
            (Tool::Circle, _) => "Click: radius",
            (Tool::Ellipse, 0) => "Click or drag: centre",
            (Tool::Ellipse, _) => "Click: corner (sets both radii)",
            (Tool::Arc, 0) => "Click or drag: arc start",
            (Tool::Arc, 1) => "Click: arc end",
            (Tool::Arc, _) => "Move to bend the arc, click to place",
            (Tool::Polygon, 0) => "Click or drag: centre",
            (Tool::Polygon, _) => "Click: corner (sets size and rotation)",
            (Tool::Fill, _) => "Click inside a closed shape to toggle outline / filled",
            (Tool::Delete, _) => "Click a shape to delete it",
        }
    }

    fn draws(self) -> bool {
        !matches!(self, Tool::Select | Tool::Fill | Tool::Delete)
    }
}

/// An in-progress pointer drag on the canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum Drag {
    /// Move the selected shapes by the pointer delta from `origin`.
    Move { origin: Point },
    /// Drag one handle of one shape.
    Handle { shape: usize, kind: HandleKind },
    /// Box selection from `origin` (screen), adding to `base`.
    Box { origin: Pos2, base: BTreeSet<usize> },
    /// A drawing tool's press-drag gesture, started at the first point.
    Draw,
    /// Middle- or right-button pan, or Space-drag.
    Pan,
}

/// Canvas UI state (not part of the scene).
pub struct CanvasState {
    pub tool: Tool,
    /// New closed shapes are created filled (CANVAS-03 default).
    pub fill_new: bool,
    pub snap: bool,
    pub show_grid: bool,
    pub bpm: f64,
    /// X grid: a `1/N` fraction of a beat (snap and grid lines).
    pub x_div: Division,
    /// Y grid: a `1/N` fraction of a semitone (or of an octave in Hz modes).
    pub y_div: Division,
    pub polygon_sides: usize,
    /// Points placed so far by the active drawing tool.
    pub pending: Vec<Point>,
    pub selected: BTreeSet<usize>,
    pub drag: Option<Drag>,
    /// Shape index under the pointer, for hover highlighting.
    pub hover: Option<usize>,
    /// Snapped pointer position (timeline units), when over the canvas.
    pub cursor: Option<Point>,
    /// Whether `cursor` snapped to an existing point.
    pub cursor_on_point: bool,
    /// Last action message, set through `status`; shown for a few seconds.
    pub status: String,
    /// `status` with the time it was set (filled in each frame).
    pub message: Option<(String, f64)>,
    /// Screen rectangle of the canvas last frame (used by the playhead
    /// overlay and the UI tests).
    pub rect: Option<Rect>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            fill_new: false,
            snap: true,
            show_grid: true,
            bpm: 120.0,
            x_div: Division::new(16).unwrap(),
            y_div: Division::new(1).unwrap(),
            polygon_sides: sketch::DEFAULT_POLYGON_SIDES,
            pending: Vec::new(),
            selected: BTreeSet::new(),
            drag: None,
            hover: None,
            cursor: None,
            cursor_on_point: false,
            status: String::new(),
            message: None,
            rect: None,
        }
    }
}

impl CanvasState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Switch tools, cancelling anything in progress.
    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.pending.clear();
        self.drag = None;
    }

    /// Cancel the current operation; with nothing in progress, clear the selection.
    pub fn escape(&mut self) {
        if self.drag.is_some() || !self.pending.is_empty() {
            self.drag = None;
            self.pending.clear();
        } else if self.tool != Tool::Select {
            self.set_tool(Tool::Select);
        } else {
            self.selected.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry in screen space.
// ---------------------------------------------------------------------------

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let u = if len2 <= f32::EPSILON {
        0.0
    } else {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    };
    (p - (a + ab * u)).length()
}

/// The shape's edges in screen space.
fn screen_edges(shape: &Shape, v: &View) -> Vec<(Pos2, Pos2)> {
    shape
        .edges()
        .iter()
        .map(|e| (v.point_to_screen(e.a), v.point_to_screen(e.b)))
        .collect()
}

/// Distance in screen points from `p` to the shape's outline.
pub fn outline_distance(shape: &Shape, v: &View, p: Pos2) -> f32 {
    screen_edges(shape, v)
        .iter()
        .map(|(a, b)| dist_to_segment(p, *a, *b))
        .fold(f32::INFINITY, f32::min)
}

/// Whether the timeline point `p` is inside a closed shape (even-odd rule, the
/// same rule that decides which frequencies a filled shape sounds).
pub fn contains(shape: &Shape, p: Point) -> bool {
    if !shape.is_closed() {
        return false;
    }
    fill_bands_edges(&shape.edges(), p.t)
        .iter()
        .any(|&(lo, hi)| lo <= p.st && p.st <= hi)
}

/// The shape under screen point `p`: the nearest outline within `PICK_PX`,
/// otherwise the topmost filled shape containing it.
pub fn pick(shapes: &[Shape], v: &View, p: Pos2) -> Option<usize> {
    let near = shapes
        .iter()
        .enumerate()
        .map(|(i, s)| (i, outline_distance(s, v, p)))
        .filter(|(_, d)| *d <= PICK_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
    if let Some((i, _)) = near {
        return Some(i);
    }
    let tp = v.screen_to_point(p);
    (0..shapes.len())
        .rev()
        .find(|&i| shapes[i].filled && contains(&shapes[i], tp))
}

/// The innermost closed shape containing screen point `p` (for the Fill tool),
/// or a closed outline under the pointer.
pub fn pick_closed(shapes: &[Shape], v: &View, p: Pos2) -> Option<usize> {
    let tp = v.screen_to_point(p);
    let area = |s: &Shape| {
        let (pts, _) = s.vertices();
        let mut a = 0.0;
        for i in 0..pts.len() {
            let (p0, p1) = (
                v.point_to_screen(pts[i]),
                v.point_to_screen(pts[(i + 1) % pts.len()]),
            );
            a += f64::from(p0.x * p1.y - p1.x * p0.y);
        }
        (a / 2.0).abs()
    };
    let inside = shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_closed() && contains(s, tp))
        .min_by(|a, b| area(a.1).total_cmp(&area(b.1)))
        .map(|(i, _)| i);
    inside.or_else(|| pick(shapes, v, p).filter(|&i| shapes[i].is_closed()))
}

/// The handle under `p` among the given shapes, nearest first.
fn pick_handle(
    shapes: &[Shape],
    among: impl Iterator<Item = usize>,
    v: &View,
    p: Pos2,
) -> Option<(usize, Handle)> {
    among
        .filter_map(|i| shapes.get(i).map(|s| (i, s)))
        .flat_map(|(i, s)| sketch::handles(s).into_iter().map(move |h| (i, h)))
        .map(|(i, h)| (i, h, (v.point_to_screen(h.pos) - p).length()))
        .filter(|(_, _, d)| *d <= HANDLE_PX)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(i, h, _)| (i, h))
}

/// Shapes whose bounding box lies inside the screen rectangle `r`.
fn shapes_in_box(shapes: &[Shape], v: &View, r: Rect) -> BTreeSet<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            let (pts, _) = s.vertices();
            !pts.is_empty() && pts.iter().all(|p| r.contains(v.point_to_screen(*p)))
        })
        .map(|(i, _)| i)
        .collect()
}

// ---------------------------------------------------------------------------
// Snapping.
// ---------------------------------------------------------------------------

/// Snap a screen position. Existing points (vertices, centres, arc points,
/// pending points) within `SNAP_PX` win; otherwise the grid if snapping is on.
/// Hold Alt (Option) to disable snapping. Returns the point and whether it
/// landed on an existing point.
pub fn snap(
    pos: Pos2,
    v: &View,
    state: &CanvasState,
    shapes: &[Shape],
    exclude: Option<usize>,
    alt: bool,
) -> (Point, bool) {
    let raw = v.screen_to_point(pos);
    if alt {
        return (raw, false);
    }
    let mut best: Option<(f32, Point)> = None;
    let mut consider = |p: Point| {
        let d = (v.point_to_screen(p) - pos).length();
        if d <= SNAP_PX && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, p));
        }
    };
    for (i, s) in shapes.iter().enumerate() {
        if Some(i) == exclude {
            continue;
        }
        for h in sketch::handles(s) {
            if !matches!(h.kind, HandleKind::Radius(_)) {
                consider(h.pos);
            }
        }
    }
    for p in &state.pending {
        consider(*p);
    }
    if let Some((_, p)) = best {
        return (p, true);
    }
    if !state.snap {
        return (raw, false);
    }
    let t = match Tempo::new(state.bpm) {
        Some(tempo) => {
            let step = state.x_div.x_step_s(tempo);
            // Don't snap to grid lines closer than 4 px apart.
            if step / v.s_per_px() >= 4.0 {
                (raw.t / step).round() * step
            } else {
                raw.t
            }
        }
        None => raw.t,
    };
    (
        Point::new(t, snap_pitch(raw.st, v, state.y_div, pos.y)),
        false,
    )
}

/// Snap a pitch (fractional MIDI) to the Y division. In semitone mode the step
/// is `1/N` semitone; in the Hz modes it is `1/N` octave, which is the same
/// grid on a log axis. Skipped when the step is smaller than 4 screen points.
pub fn snap_pitch(st: f64, v: &View, y: Division, screen_y: f32) -> f64 {
    let step = match v.pitch.scale {
        PitchScale::Semitones => y.y_step_st(),
        PitchScale::LogHz | PitchScale::LinearHz => {
            12.0 * synthesis_engine::grid::y_step_octaves(y)
        }
    };
    if step / v.st_per_px(screen_y) < 4.0 {
        return st;
    }
    (st / step).round() * step
}

// ---------------------------------------------------------------------------
// Construction.
// ---------------------------------------------------------------------------

/// The geometry the active tool would create from `pending` plus `cursor`.
/// `k` is the view's seconds-per-semitone so round shapes look round.
pub fn construct(
    tool: Tool,
    pending: &[Point],
    cursor: Point,
    k: f64,
    sides: usize,
) -> Option<Geometry> {
    match (tool, pending) {
        (Tool::Line, [a]) => sketch::line(*a, cursor),
        (Tool::Rectangle, [a]) => sketch::rectangle(*a, cursor),
        (Tool::Circle, [c]) => sketch::circle(*c, cursor, k),
        (Tool::Ellipse, [c]) => sketch::ellipse(*c, cursor),
        (Tool::Polygon, [c]) => sketch::regular_polygon(*c, cursor, sides, k),
        (Tool::Arc, [a]) => sketch::line(*a, cursor),
        (Tool::Arc, [a, b]) => sketch::arc_3pt(*a, *b, cursor, k),
        (Tool::Polyline, pts) if !pts.is_empty() => {
            let mut points = pts.to_vec();
            points.push(cursor);
            (points.len() >= 2).then_some(Geometry::Polyline { points })
        }
        _ => None,
    }
}

/// Points a tool needs before the cursor completes the shape.
fn points_needed(tool: Tool) -> usize {
    match tool {
        Tool::Arc => 2,
        _ => 1,
    }
}

fn add_shape(doc: &mut Document, state: &mut CanvasState, geom: Geometry) -> bool {
    let fill = state.fill_new
        && matches!(
            geom,
            Geometry::Rectangle { .. }
                | Geometry::Circle { .. }
                | Geometry::Ellipse { .. }
                | Geometry::Polygon { .. }
        );
    match Shape::new(geom, fill) {
        Ok(shape) => {
            if doc.add_shape(shape) {
                let n = doc.scene.voices[doc.selected_voice].shapes.len();
                state.selected = BTreeSet::from([n - 1]);
                true
            } else {
                state.status = "Shape rejected: it would make the scene invalid".into();
                false
            }
        }
        Err(e) => {
            state.status = format!("Shape rejected: {e}");
            false
        }
    }
}

/// Finish a polyline from the pending points (two or more).
fn finish_polyline(doc: &mut Document, state: &mut CanvasState) {
    let pts = std::mem::take(&mut state.pending);
    if pts.len() >= 2 {
        add_shape(doc, state, Geometry::Polyline { points: pts });
    }
}

/// Apply a click (or press-drag-release endpoint) of a drawing tool at `p`.
pub fn tool_click(doc: &mut Document, state: &mut CanvasState, p: Point, k: f64) {
    if state.tool == Tool::Polyline {
        if let Some(last) = state.pending.last()
            && (last.t - p.t).abs() < 1e-9
            && (last.st - p.st).abs() < 1e-9
        {
            finish_polyline(doc, state);
            return;
        }
        state.pending.push(p);
        return;
    }
    if state.pending.len() < points_needed(state.tool) {
        state.pending.push(p);
        return;
    }
    match construct(state.tool, &state.pending, p, k, state.polygon_sides) {
        Some(geom) => {
            let line_end = p;
            if add_shape(doc, state, geom) && state.tool == Tool::Line {
                // Onshape chains lines: the next line starts where this one ended.
                state.pending = vec![line_end];
            } else {
                state.pending.clear();
            }
        }
        None => state.status = "Too small or degenerate: keep moving, or Esc to cancel".into(),
    }
}

// ---------------------------------------------------------------------------
// Edits.
// ---------------------------------------------------------------------------

fn voice_shapes(doc: &Document) -> &[Shape] {
    doc.scene
        .voices
        .get(doc.selected_voice)
        .map(|v| v.shapes.as_slice())
        .unwrap_or(&[])
}

/// The shapes as they would be if the current drag were released at `cursor`.
pub fn drag_preview(
    shapes: &[Shape],
    drag: &Drag,
    selected: &BTreeSet<usize>,
    cursor: Point,
    k: f64,
) -> Option<Vec<(usize, Shape)>> {
    match drag {
        Drag::Move { origin } => {
            let (dt, dst) = (cursor.t - origin.t, cursor.st - origin.st);
            selected
                .iter()
                .filter_map(|&i| shapes.get(i).map(|s| (i, s)))
                .map(|(i, s)| {
                    synthesis_engine::transform::translate(s, dt, dst)
                        .ok()
                        .map(|n| (i, n))
                })
                .collect()
        }
        Drag::Handle { shape, kind } => {
            let s = shapes.get(*shape)?;
            sketch::drag_handle(s, *kind, cursor, k)
                .ok()
                .map(|n| vec![(*shape, n)])
        }
        _ => None,
    }
}

fn commit_preview(doc: &mut Document, state: &mut CanvasState, changes: Vec<(usize, Shape)>) {
    if changes.is_empty() {
        return;
    }
    let voice = doc.selected_voice;
    let ok = doc
        .edit(|s| {
            for (i, shape) in changes {
                s.voices[voice].shapes[i] = shape;
            }
        })
        .is_some();
    if ok {
    } else {
        state.status = "Edit rejected: it would make the scene invalid".into();
    }
}

/// Delete the selected shapes as one undo step.
pub fn delete_selected(doc: &mut Document, state: &mut CanvasState) {
    if state.selected.is_empty() {
        return;
    }
    let voice = doc.selected_voice;
    let sel: Vec<usize> = state.selected.iter().rev().copied().collect();
    if doc
        .edit(|s| {
            for i in sel {
                if i < s.voices[voice].shapes.len() {
                    s.voices[voice].shapes.remove(i);
                }
            }
        })
        .is_some()
    {
        state.status = format!("Deleted {} shape(s)", state.selected.len());
        state.selected.clear();
    }
}

/// Toggle outline/filled on a closed shape (CANVAS-03).
pub fn toggle_fill(doc: &mut Document, state: &mut CanvasState, i: usize) {
    let voice = doc.selected_voice;
    if doc
        .edit(|s| {
            let shape = &mut s.voices[voice].shapes[i];
            if shape.is_closed() {
                shape.filled = !shape.filled;
            }
        })
        .is_some()
    {
        let filled = doc.scene.voices[voice].shapes[i].filled;
        state.status = if filled { "Filled" } else { "Outline" }.into();
    }
}

/// Toggle fill on every selected closed shape (the F key with Select).
pub fn toggle_fill_selected(doc: &mut Document, state: &mut CanvasState) {
    let voice = doc.selected_voice;
    let closed: Vec<usize> = state
        .selected
        .iter()
        .copied()
        .filter(|&i| voice_shapes(doc).get(i).is_some_and(Shape::is_closed))
        .collect();
    if closed.is_empty() {
        state.status = "Select a closed shape to fill".into();
        return;
    }
    let fill = !doc.scene.voices[voice].shapes[closed[0]].filled;
    if doc
        .edit(|s| {
            for &i in &closed {
                s.voices[voice].shapes[i].filled = fill;
            }
        })
        .is_some()
    {
        state.status = if fill { "Filled" } else { "Outline" }.into();
    }
}

// ---------------------------------------------------------------------------
// The canvas widget.
// ---------------------------------------------------------------------------

/// Draw the canvas and handle its input.
pub fn show(ui: &mut Ui, doc: &mut Document, state: &mut CanvasState, a4: f64) -> Response {
    let size = Vec2::new(ui.available_width(), ui.available_height().max(120.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(22));
    state.rect = Some(rect);

    navigate(ui, &response, rect, doc, state);
    let v = View {
        rect,
        time: doc.time,
        pitch: doc.pitch,
        a4,
    };

    // Drop selection indices that no longer exist (after undo, for example).
    let n = voice_shapes(doc).len();
    state.selected.retain(|&i| i < n);

    handle_pointer(ui, &response, &v, doc, state);
    handle_keys(ui, &response, doc, state);
    if !state.status.is_empty() {
        let now = ui.input(|i| i.time);
        state.message = Some((std::mem::take(&mut state.status), now));
    }

    paint(&painter, &v, doc, state);
    set_cursor(ui, &response, &v, doc, state);
    response
}

/// Pan and zoom (CANVAS-01): scroll pans (trackpad two-finger), Cmd/Ctrl +
/// scroll or pinch zooms at the pointer, a mouse wheel zooms at the pointer,
/// and middle-drag, right-drag, or Space-drag pans.
fn navigate(ui: &Ui, response: &Response, rect: Rect, doc: &mut Document, state: &mut CanvasState) {
    if let Some(at) = response.hover_pos() {
        // Raw scroll events, not egui's smoothed delta, so a mouse wheel
        // (line units) and a trackpad (point units) can be told apart.
        // Cmd/Ctrl-scroll and pinch arrive as zoom_delta instead.
        let (lines, points, zoom) = ui.input(|i| {
            let (mut lines, mut points) = (0.0f32, Vec2::ZERO);
            for e in &i.events {
                if let egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } = e
                    && !modifiers.command
                {
                    match unit {
                        egui::MouseWheelUnit::Line => lines += delta.y + delta.x,
                        egui::MouseWheelUnit::Point => points += *delta,
                        egui::MouseWheelUnit::Page => lines += 3.0 * (delta.y + delta.x),
                    }
                }
            }
            (lines, points, i.zoom_delta_2d())
        });
        if lines != 0.0 {
            // Mouse wheel zooms at the pointer, like a CAD canvas.
            let f = f64::from(1.15f32.powf(lines));
            view::zoom_at(&mut doc.time, &mut doc.pitch, rect, at, f, f);
        }
        if points != Vec2::ZERO {
            // Trackpad two-finger scroll pans.
            view::pan(&mut doc.time, &mut doc.pitch, rect, points);
        }
        if zoom != Vec2::splat(1.0) {
            let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
            // Shift+zoom: time only. Alt+zoom: pitch only.
            let ft = if alt { 1.0 } else { f64::from(zoom.x) };
            let fp = if shift { 1.0 } else { f64::from(zoom.y) };
            view::zoom_at(&mut doc.time, &mut doc.pitch, rect, at, ft, fp);
        }
    }
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    let pan_drag = response.dragged_by(egui::PointerButton::Middle)
        || response.dragged_by(egui::PointerButton::Secondary)
        || (space && response.dragged_by(egui::PointerButton::Primary));
    if pan_drag {
        if state.drag.is_none() || state.drag == Some(Drag::Pan) {
            state.drag = Some(Drag::Pan);
        }
        if state.drag == Some(Drag::Pan) {
            view::pan(&mut doc.time, &mut doc.pitch, rect, response.drag_delta());
        }
    }
    if state.drag == Some(Drag::Pan) && !ui.input(|i| i.pointer.any_down()) {
        state.drag = None;
    }
}

fn handle_pointer(
    ui: &Ui,
    response: &Response,
    v: &View,
    doc: &mut Document,
    state: &mut CanvasState,
) {
    let (mods, pointer) = ui.input(|i| (i.modifiers, i.pointer.latest_pos()));
    let over = pointer.filter(|p| v.rect.contains(*p));
    let primary = egui::PointerButton::Primary;
    let shapes_now = voice_shapes(doc).to_vec();

    // Hover and cursor.
    state.hover = over.and_then(|p| pick(&shapes_now, v, p));
    let exclude = match &state.drag {
        Some(Drag::Handle { shape, .. }) => Some(*shape),
        _ => None,
    };
    let snapped = over
        .or(response.interact_pointer_pos())
        .map(|p| snap(p, v, state, &shapes_now, exclude, mods.alt));
    state.cursor = snapped.map(|s| s.0);
    state.cursor_on_point = snapped.is_some_and(|s| s.1);
    let k = over.map_or(1.0, |p| v.k_at(p.y));

    if state.drag == Some(Drag::Pan) {
        return;
    }

    // Press: decide what a drag would do.
    if response.drag_started_by(primary)
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
    {
        let at = snap(press, v, state, &shapes_now, None, mods.alt).0;
        let candidates: Vec<usize> = if state.tool == Tool::Select {
            state
                .selected
                .iter()
                .copied()
                .chain(0..shapes_now.len())
                .collect()
        } else {
            (0..shapes_now.len()).collect()
        };
        // Only Select edits existing geometry. With a drawing tool, pressing on
        // an existing point starts the new shape there (snapped), as in Onshape.
        let handle = if state.tool == Tool::Select {
            pick_handle(&shapes_now, candidates.into_iter(), v, press)
        } else {
            None
        };
        if let Some((i, h)) = handle {
            state.selected = BTreeSet::from([i]);
            state.drag = Some(Drag::Handle {
                shape: i,
                kind: h.kind,
            });
        } else if state.tool == Tool::Select {
            match pick(&shapes_now, v, press) {
                Some(i) => {
                    if !state.selected.contains(&i) {
                        if !mods.shift {
                            state.selected.clear();
                        }
                        state.selected.insert(i);
                    }
                    state.drag = Some(Drag::Move { origin: at });
                }
                None => {
                    let base = if mods.shift {
                        state.selected.clone()
                    } else {
                        BTreeSet::new()
                    };
                    state.drag = Some(Drag::Box {
                        origin: press,
                        base,
                    });
                }
            }
        } else if state.tool.draws() {
            if state.pending.is_empty() {
                state.pending.push(at);
                state.drag = Some(Drag::Draw);
            }
        } else {
            state.drag = None;
        }
    }

    // Live box selection.
    if let (Some(Drag::Box { origin, base }), Some(p)) =
        (&state.drag, response.interact_pointer_pos())
    {
        let r = Rect::from_two_pos(*origin, p);
        let mut sel = base.clone();
        sel.extend(shapes_in_box(&shapes_now, v, r));
        state.selected = sel;
    }

    // Release.
    if response.drag_stopped_by(primary) {
        let end = response
            .interact_pointer_pos()
            .or(pointer)
            .map(|p| snap(p, v, state, &shapes_now, exclude, mods.alt).0);
        match (state.drag.take(), end) {
            (Some(Drag::Draw), Some(end)) => {
                // Press-drag-release: the release is the next click. A tiny
                // drag keeps the first point and waits for a click instead.
                let first = state.pending[0];
                let moved = (v.point_to_screen(first) - v.point_to_screen(end)).length() > 4.0;
                if moved {
                    tool_click(doc, state, end, k);
                }
            }
            (Some(drag @ (Drag::Move { .. } | Drag::Handle { .. })), Some(end)) => {
                if let Some(changes) = drag_preview(&shapes_now, &drag, &state.selected, end, k) {
                    commit_preview(doc, state, changes);
                } else {
                    state.status = "Edit rejected: the shape would be degenerate".into();
                }
            }
            _ => {}
        }
    }

    // Clicks.
    if response.clicked_by(primary) && !ui.input(|i| i.key_down(egui::Key::Space)) {
        let pos = response.interact_pointer_pos().unwrap_or_default();
        let at = snap(pos, v, state, &shapes_now, None, mods.alt).0;
        match state.tool {
            Tool::Select => match pick(&shapes_now, v, pos) {
                Some(i) if mods.shift || mods.command => {
                    if !state.selected.remove(&i) {
                        state.selected.insert(i);
                    }
                }
                Some(i) => state.selected = BTreeSet::from([i]),
                None if !mods.shift => state.selected.clear(),
                None => {}
            },
            Tool::Fill => match pick_closed(&shapes_now, v, pos) {
                Some(i) => toggle_fill(doc, state, i),
                None => state.status = "Click inside a closed shape to toggle fill".into(),
            },
            Tool::Delete => {
                if let Some(i) = pick(&shapes_now, v, pos) {
                    state.selected = BTreeSet::from([i]);
                    delete_selected(doc, state);
                }
            }
            _ => {
                if state.tool == Tool::Polyline
                    && response.double_clicked_by(primary)
                    && state.pending.len() >= 2
                {
                    // The first click of the double-click already added the
                    // point; finish here.
                    finish_polyline(doc, state);
                } else {
                    tool_click(doc, state, at, k);
                }
            }
        }
    }
}

fn handle_keys(ui: &Ui, response: &Response, doc: &mut Document, state: &mut CanvasState) {
    // Shortcuts apply unless a text field (BPM, etc.) has keyboard focus.
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let _ = response;
    let (esc, enter, del, mods) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            i.modifiers,
        )
    });
    if esc {
        state.escape();
    }
    if enter && state.tool == Tool::Polyline {
        finish_polyline(doc, state);
    }
    if del {
        delete_selected(doc, state);
    }
    if mods.command && ui.input(|i| i.key_pressed(egui::Key::A)) {
        state.selected = (0..voice_shapes(doc).len()).collect();
        return;
    }
    if mods.any() {
        return;
    }
    for tool in Tool::ALL {
        if let Some(key) = tool.shortcut()
            && ui.input(|i| i.key_pressed(key))
        {
            if tool == Tool::Fill && state.tool == Tool::Select && !state.selected.is_empty() {
                toggle_fill_selected(doc, state);
            } else {
                state.set_tool(tool);
            }
        }
    }
}

fn set_cursor(ui: &Ui, response: &Response, v: &View, doc: &Document, state: &CanvasState) {
    if !response.hovered() {
        return;
    }
    let icon = match &state.drag {
        Some(Drag::Pan) => CursorIcon::Grabbing,
        Some(Drag::Move { .. }) => CursorIcon::Grabbing,
        Some(Drag::Handle { .. }) => CursorIcon::Grabbing,
        Some(Drag::Box { .. }) | Some(Drag::Draw) => CursorIcon::Crosshair,
        None => {
            let pos = response.hover_pos().unwrap_or_default();
            let shapes = voice_shapes(doc);
            let on_handle = state.tool == Tool::Select
                && pick_handle(shapes, 0..shapes.len(), v, pos).is_some();
            if on_handle {
                CursorIcon::Grab
            } else {
                match state.tool {
                    Tool::Select if state.hover.is_some() => CursorIcon::Move,
                    Tool::Select => CursorIcon::Default,
                    Tool::Fill | Tool::Delete if state.hover.is_some() => CursorIcon::PointingHand,
                    Tool::Fill | Tool::Delete => CursorIcon::Default,
                    _ => CursorIcon::Crosshair,
                }
            }
        }
    };
    ui.ctx().set_cursor_icon(icon);
}

// ---------------------------------------------------------------------------
// Painting.
// ---------------------------------------------------------------------------

pub fn voice_color(index: usize) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(235, 120, 95),
        Color32::from_rgb(95, 175, 235),
        Color32::from_rgb(125, 210, 120),
        Color32::from_rgb(230, 200, 90),
        Color32::from_rgb(185, 125, 235),
        Color32::from_rgb(90, 220, 200),
    ];
    PALETTE[index % PALETTE.len()]
}

const SELECT_COLOR: Color32 = Color32::from_rgb(255, 205, 70);
const PREVIEW_COLOR: Color32 = Color32::from_rgb(235, 235, 235);

fn paint(p: &egui::Painter, v: &View, doc: &Document, state: &CanvasState) {
    paint_grid(p, v, state);
    let shapes = voice_shapes(doc);

    // Shapes, with the in-progress edit applied for preview.
    let preview: Vec<(usize, Shape)> = match (&state.drag, state.cursor) {
        (Some(d), Some(c)) => {
            let k = v.k_at(v.y_of(c.st));
            drag_preview(shapes, d, &state.selected, c, k).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    for (vi, voice) in doc.scene.voices.iter().enumerate() {
        let color = voice_color(vi);
        let is_current = vi == doc.selected_voice;
        for (si, base) in voice.shapes.iter().enumerate() {
            let shape = if is_current {
                preview
                    .iter()
                    .find(|(i, _)| *i == si)
                    .map_or(base, |(_, s)| s)
            } else {
                base
            };
            let selected = is_current && state.selected.contains(&si);
            let hovered = is_current && state.hover == Some(si) && state.drag.is_none();
            paint_shape(p, v, shape, color, selected, hovered);
        }
    }

    // Handles of selected shapes.
    for &i in &state.selected {
        if let Some(s) = preview
            .iter()
            .find(|(j, _)| *j == i)
            .map(|(_, s)| s)
            .or_else(|| shapes.get(i))
        {
            for h in sketch::handles(s) {
                let c = v.point_to_screen(h.pos);
                let r = Rect::from_center_size(c, Vec2::splat(8.0));
                p.rect_filled(r, 1.0, Color32::from_gray(25));
                p.rect_stroke(r, 1.0, Stroke::new(1.5, SELECT_COLOR), StrokeKind::Inside);
            }
        }
    }

    // Box selection.
    if let (Some(Drag::Box { origin, .. }), Some(c)) = (&state.drag, state.cursor) {
        let r = Rect::from_two_pos(*origin, v.point_to_screen(c));
        p.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 205, 70, 25));
        p.rect_stroke(r, 0.0, Stroke::new(1.0, SELECT_COLOR), StrokeKind::Inside);
    }

    // Drawing-tool rubber band.
    if state.tool.draws()
        && !state.pending.is_empty()
        && let Some(c) = state.cursor
    {
        let k = v.k_at(v.y_of(c.st));
        let dashed = |a: Pos2, b: Pos2| {
            p.extend(EShape::dashed_line(
                &[a, b],
                Stroke::new(1.0, Color32::from_gray(150)),
                5.0,
                4.0,
            ));
        };
        if let Some(g) = construct(state.tool, &state.pending, c, k, state.polygon_sides) {
            let fill = state.fill_new
                && !matches!(
                    g,
                    Geometry::Line { .. } | Geometry::Polyline { .. } | Geometry::Arc { .. }
                );
            if let Ok(s) = Shape::new(g, fill) {
                paint_shape(p, v, &s, PREVIEW_COLOR, false, false);
            }
        }
        // Construction lines, as Onshape shows them.
        let first = v.point_to_screen(state.pending[0]);
        match state.tool {
            Tool::Circle | Tool::Polygon | Tool::Ellipse => dashed(first, v.point_to_screen(c)),
            Tool::Arc if state.pending.len() == 2 => {
                dashed(first, v.point_to_screen(state.pending[1]));
            }
            _ => {}
        }
        for q in &state.pending {
            p.circle_filled(v.point_to_screen(*q), 3.5, PREVIEW_COLOR);
        }
    }

    // Cursor marker: a ring when snapped onto an existing point.
    if let Some(c) = state.cursor
        && (state.tool.draws() || state.drag.is_some())
        && v.rect.contains(v.point_to_screen(c))
    {
        let pos = v.point_to_screen(c);
        if state.cursor_on_point {
            p.circle_stroke(pos, 6.0, Stroke::new(1.5, SELECT_COLOR));
        } else {
            p.circle_filled(pos, 2.5, Color32::from_gray(200));
        }
    }

    // Pointer readout, bottom right.
    if let Some(c) = state.cursor {
        let hz = synthesis_engine::pitch::st_to_hz(c.st, v.a4);
        let midi = c.st.round() as i32;
        let cents = ((c.st - c.st.round()) * 100.0).round() as i32;
        let text = format!(
            "{:.3} s   {:.1} Hz   {}{:+}c",
            c.t,
            hz,
            note_name(midi),
            cents
        );
        p.text(
            v.rect.right_bottom() - Vec2::new(8.0, 6.0),
            egui::Align2::RIGHT_BOTTOM,
            text,
            egui::FontId::monospace(11.0),
            Color32::from_gray(170),
        );
    }
}

fn paint_shape(
    p: &egui::Painter,
    v: &View,
    shape: &Shape,
    color: Color32,
    selected: bool,
    hovered: bool,
) {
    let edges = shape.edges();
    if shape.filled {
        paint_fill(p, v, &edges, color);
    }
    let stroke_color = if selected { SELECT_COLOR } else { color };
    let width = if hovered || selected { 3.0 } else { 2.0 };
    let pts: Vec<Pos2> = {
        let (verts, closed) = shape.vertices();
        let mut s: Vec<Pos2> = verts.iter().map(|q| v.point_to_screen(*q)).collect();
        if closed && let Some(f) = s.first().copied() {
            s.push(f);
        }
        s
    };
    if hovered && !selected {
        p.add(EShape::line(
            pts.clone(),
            Stroke::new(6.0, Color32::from_rgba_unmultiplied(255, 255, 255, 40)),
        ));
    }
    p.add(EShape::line(pts, Stroke::new(width, stroke_color)));
}

/// Fill display: the even-odd bands the engine sounds, sampled every two
/// screen points across the shape, as vertical strips joined into a mesh.
fn paint_fill(p: &egui::Painter, v: &View, edges: &[synthesis_engine::Segment], color: Color32) {
    let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
    for e in edges {
        t0 = t0.min(e.a.t.min(e.b.t));
        t1 = t1.max(e.a.t.max(e.b.t));
    }
    if t0.partial_cmp(&t1) != Some(std::cmp::Ordering::Less) {
        return;
    }
    let x0 = v.x_of(t0).max(v.rect.left());
    let x1 = v.x_of(t1).min(v.rect.right());
    if x1 <= x0 {
        return;
    }
    let tint = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 70);
    let mut mesh = Mesh::default();
    let step = 2.0f32;
    let mut x = x0;
    while x < x1 {
        let xn = (x + step).min(x1);
        let t = v.t_of((x + xn) / 2.0);
        for (lo, hi) in fill_bands_edges(edges, t) {
            let (ya, yb) = (v.y_of(hi), v.y_of(lo));
            mesh.add_colored_rect(
                Rect::from_min_max(Pos2::new(x, ya.min(yb)), Pos2::new(xn, ya.max(yb))),
                tint,
            );
        }
        x = xn;
    }
    p.add(EShape::mesh(mesh));
}

fn paint_grid(p: &egui::Painter, v: &View, state: &CanvasState) {
    let minor = Stroke::new(1.0, Color32::from_gray(34));
    let beat = Stroke::new(1.0, Color32::from_gray(48));
    let bar = Stroke::new(1.0, Color32::from_gray(70));
    let octave = Stroke::new(1.0, Color32::from_gray(62));
    let r = v.rect;

    // Time grid from BPM (GRID-02): bars strongest, then beats, then the snap step.
    if state.show_grid
        && let Some(tempo) = Tempo::new(state.bpm)
    {
        let beat_s = tempo.beat_s();
        let bar_s = beat_s * Tempo::BEATS_PER_BAR;
        let sub_s = state.x_div.x_step_s(tempo);
        let first = (v.time.start_s.max(0.0) / sub_s).floor() as i64;
        let last = ((v.time.start_s + v.time.width_s) / sub_s).ceil() as i64;
        let dense = sub_s / v.s_per_px() < 6.0;
        for i in first..=last {
            let t = i as f64 * sub_s;
            if t < 0.0 {
                continue;
            }
            let is_bar = (t / bar_s - (t / bar_s).round()).abs() < 1e-6;
            let is_beat = (t / beat_s - (t / beat_s).round()).abs() < 1e-6;
            let stroke = if is_bar {
                bar
            } else if is_beat {
                beat
            } else if dense {
                continue;
            } else {
                minor
            };
            if !is_bar && beat_s / v.s_per_px() < 6.0 {
                continue;
            }
            let x = v.x_of(t);
            p.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], stroke);
        }
    }

    // Pitch grid at the Y division (GRID-03). Octaves (C notes in semitone
    // mode) are emphasised and labelled. Finer lines show when they have room.
    let (lo_st, hi_st) = (v.st_of(r.bottom()), v.st_of(r.top()));
    let step = match v.pitch.scale {
        PitchScale::Semitones => state.y_div.y_step_st(),
        PitchScale::LogHz | PitchScale::LinearHz => {
            12.0 * synthesis_engine::grid::y_step_octaves(state.y_div)
        }
    };
    let px_per_step = (step / v.st_per_px(r.center().y)) as f32;
    let label_every = if px_per_step >= 24.0 {
        1
    } else {
        (24.0 / px_per_step).ceil() as i64
    };
    let first = (lo_st / step).floor() as i64;
    let last = (hi_st / step).ceil() as i64;
    for i in first..=last {
        let st = i as f64 * step;
        let y = v.y_of(st);
        if y < r.top() || y > r.bottom() {
            continue;
        }
        // An octave is a C (a multiple of 12 semitones on the MIDI axis).
        let midi = st.round() as i32;
        let on_semitone = (st - st.round()).abs() < 1e-9;
        let is_octave = on_semitone && midi.rem_euclid(12) == 0;
        if state.show_grid && (px_per_step >= 4.0 || is_octave) {
            p.line_segment(
                [Pos2::new(r.left(), y), Pos2::new(r.right(), y)],
                if is_octave { octave } else { minor },
            );
        }
        let labelled = is_octave || i.rem_euclid(label_every) == 0;
        if !labelled {
            continue;
        }
        let text = match v.pitch.scale {
            PitchScale::Semitones => {
                if on_semitone {
                    note_name(midi)
                } else {
                    format!("{:+.0}c", (st - st.round()) * 100.0)
                }
            }
            PitchScale::LogHz | PitchScale::LinearHz => {
                format!("{:.0} Hz", synthesis_engine::pitch::st_to_hz(st, v.a4))
            }
        };
        let pos = Pos2::new(r.left() + 4.0, y - 2.0);
        let galley = p.layout_no_wrap(
            text,
            egui::FontId::monospace(12.0),
            Color32::from_gray(if is_octave { 215 } else { 150 }),
        );
        let bg = Rect::from_min_size(
            pos - Vec2::new(2.0, galley.size().y),
            galley.size() + Vec2::new(4.0, 2.0),
        );
        p.rect_filled(bg, 2.0, Color32::from_black_alpha(150));
        p.galley(
            pos - Vec2::new(0.0, galley.size().y),
            galley,
            Color32::WHITE,
        );
    }
}

/// Fit the view to all shapes in the current voice, or to the timeline start.
pub fn zoom_to_fit(doc: &mut Document) {
    let shapes = voice_shapes(doc);
    let pts: Vec<Point> = shapes.iter().flat_map(|s| s.vertices().0).collect();
    if pts.is_empty() {
        view::fit(
            &mut doc.time,
            &mut doc.pitch,
            (0.0, 4.0),
            (57.0, 81.0),
            doc.scene.settings.a4_hz,
        );
        return;
    }
    let t0 = pts.iter().map(|p| p.t).fold(f64::INFINITY, f64::min);
    let t1 = pts.iter().map(|p| p.t).fold(f64::NEG_INFINITY, f64::max);
    let s0 = pts.iter().map(|p| p.st).fold(f64::INFINITY, f64::min);
    let s1 = pts.iter().map(|p| p.st).fold(f64::NEG_INFINITY, f64::max);
    let a4 = doc.scene.settings.a4_hz;
    view::fit(&mut doc.time, &mut doc.pitch, (t0, t1), (s0, s1), a4);
}

#[cfg(test)]
mod snap_tests {
    use super::*;
    use eframe::egui::{Pos2, Rect};
    use synthesis_engine::{Division, Tempo};

    fn view() -> View {
        View {
            rect: Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1000.0, 500.0)),
            time: crate::document::TimeView {
                start_s: 0.0,
                width_s: 4.0,
            },
            pitch: crate::document::PitchView {
                lo_hz: 110.0,
                hi_hz: 1760.0,
                scale: PitchScale::Semitones,
            },
            a4: 440.0,
        }
    }

    /// The grid snap rounds to the typed X step at the given tempo.
    #[test]
    fn x_snap_uses_the_typed_division() {
        let v = view();
        let state = CanvasState {
            x_div: Division::new(8).unwrap(),
            bpm: 120.0,
            snap: true,
            ..CanvasState::default()
        };
        // `1/8` is an eighth of a beat: 0.5 s / 8 = 0.0625 s at 120 BPM.
        let step = state.x_div.x_step_s(Tempo::new(120.0).unwrap());
        assert!((step - 0.0625).abs() < 1e-12, "step {step}");
        // 0.6 s is at x = 150 px (1000 px / 4 s = 250 px/s); nearest eighth-beat is 0.625 s.
        let (p, on_point) = snap(Pos2::new(150.0, 250.0), &v, &state, &[], None, false);
        assert!(!on_point);
        assert!((p.t - 0.625).abs() < 1e-9, "snapped to {}", p.t);
    }

    /// The pitch snap rounds to the typed Y step when the step is wide enough to see.
    #[test]
    fn y_snap_uses_the_typed_division() {
        let v = view();
        // At this zoom a semitone is about 27 px, so 1/4 st is about 7 px: too
        // fine to snap. Zoom in pitch (1/4 st is then wide enough).
        let mut v4 = v;
        v4.pitch.lo_hz = 440.0 * ((60.0 - 69.0 - 6.0) / 12.0_f64).exp2();
        v4.pitch.hi_hz = 440.0 * ((60.0 - 69.0 + 6.0) / 12.0_f64).exp2();
        let st_px = v4.st_per_px(250.0);
        assert!(st_px < 0.05, "zoomed in: {st_px} st per px");
        let y = Division::new(4).unwrap();
        let st = snap_pitch(60.3, &v4, y, 250.0);
        assert!((st - 60.25).abs() < 1e-9, "snapped to {st}");
        // At a coarse zoom the step is under 4 px, so the pitch is left alone.
        let coarse = snap_pitch(60.3, &v, Division::new(4).unwrap(), 250.0);
        assert!(
            (coarse - 60.3).abs() < 1e-12,
            "fine steps are not snapped at coarse zoom"
        );
    }
}

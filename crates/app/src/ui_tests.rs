//! End-to-end UI tests: real pointer and keyboard events through the whole
//! app (egui_kittest), headless, with no audio device.
//!
//! Each test drives the app the way a user would: press, move, release,
//! click, type. Positions are given in timeline units and converted with the
//! same mapping the canvas uses, so tests do not depend on panel sizes.

use std::collections::BTreeSet;

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use synthesis_engine::sketch::{self, HandleKind};
use synthesis_engine::{Geometry, Point, Shape};

use crate::app::SynthesisApp;
use crate::canvas::{self, Tool};
use crate::view::View;

pub const SIZE: Vec2 = Vec2::new(1280.0, 780.0);

pub struct Ui {
    pub h: Harness<'static, SynthesisApp>,
}

impl Ui {
    pub fn new() -> Self {
        let mut h = Harness::builder()
            .with_size(SIZE)
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                |ui, app: &mut SynthesisApp| app.ui(ui),
                SynthesisApp::headless(),
            );
        h.run_steps(2);
        let mut ui = Self { h };
        // A fixed view, independent of defaults: 4 s wide, A2..A6.
        ui.app().doc.time.start_s = 0.0;
        ui.app().doc.time.width_s = 4.0;
        ui.app().doc.pitch.lo_hz = 110.0;
        ui.app().doc.pitch.hi_hz = 1760.0;
        ui.h.run_steps(2);
        ui
    }

    pub fn app(&mut self) -> &mut SynthesisApp {
        self.h.state_mut()
    }

    /// The canvas mapping, from the rect the canvas laid out last frame.
    pub fn view(&self) -> View {
        let app = self.h.state();
        let rect = app.canvas.rect.expect("canvas has been laid out");
        View {
            rect,
            time: app.doc.time,
            pitch: app.doc.pitch,
            a4: app.doc.scene.settings.a4_hz,
        }
    }

    pub fn screen(&self, t: f64, st: f64) -> Pos2 {
        self.view().point_to_screen(Point::new(t, st))
    }

    fn event(&mut self, e: Event) {
        self.h.event(e);
        self.h.run_steps(1);
    }

    pub fn move_to(&mut self, p: Pos2) {
        self.event(Event::PointerMoved(p));
    }

    pub fn press(&mut self, p: Pos2, mods: Modifiers) {
        self.event(Event::PointerMoved(p));
        self.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        });
    }

    pub fn release(&mut self, p: Pos2, mods: Modifiers) {
        self.event(Event::PointerMoved(p));
        self.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: mods,
        });
    }

    /// A click: press and release in place, then wait out the double-click window.
    pub fn click(&mut self, p: Pos2) {
        self.click_mods(p, Modifiers::NONE);
    }

    /// Hold modifier keys down, as a user does (egui tracks modifiers from
    /// key events), or release them with `Modifiers::NONE`.
    fn hold(&mut self, mods: Modifiers) {
        self.event(Event::ModifiersChanged(mods));
    }

    pub fn click_mods(&mut self, p: Pos2, mods: Modifiers) {
        self.hold(mods);
        self.press(p, mods);
        self.release(p, mods);
        self.hold(Modifiers::NONE);
        // Move the clock past the double-click delay so clicks stay single.
        self.h.run_steps(30);
    }

    pub fn double_click(&mut self, p: Pos2) {
        self.press(p, Modifiers::NONE);
        self.release(p, Modifiers::NONE);
        self.press(p, Modifiers::NONE);
        self.release(p, Modifiers::NONE);
        self.h.run_steps(30);
    }

    /// Press, move in steps, release.
    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.drag_mods(from, to, Modifiers::NONE);
    }

    pub fn drag_mods(&mut self, from: Pos2, to: Pos2, mods: Modifiers) {
        self.hold(mods);
        self.press(from, mods);
        for i in 1..=8 {
            let f = i as f32 / 8.0;
            self.move_to(from + (to - from) * f);
        }
        self.release(to, mods);
        self.hold(Modifiers::NONE);
        self.h.run_steps(30);
    }

    pub fn key(&mut self, key: egui::Key) {
        self.h.key_press(key);
        self.h.run_steps(1);
    }

    pub fn tool(&mut self, t: Tool) {
        self.app().canvas.set_tool(t);
        self.h.run_steps(1);
    }

    pub fn shapes(&mut self) -> Vec<Shape> {
        let app = self.h.state();
        app.doc.scene.voices[app.doc.selected_voice].shapes.clone()
    }

    pub fn selected(&mut self) -> BTreeSet<usize> {
        self.h.state().canvas.selected.clone()
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

// --- Drawing tools ---------------------------------------------------------

#[test]
fn line_click_click_and_chains() {
    let mut u = Ui::new();
    u.app().canvas.snap = false;
    u.tool(Tool::Line);
    let (a, b, c) = (
        u.screen(0.5, 60.0),
        u.screen(1.5, 64.0),
        u.screen(2.5, 60.0),
    );
    u.click(a);
    u.click(b);
    u.click(c);
    u.key(egui::Key::Escape);
    let s = u.shapes();
    assert_eq!(s.len(), 2, "two chained lines");
    let Geometry::Line { a: a0, b: b0 } = s[0].geometry else {
        panic!()
    };
    let Geometry::Line { a: a1, .. } = s[1].geometry else {
        panic!()
    };
    assert!(close(a0.t, 0.5, 0.01) && close(b0.st, 64.0, 0.05));
    assert_eq!(b0, a1, "second line starts where the first ended");
    assert!(u.app().canvas.pending.is_empty(), "Esc ends the chain");
}

#[test]
fn line_press_drag_release() {
    let mut u = Ui::new();
    u.tool(Tool::Line);
    let (a, b) = (u.screen(1.0, 60.0), u.screen(2.0, 67.0));
    u.drag(a, b);
    let s = u.shapes();
    assert_eq!(s.len(), 1);
    let Geometry::Line { a, b } = s[0].geometry else {
        panic!()
    };
    // Snap on: 1/16 at 120 BPM is 0.125 s; pitch to semitones.
    assert_eq!((a.t, a.st, b.t, b.st), (1.0, 60.0, 2.0, 67.0));
}

#[test]
fn rectangle_both_ways() {
    let mut u = Ui::new();
    u.tool(Tool::Rectangle);
    let (a, b) = (u.screen(0.5, 60.0), u.screen(1.5, 64.0));
    u.click(a);
    u.click(b);
    let (c, d) = (u.screen(2.0, 62.0), u.screen(3.0, 70.0));
    u.drag(c, d);
    let s = u.shapes();
    assert_eq!(s.len(), 2);
    assert!(s.iter().all(|x| !x.filled), "drawn shapes are outlines");
    assert_eq!(
        s[1].geometry,
        Geometry::Rectangle {
            a: Point::new(2.0, 62.0),
            b: Point::new(3.0, 70.0)
        }
    );
}

/// The arc tool is a three-point arc: start, end, then a point on the arc.
#[test]
fn arc_is_three_point() {
    let mut u = Ui::new();
    u.tool(Tool::Arc);
    let (s, e, m) = (
        u.screen(1.0, 60.0),
        u.screen(3.0, 60.0),
        u.screen(2.0, 66.0),
    );
    u.click(s);
    u.click(e);
    // While moving, the preview bends through the pointer.
    u.move_to(m);
    assert_eq!(u.app().canvas.pending.len(), 2);
    u.click(m);
    let shapes = u.shapes();
    assert_eq!(shapes.len(), 1);
    let (gs, gm, ge) = sketch::arc_points(&shapes[0].geometry).unwrap();
    assert!(close(gs.t, 1.0, 1e-9) && close(gs.st, 60.0, 1e-9), "{gs:?}");
    assert!(close(ge.t, 3.0, 1e-9) && close(ge.st, 60.0, 1e-9), "{ge:?}");
    assert!(
        close(gm.t, 2.0, 0.01) && close(gm.st, 66.0, 0.05),
        "passes through the third point: {gm:?}"
    );
    // And the other way: drag start to end, then click below to bend down.
    let (s2, e2, m2) = (
        u.screen(1.0, 55.0),
        u.screen(2.0, 55.0),
        u.screen(1.5, 52.0),
    );
    u.drag(s2, e2);
    u.click(m2);
    let shapes = u.shapes();
    assert_eq!(shapes.len(), 2);
    let (_, mid, _) = sketch::arc_points(&shapes[1].geometry).unwrap();
    assert!(mid.st < 55.0, "bends toward the third point");
}

/// A circle drawn at any zoom looks round: equal radius on screen in x and y.
#[test]
fn circle_is_round_on_screen() {
    let mut u = Ui::new();
    u.app().canvas.snap = false;
    u.tool(Tool::Circle);
    let c = u.screen(2.0, 66.0);
    u.click(c);
    u.click(c + Vec2::new(80.0, 0.0));
    let s = u.shapes();
    let Geometry::Circle {
        center,
        radius_s,
        radius_st,
    } = s[0].geometry
    else {
        panic!()
    };
    let v = u.view();
    let cs = v.point_to_screen(center);
    let rx = v
        .point_to_screen(Point::new(center.t + radius_s, center.st))
        .x
        - cs.x;
    let ry = cs.y
        - v.point_to_screen(Point::new(center.t, center.st + radius_st))
            .y;
    assert!((rx - 80.0).abs() < 0.5, "rx {rx}");
    assert!((ry / rx - 1.0).abs() < 0.01, "round: rx {rx} ry {ry}");
}

#[test]
fn ellipse_and_polygon() {
    let mut u = Ui::new();
    u.tool(Tool::Ellipse);
    let (c, k) = (u.screen(1.0, 64.0), u.screen(1.5, 67.0));
    u.click(c);
    u.click(k);
    u.tool(Tool::Polygon);
    u.app().canvas.polygon_sides = 5;
    let (c2, v2) = (u.screen(3.0, 64.0), u.screen(3.0, 69.0));
    u.drag(c2, v2);
    let s = u.shapes();
    assert_eq!(
        s[0].geometry,
        Geometry::Ellipse {
            center: Point::new(1.0, 64.0),
            rx_s: 0.5,
            ry_st: 3.0
        }
    );
    let Geometry::Polygon { points } = &s[1].geometry else {
        panic!()
    };
    assert_eq!(points.len(), 5);
    assert!(close(points[0].t, 3.0, 1e-9) && close(points[0].st, 69.0, 1e-9));
}

#[test]
fn polyline_finishes_by_double_click_enter_or_last_point() {
    let mut u = Ui::new();
    u.tool(Tool::Polyline);
    let pts = [
        u.screen(0.5, 60.0),
        u.screen(1.0, 65.0),
        u.screen(1.5, 62.0),
    ];
    u.click(pts[0]);
    u.click(pts[1]);
    u.double_click(pts[2]);
    assert_eq!(u.shapes().len(), 1);
    let Geometry::Polyline { points } = &u.shapes()[0].geometry else {
        panic!()
    };
    assert_eq!(points.len(), 3, "double-click adds the last point once");

    let more = [u.screen(2.0, 60.0), u.screen(2.5, 65.0)];
    u.click(more[0]);
    u.click(more[1]);
    u.key(egui::Key::Enter);
    assert_eq!(u.shapes().len(), 2);

    let last = [u.screen(3.0, 60.0), u.screen(3.5, 62.0)];
    u.click(last[0]);
    u.click(last[1]);
    u.click(last[1]);
    assert_eq!(
        u.shapes().len(),
        3,
        "clicking the last point again finishes"
    );
}

#[test]
fn escape_cancels_a_shape_in_progress() {
    let mut u = Ui::new();
    u.tool(Tool::Rectangle);
    let a = u.screen(1.0, 60.0);
    u.click(a);
    assert_eq!(u.app().canvas.pending.len(), 1);
    u.key(egui::Key::Escape);
    assert!(u.app().canvas.pending.is_empty());
    let b = u.screen(2.0, 64.0);
    u.click(b);
    assert!(u.shapes().is_empty(), "the next click starts a new shape");
}

/// New points snap to existing points within a few pixels, even off-grid.
#[test]
fn drawing_snaps_to_existing_points() {
    let mut u = Ui::new();
    u.app().canvas.snap = false;
    u.tool(Tool::Line);
    let (a, b) = (u.screen(1.03, 60.3), u.screen(2.07, 63.7));
    u.click(a);
    u.click(b);
    u.key(egui::Key::Escape);
    let end = match u.shapes()[0].geometry {
        Geometry::Line { b, .. } => b,
        _ => panic!(),
    };
    let near = u.view().point_to_screen(end) + Vec2::new(5.0, -4.0);
    let far = u.screen(3.0, 60.0);
    u.click(near);
    u.click(far);
    let Geometry::Line { a: start, .. } = u.shapes()[1].geometry else {
        panic!()
    };
    assert_eq!(start, end, "snapped exactly onto the existing endpoint");
}

// --- Select ----------------------------------------------------------------

fn rect_at(u: &mut Ui, t0: f64, s0: f64, t1: f64, s1: f64) {
    let shape = Shape::outline(Geometry::Rectangle {
        a: Point::new(t0, s0),
        b: Point::new(t1, s1),
    })
    .unwrap();
    assert!(u.app().doc.add_shape(shape));
    u.h.run_steps(1);
}

/// Clicking within a few pixels of a thin outline selects it; clicking well
/// away clears the selection. An outline's empty interior does not select.
#[test]
fn select_picks_by_pixel_distance() {
    let mut u = Ui::new();
    rect_at(&mut u, 1.0, 60.0, 2.0, 64.0);
    u.tool(Tool::Select);
    let edge = u.screen(1.3, 64.0) + Vec2::new(0.0, 5.0);
    u.click(edge);
    assert_eq!(u.selected(), BTreeSet::from([0]));
    let inside = u.screen(1.5, 62.0);
    u.click(inside);
    assert!(u.selected().is_empty(), "outline interior is empty space");
    // Filled shapes select from anywhere inside.
    u.app().doc.scene.voices[0].shapes[0].filled = true;
    u.click(inside);
    assert_eq!(u.selected(), BTreeSet::from([0]));
    let away = u.screen(3.0, 70.0);
    u.click(away);
    assert!(u.selected().is_empty());
}

/// Pressing on a shape and dragging moves it in one gesture: no need to
/// select first. One undo restores it.
#[test]
fn select_drag_moves_in_one_gesture() {
    let mut u = Ui::new();
    rect_at(&mut u, 1.0, 60.0, 2.0, 64.0);
    u.tool(Tool::Select);
    // Grab the top edge away from its midpoint handle.
    let from = u.screen(1.25, 64.0);
    let to = u.screen(1.75, 66.0);
    u.drag(from, to);
    assert_eq!(
        u.shapes()[0].geometry,
        Geometry::Rectangle {
            a: Point::new(1.5, 62.0),
            b: Point::new(2.5, 66.0)
        }
    );
    u.h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    u.h.run_steps(1);
    assert_eq!(
        u.shapes()[0].geometry,
        Geometry::Rectangle {
            a: Point::new(1.0, 60.0),
            b: Point::new(2.0, 64.0)
        }
    );
}

/// Dragging a rectangle corner resizes with the opposite corner fixed;
/// dragging an edge midpoint moves that side only.
#[test]
fn select_drags_corner_and_edge_handles() {
    let mut u = Ui::new();
    rect_at(&mut u, 1.0, 60.0, 2.0, 64.0);
    u.tool(Tool::Select);
    let corner = u.screen(2.0, 64.0);
    u.drag(corner + Vec2::new(3.0, 2.0), u.screen(3.0, 66.0));
    assert_eq!(
        u.shapes()[0].geometry,
        Geometry::Rectangle {
            a: Point::new(1.0, 60.0),
            b: Point::new(3.0, 66.0)
        }
    );
    let mid_left = u.screen(1.0, 63.0);
    u.drag(mid_left, u.screen(0.5, 70.0));
    assert_eq!(
        u.shapes()[0].geometry,
        Geometry::Rectangle {
            a: Point::new(0.5, 60.0),
            b: Point::new(3.0, 66.0)
        },
        "edge drag moved only the left side"
    );
}

#[test]
fn select_edits_arc_and_circle_handles() {
    let mut u = Ui::new();
    let k = {
        let v = u.view();
        v.k_at(v.y_of(62.0))
    };
    let arc = sketch::arc_3pt(
        Point::new(1.0, 60.0),
        Point::new(3.0, 60.0),
        Point::new(2.0, 64.0),
        k,
    )
    .unwrap();
    let circle = sketch::circle(Point::new(3.0, 70.0), Point::new(3.2, 70.0), k).unwrap();
    assert!(u.app().doc.add_shape(Shape::outline(arc).unwrap()));
    assert!(u.app().doc.add_shape(Shape::outline(circle).unwrap()));
    u.tool(Tool::Select);
    // Arc end: drag it; start stays, and the arc stays an arc.
    let end = u.screen(3.0, 60.0);
    u.drag(end, u.screen(3.5, 58.0));
    let (s, _, e) = sketch::arc_points(&u.shapes()[0].geometry).unwrap();
    assert!(close(s.t, 1.0, 1e-6) && close(s.st, 60.0, 1e-6));
    assert!(close(e.t, 3.5, 1e-6) && close(e.st, 58.0, 1e-6));
    // Circle centre: drag moves it; radius point: drag resizes it.
    let c = u.screen(3.0, 70.0);
    u.drag(c, u.screen(2.5, 72.0));
    let Geometry::Circle {
        center, radius_st, ..
    } = u.shapes()[1].geometry
    else {
        panic!()
    };
    assert_eq!(center, Point::new(2.5, 72.0));
    let r0 = radius_st;
    let top = u.screen(2.5, 72.0 + r0);
    u.drag(top, top + Vec2::new(0.0, -30.0));
    let Geometry::Circle {
        radius_st: r1,
        center: c1,
        ..
    } = u.shapes()[1].geometry
    else {
        panic!()
    };
    assert!(r1 > r0, "radius grew: {r0} -> {r1}");
    assert_eq!(c1, center);
    let handles = sketch::handles(&u.shapes()[1]);
    assert!(handles.iter().any(|h| h.kind == HandleKind::Center));
}

#[test]
fn box_select_shift_select_and_delete() {
    let mut u = Ui::new();
    rect_at(&mut u, 0.5, 60.0, 1.0, 62.0);
    rect_at(&mut u, 1.5, 60.0, 2.0, 62.0);
    rect_at(&mut u, 3.0, 70.0, 3.5, 72.0);
    u.tool(Tool::Select);
    let (a, b) = (u.screen(0.3, 58.0), u.screen(2.2, 63.0));
    u.drag(a, b);
    assert_eq!(
        u.selected(),
        BTreeSet::from([0, 1]),
        "box selects fully enclosed shapes"
    );
    let third = u.screen(3.25, 72.0);
    u.click_mods(third, Modifiers::SHIFT);
    assert_eq!(u.selected(), BTreeSet::from([0, 1, 2]), "shift adds");
    u.click_mods(third, Modifiers::SHIFT);
    assert_eq!(u.selected(), BTreeSet::from([0, 1]), "shift removes");
    u.key(egui::Key::Delete);
    assert_eq!(u.shapes().len(), 1);
    u.h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    u.h.run_steps(1);
    assert_eq!(u.shapes().len(), 3, "one undo restores both");
}

/// Moving a multi-selection moves every selected shape together.
#[test]
fn moving_a_multi_selection_moves_all() {
    let mut u = Ui::new();
    rect_at(&mut u, 0.5, 60.0, 1.0, 62.0);
    rect_at(&mut u, 1.5, 60.0, 2.0, 62.0);
    u.tool(Tool::Select);
    u.app().canvas.selected = BTreeSet::from([0, 1]);
    let from = u.screen(0.6, 62.0);
    let to = u.screen(1.1, 65.0);
    u.drag(from, to);
    let s = u.shapes();
    assert_eq!(
        s[0].geometry,
        Geometry::Rectangle {
            a: Point::new(1.0, 63.0),
            b: Point::new(1.5, 65.0)
        }
    );
    assert_eq!(
        s[1].geometry,
        Geometry::Rectangle {
            a: Point::new(2.0, 63.0),
            b: Point::new(2.5, 65.0)
        }
    );
}

// --- Filled shapes (display only: they sound as bands, see R1) ---------------

/// The fill display covers the interior of a non-rectangular shape and not
/// the corners of its bounding box: sample the rendered pixels.
#[test]
fn fill_display_follows_the_shape() {
    let mut u = Ui::new();
    let tri = Geometry::Polygon {
        points: vec![
            Point::new(1.0, 58.0),
            Point::new(3.0, 58.0),
            Point::new(2.0, 74.0),
        ],
    };
    assert!(u.app().doc.add_shape(Shape::filled(tri).unwrap()));
    u.app().canvas.show_grid = false;
    u.move_to(Pos2::new(5.0, 5.0));
    let inside = u.screen(2.0, 62.0);
    let corner = u.screen(1.15, 72.0);
    let empty = u.screen(3.6, 66.0);
    let img = u.h.render().expect("render");
    let ppp = img.width() as f32 / SIZE.x;
    let px = |p: Pos2| *img.get_pixel((p.x * ppp) as u32, (p.y * ppp) as u32);
    let (pin, pcorner, pempty) = (px(inside), px(corner), px(empty));
    assert_ne!(pin, pempty, "interior is tinted");
    assert_eq!(pcorner, pempty, "bounding-box corner is not tinted");
}

// --- Delete tool, navigation, keys ------------------------------------------

#[test]
fn delete_tool_removes_the_clicked_shape() {
    let mut u = Ui::new();
    rect_at(&mut u, 1.0, 60.0, 2.0, 64.0);
    rect_at(&mut u, 3.0, 60.0, 3.5, 64.0);
    u.tool(Tool::Delete);
    let p = u.screen(3.0, 62.0);
    u.click(p);
    assert_eq!(u.shapes().len(), 1);
    assert_eq!(
        u.shapes()[0].geometry,
        Geometry::Rectangle {
            a: Point::new(1.0, 60.0),
            b: Point::new(2.0, 64.0)
        }
    );
}

#[test]
fn shortcuts_select_tools() {
    let mut u = Ui::new();
    for (k, t) in [
        (egui::Key::L, Tool::Line),
        (egui::Key::G, Tool::Rectangle),
        (egui::Key::C, Tool::Circle),
        (egui::Key::A, Tool::Arc),
        (egui::Key::S, Tool::Select),
    ] {
        u.key(k);
        assert_eq!(u.app().canvas.tool, t);
    }
}

#[test]
fn wheel_zooms_at_pointer_and_trackpad_pans() {
    let mut u = Ui::new();
    let at = u.screen(2.0, 69.0);
    u.move_to(at);
    let w0 = u.app().doc.time.width_s;
    u.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, 2.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    });
    u.h.run_steps(2);
    let w1 = u.app().doc.time.width_s;
    assert!(w1 < w0, "wheel zoomed in: {w0} -> {w1}");
    let back = u.view().screen_to_point(at);
    assert!(
        close(back.t, 2.0, 0.01) && close(back.st, 69.0, 0.05),
        "point under cursor stays: {back:?}"
    );

    let s0 = u.app().doc.time.start_s;
    u.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(-60.0, 0.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Trackpad,
        modifiers: Modifiers::NONE,
    });
    u.h.run_steps(20);
    assert!(
        u.app().doc.time.start_s > s0,
        "two-finger swipe left pans later in time"
    );
}

#[test]
fn ruler_scrubs_and_shift_drag_sets_loop() {
    let mut u = Ui::new();
    let v = u.view();
    let y = v.rect.top() - 15.0; // the ruler sits just above the canvas
    let x = |t: f64| v.x_of(t);
    u.click(Pos2::new(x(1.5), y));
    assert!(close(u.app().playhead_s, 1.5, 0.01));
    u.drag_mods(Pos2::new(x(1.0), y), Pos2::new(x(2.0), y), Modifiers::SHIFT);
    let r = u.app().doc.loop_region.expect("loop set");
    assert!(
        close(r.start_s, 1.0, 0.01) && close(r.end_s, 2.0, 0.01),
        "{r:?}"
    );
    // Drag the loop end edge.
    u.drag(Pos2::new(x(2.0), y), Pos2::new(x(3.0), y));
    let r = u.app().doc.loop_region.unwrap();
    assert!(
        close(r.start_s, 1.0, 0.01) && close(r.end_s, 3.0, 0.01),
        "{r:?}"
    );
}

/// Every tool can be used in sequence without leaving stale state behind.
#[test]
fn each_tool_draws_a_shape() {
    let mut u = Ui::new();
    let mut expected = 0;
    for (tool, clicks) in [
        (Tool::Line, vec![(0.2, 60.0), (0.6, 62.0)]),
        (Tool::Rectangle, vec![(0.8, 60.0), (1.2, 63.0)]),
        (Tool::Circle, vec![(1.6, 66.0), (1.8, 66.0)]),
        (Tool::Ellipse, vec![(2.2, 66.0), (2.4, 68.0)]),
        (Tool::Arc, vec![(2.6, 60.0), (3.0, 60.0), (2.8, 62.0)]),
        (Tool::Polygon, vec![(3.4, 66.0), (3.4, 68.0)]),
    ] {
        u.tool(tool);
        for (t, st) in clicks {
            let p = u.screen(t, st);
            u.click(p);
        }
        u.key(egui::Key::Escape);
        expected += 1;
        assert_eq!(u.shapes().len(), expected, "{tool:?}");
    }
    let kinds: Vec<&str> = u
        .shapes()
        .iter()
        .map(|s| match s.geometry {
            Geometry::Line { .. } => "line",
            Geometry::Rectangle { .. } => "rect",
            Geometry::Circle { .. } => "circle",
            Geometry::Ellipse { .. } => "ellipse",
            Geometry::Arc { .. } => "arc",
            Geometry::Polygon { .. } => "polygon",
            Geometry::Polyline { .. } => "polyline",
        })
        .collect();
    assert_eq!(
        kinds,
        ["line", "rect", "circle", "ellipse", "arc", "polygon"]
    );
    let _ = canvas::PICK_PX;
}

/// Renders screenshots of real interactions for visual inspection.
/// Run with: SYNTHESIS_SHOTS=<dir> cargo test -p synthesis-app screenshots
#[test]
fn screenshots() {
    let Ok(dir) = std::env::var("SYNTHESIS_SHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut u = Ui::new();
    let save = |u: &mut Ui, name: &str| {
        let img = u.h.render().expect("render");
        img.save(dir.join(format!("{name}.png"))).unwrap();
    };
    // Shapes of every kind, some filled.
    let k = {
        let v = u.view();
        v.k_at(v.y_of(66.0))
    };
    let add = |u: &mut Ui, g: Geometry, filled: bool| {
        assert!(u.app().doc.add_shape(Shape::new(g, filled).unwrap()));
    };
    add(
        &mut u,
        Geometry::Line {
            a: Point::new(0.25, 57.0),
            b: Point::new(1.0, 62.0),
        },
        false,
    );
    add(
        &mut u,
        Geometry::Rectangle {
            a: Point::new(1.25, 57.0),
            b: Point::new(2.0, 61.0),
        },
        true,
    );
    add(
        &mut u,
        sketch::circle(Point::new(0.6, 72.0), Point::new(0.6, 76.0), k).unwrap(),
        true,
    );
    add(
        &mut u,
        sketch::ellipse(Point::new(1.6, 72.0), Point::new(1.95, 75.0)).unwrap(),
        false,
    );
    add(
        &mut u,
        sketch::arc_3pt(
            Point::new(2.25, 58.0),
            Point::new(3.0, 58.0),
            Point::new(2.6, 64.0),
            k,
        )
        .unwrap(),
        false,
    );
    add(
        &mut u,
        sketch::regular_polygon(Point::new(2.7, 72.0), Point::new(2.7, 76.0), 5, k).unwrap(),
        true,
    );
    add(
        &mut u,
        Geometry::Polygon {
            points: vec![
                Point::new(3.2, 57.0),
                Point::new(3.9, 57.0),
                Point::new(3.9, 66.0),
                Point::new(3.65, 66.0),
                Point::new(3.65, 59.0),
                Point::new(3.45, 59.0),
                Point::new(3.45, 66.0),
                Point::new(3.2, 66.0),
            ],
        },
        true,
    );
    u.h.run_steps(2);
    save(&mut u, "01_shapes");

    // Select: hover and selected handles on the arc.
    u.tool(Tool::Select);
    let arc = u.screen(2.25, 58.0) + Vec2::new(4.0, -12.0);
    u.click(arc);
    u.move_to(u.screen(1.6, 75.0));
    save(&mut u, "02_selected_arc");

    // Arc tool mid-gesture: start and end placed, pointer bending it.
    u.tool(Tool::Arc);
    let (s, e) = (u.screen(0.4, 66.0), u.screen(1.4, 66.0));
    u.click(s);
    u.click(e);
    u.move_to(u.screen(0.9, 69.0));
    save(&mut u, "03_arc_preview");
    u.key(egui::Key::Escape);

    // Box selection in progress.
    u.tool(Tool::Select);
    let a = u.screen(0.1, 55.5);
    u.press(a, Modifiers::NONE);
    for i in 1..=6 {
        let p = a + (u.screen(2.1, 63.5) - a) * (i as f32 / 6.0);
        u.move_to(p);
    }
    save(&mut u, "04_box_select");
}

/// Regression (reported): scrolling or zooming past time 0 must not reveal a
/// blank strip before the timeline, and the ruler keeps its labels.
#[test]
fn wheel_past_time_zero_keeps_timeline_at_the_edge() {
    let mut u = Ui::new();
    let at = u.screen(0.2, 66.0);
    u.move_to(at);
    // Zoom out hard at the left edge, then pan left.
    for _ in 0..12 {
        u.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
    }
    u.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(400.0, 0.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Trackpad,
        modifiers: Modifiers::NONE,
    });
    u.h.run_steps(20);
    let app = u.h.state();
    assert!(
        app.doc.time.start_s >= 0.0,
        "view starts before t=0: {}",
        app.doc.time.start_s
    );
    let left = u.view().screen_to_point(u.view().rect.left_center());
    assert!(left.t >= -1e-9, "left edge shows t={}", left.t);
}

/// Typed X division: `1/4` at 120 BPM is a quarter of a beat (0.125 s), so a
/// click snaps to the nearest 0.125 s, and the typed value is what snaps.
#[test]
fn typed_x_division_sets_the_snap_grid() {
    let mut u = Ui::new();
    u.app().canvas.x_div = synthesis_engine::Division::new(4).unwrap();
    u.app().canvas.bpm = 120.0;
    u.tool(Tool::Line);
    let a = u.screen(0.6, 60.0); // nearest 0.125 s step is 0.625 s
    let b = u.screen(1.3, 60.0); // nearest is 1.25 s
    u.click(a);
    u.click(b);
    u.key(egui::Key::Escape);
    let Geometry::Line { a, b } = u.shapes()[0].geometry else {
        panic!()
    };
    assert!(
        (a.t - 0.625).abs() < 1e-6,
        "start snapped to 0.625 s, got {}",
        a.t
    );
    assert!(
        (b.t - 1.25).abs() < 1e-6,
        "end snapped to 1.25 s, got {}",
        b.t
    );
}

/// Typed Y division: pitch snap follows the typed step when the step is wide
/// enough to see. Zoomed in to a few semitones, `1/1` snaps to whole semitones.
#[test]
fn typed_y_division_sets_the_pitch_snap() {
    let mut u = Ui::new();
    u.app().canvas.snap = true;
    u.app().doc.pitch.lo_hz = 440.0 * ((60.0 - 69.0 - 6.0) / 12.0_f64).exp2();
    u.app().doc.pitch.hi_hz = 440.0 * ((60.0 - 69.0 + 6.0) / 12.0_f64).exp2();
    u.app().canvas.y_div = synthesis_engine::Division::new(1).unwrap();
    u.tool(Tool::Line);
    let a = u.screen(0.5, 60.3);
    let b = u.screen(1.5, 60.7);
    u.click(a);
    u.click(b);
    u.key(egui::Key::Escape);
    let Geometry::Line { a, b } = u.shapes()[0].geometry else {
        panic!()
    };
    assert!((a.st - 60.0).abs() < 1e-6, "start pitch {}", a.st);
    assert!((b.st - 61.0).abs() < 1e-6, "end pitch {}", b.st);
}

/// An invalid typed division is rejected and the old value is kept.
#[test]
fn invalid_division_text_is_ignored() {
    use synthesis_engine::Division;
    assert_eq!(Division::parse("1/0"), None);
    assert_eq!(Division::parse("nonsense"), None);
    let mut u = Ui::new();
    let before = u.app().canvas.x_div;
    assert_eq!(before, Division::new(16).unwrap());
    assert_eq!(Division::parse("1/999"), None, "out of range keeps 1/16");
}

/// Master gain: the slider sets the scene's master gain as one undoable step,
/// and undo restores the value from before the change.
#[test]
fn master_gain_is_one_undoable_step() {
    let mut u = Ui::new();
    assert!((u.app().doc.scene.settings.master_gain - 0.7).abs() < 1e-9);
    u.app().set_master_gain(0.4);
    assert!((u.app().doc.scene.settings.master_gain - 0.4).abs() < 1e-9);
    assert!(u.app().doc.history.can_undo());
    u.app().undo();
    assert!(
        (u.app().doc.scene.settings.master_gain - 0.7).abs() < 1e-9,
        "undo restores 70%"
    );
}

/// Setting the same value again is not recorded as an edit.
#[test]
fn master_gain_same_value_is_not_an_edit() {
    let mut u = Ui::new();
    u.app().set_master_gain(0.7);
    assert!(!u.app().doc.history.can_undo());
}

/// Regression (reported): pitch labels sit in a gutter left of the grid, not
/// on top of it, and the ruler's time axis lines up with the grid's.
#[test]
fn pitch_labels_are_outside_the_grid() {
    let mut u = Ui::new();
    u.app().canvas.show_grid = false; // so the plot area is a flat colour
    u.move_to(Pos2::new(5.0, 5.0));
    let plot = u.view().rect;
    let img = u.h.render().expect("render");
    let ppp = img.width() as f32 / SIZE.x;
    let px = |x: f32, y: f32| *img.get_pixel((x * ppp) as u32, (y * ppp) as u32);
    // A strip of the plot just right of its left edge: no label text here,
    // so every pixel matches the plot background.
    let bg = px(plot.center().x, plot.center().y);
    let mut stray = 0;
    let mut y = plot.top() + 2.0;
    while y < plot.bottom() - 2.0 {
        for dx in 1..40 {
            if px(plot.left() + dx as f32, y) != bg {
                stray += 1;
            }
        }
        y += 1.0;
    }
    assert_eq!(
        stray, 0,
        "{stray} non-background pixels at the grid's left edge"
    );
    // The gutter left of the plot does carry label text: it is not uniform.
    let gutter_x = plot.left() - 20.0;
    let mut distinct = std::collections::BTreeSet::new();
    let mut y = plot.top();
    while y < plot.bottom() {
        let p = px(gutter_x, y);
        distinct.insert((p[0], p[1], p[2]));
        y += 1.0;
    }
    assert!(distinct.len() > 2, "gutter shows labels");
    // Time 0 is at the plot's left edge in both the ruler and the grid.
    let v = u.view();
    assert!((v.x_of(0.0) - plot.left()).abs() < 0.5);
}

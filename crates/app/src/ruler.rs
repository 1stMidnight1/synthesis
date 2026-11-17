//! Time ruler above the canvas (TRANS-01 scrub, TRANS-02 loop, TRANS-03).
//!
//! - Click or drag to move the playhead.
//! - Shift-drag sets a loop region. Drag a loop edge to adjust it, or drag the
//!   middle of the region to move it. Double-click the region to clear it.
//! - Labels show bars, with seconds underneath (D-15 primary display seconds;
//!   the ruler grid follows BPM, as the canvas grid does).

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use synthesis_engine::{LoopRegion, Tempo};

use crate::document::Document;

/// Height of the ruler strip in points.
pub const HEIGHT: f32 = 30.0;
/// Grab distance for loop edges, in points.
const EDGE_PX: f32 = 6.0;
const LOOP_COLOR: Color32 = Color32::from_rgb(110, 165, 255);

/// What a ruler drag is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RulerDrag {
    Scrub,
    /// Building a new loop from this anchor time.
    NewLoop(f64),
    LoopStart,
    LoopEnd,
    /// Moving the loop; `offset` is pointer time minus loop start.
    MoveLoop(f64),
}

fn x_of(t: f64, rect: Rect, doc: &Document) -> f32 {
    rect.left() + ((t - doc.time.start_s) / doc.time.width_s) as f32 * rect.width()
}

fn t_of(x: f32, rect: Rect, doc: &Document) -> f64 {
    doc.time.start_s + f64::from((x - rect.left()) / rect.width()) * doc.time.width_s
}

/// Draw the ruler and handle its input. Returns a seek request in seconds.
pub fn show(
    ui: &mut Ui,
    doc: &mut Document,
    playhead_s: f64,
    bpm: f64,
    drag: &mut Option<RulerDrag>,
) -> Option<f64> {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, Color32::from_gray(30));
    let dur = doc.scene.duration_s;
    let s_per_px = doc.time.width_s / f64::from(rect.width());

    // Beyond the timeline is shaded.
    let (x0, x1) = (x_of(0.0, rect, doc), x_of(dur, rect, doc));
    for (a, b) in [(rect.left(), x0), (x1, rect.right())] {
        if b > a {
            p.rect_filled(
                Rect::from_min_max(
                    Pos2::new(a.max(rect.left()), rect.top()),
                    Pos2::new(b.min(rect.right()), rect.bottom()),
                ),
                0.0,
                Color32::from_gray(18),
            );
        }
    }

    // Loop region.
    if let Some(r) = doc.loop_region {
        let (a, b) = (x_of(r.start_s, rect, doc), x_of(r.end_s, rect, doc));
        let band = Rect::from_min_max(Pos2::new(a, rect.top()), Pos2::new(b, rect.bottom()));
        p.rect_filled(
            band,
            0.0,
            Color32::from_rgba_unmultiplied(110, 165, 255, 55),
        );
        for x in [a, b] {
            p.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(2.0, LOOP_COLOR),
            );
        }
    }

    // Ticks: bars (labelled), beats; fall back to coarser steps when crowded.
    if let Some(tempo) = Tempo::new(bpm) {
        let beat = tempo.beat_s();
        let bar = beat * Tempo::BEATS_PER_BAR;
        let mut label_every = 1u32;
        while bar * f64::from(label_every) / s_per_px < 48.0 && label_every < 4096 {
            label_every *= 2;
        }
        let show_beats = beat / s_per_px >= 8.0;
        let first = (doc.time.start_s.max(0.0) / beat).floor() as i64;
        let last = ((doc.time.start_s + doc.time.width_s).min(dur) / beat).ceil() as i64;
        for i in first..=last {
            let t = i as f64 * beat;
            let x = x_of(t, rect, doc);
            let is_bar = i % 4 == 0;
            if is_bar {
                let n = (i / 4) as u32;
                let labelled = n.is_multiple_of(label_every);
                let h = if labelled { 12.0 } else { 6.0 };
                p.line_segment(
                    [Pos2::new(x, rect.bottom() - h), Pos2::new(x, rect.bottom())],
                    Stroke::new(1.0, Color32::from_gray(150)),
                );
                if labelled {
                    p.text(
                        Pos2::new(x + 3.0, rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", n + 1),
                        egui::FontId::proportional(11.0),
                        Color32::from_gray(200),
                    );
                    p.text(
                        Pos2::new(x + 3.0, rect.top() + 15.0),
                        egui::Align2::LEFT_TOP,
                        format!("{t:.2}s"),
                        egui::FontId::monospace(9.0),
                        Color32::from_gray(120),
                    );
                }
            } else if show_beats {
                p.line_segment(
                    [
                        Pos2::new(x, rect.bottom() - 4.0),
                        Pos2::new(x, rect.bottom()),
                    ],
                    Stroke::new(1.0, Color32::from_gray(95)),
                );
            }
        }
    }

    // Playhead.
    let px = x_of(playhead_s, rect, doc);
    if rect.x_range().contains(px) {
        p.line_segment(
            [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
            Stroke::new(2.0, Color32::WHITE),
        );
        let tri = vec![
            Pos2::new(px - 5.0, rect.top()),
            Pos2::new(px + 5.0, rect.top()),
            Pos2::new(px, rect.top() + 6.0),
        ];
        p.add(egui::Shape::convex_polygon(
            tri,
            Color32::WHITE,
            Stroke::NONE,
        ));
    }

    // Input.
    let shift = ui.input(|i| i.modifiers.shift);
    let near_edge = |x: f32| -> Option<RulerDrag> {
        let r = doc.loop_region?;
        if (x - x_of(r.start_s, rect, doc)).abs() <= EDGE_PX {
            Some(RulerDrag::LoopStart)
        } else if (x - x_of(r.end_s, rect, doc)).abs() <= EDGE_PX {
            Some(RulerDrag::LoopEnd)
        } else {
            None
        }
    };
    let in_loop = |t: f64| {
        doc.loop_region
            .is_some_and(|r| r.start_s < t && t < r.end_s)
    };
    if let Some(h) = response.hover_pos() {
        let t = t_of(h.x, rect, doc);
        let icon = if near_edge(h.x).is_some() {
            CursorIcon::ResizeHorizontal
        } else if !shift && in_loop(t) && ui.input(|i| i.modifiers.command) {
            CursorIcon::Grab
        } else {
            CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(icon);
    }

    let mut seek = None;
    let min_len = (s_per_px * 4.0).max(0.01);
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && in_loop(t_of(pos.x, rect, doc))
    {
        doc.clear_loop();
        return None;
    }
    if response.drag_started()
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
    {
        let t = t_of(press.x, rect, doc).clamp(0.0, dur);
        *drag = Some(if shift {
            RulerDrag::NewLoop(t)
        } else if let Some(edge) = near_edge(press.x) {
            edge
        } else if in_loop(t) && ui.input(|i| i.modifiers.command) {
            RulerDrag::MoveLoop(t - doc.loop_region.map_or(0.0, |r| r.start_s))
        } else {
            RulerDrag::Scrub
        });
    }
    if let (Some(d), Some(pos)) = (*drag, response.interact_pointer_pos())
        && (response.dragged() || response.drag_stopped())
    {
        let t = t_of(pos.x, rect, doc).clamp(0.0, dur);
        match d {
            RulerDrag::Scrub => seek = Some(t),
            RulerDrag::NewLoop(anchor) => {
                doc.set_loop(anchor, t, min_len);
            }
            RulerDrag::LoopStart => {
                if let Some(r) = doc.loop_region {
                    doc.set_loop(t.min(r.end_s - min_len), r.end_s, min_len);
                }
            }
            RulerDrag::LoopEnd => {
                if let Some(r) = doc.loop_region {
                    doc.set_loop(r.start_s, t.max(r.start_s + min_len), min_len);
                }
            }
            RulerDrag::MoveLoop(offset) => {
                if let Some(r) = doc.loop_region {
                    let len = r.len_s();
                    let start = (t - offset).clamp(0.0, (dur - len).max(0.0));
                    doc.loop_region = LoopRegion::new(start, start + len);
                }
            }
        }
    }
    if response.drag_stopped() {
        *drag = None;
    }
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        seek = Some(t_of(pos.x, rect, doc).clamp(0.0, dur));
    }
    seek
}

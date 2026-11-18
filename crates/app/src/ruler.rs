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
pub const HEIGHT: f32 = 34.0;
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
    // Same left gutter as the canvas, so ruler time lines up with the grid.
    let (full, _) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::hover());
    ui.painter_at(full)
        .rect_filled(full, 0.0, Color32::from_gray(30));
    let rect = Rect::from_min_max(
        Pos2::new(full.left() + crate::canvas::GUTTER_PX, full.top()),
        full.max,
    );
    let response = ui.interact(
        rect,
        ui.scope_id().with("synthesis_ruler"),
        Sense::click_and_drag(),
    );
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

    // Ticks and labels. Every beat gets a tick; a label (bar number, and
    // seconds) goes on each bar when there is room, and on each beat when
    // zoomed in far enough. Labels step to coarser bar multiples when crowded.
    if let Some(tempo) = Tempo::new(bpm) {
        let beat = tempo.beat_s();
        let bar = beat * Tempo::BEATS_PER_BAR;
        let beat_px = beat / s_per_px;
        let bar_px = bar / s_per_px;
        // Smallest bar multiple whose labels are at least 90 px apart.
        let mut bars_per_label = 1u32;
        while bar_px * f64::from(bars_per_label) < 90.0 && bars_per_label < 4096 {
            bars_per_label *= 2;
        }
        // Beat labels need room for "1.25 s": about 70 px. Below that only bars are labelled.
        let label_beats = beat_px >= 70.0 && bars_per_label == 1;
        // A bar label needs room for "1   2.00 s" at 13 px type (about 80 px).
        let bar_labels_ok = bar_px >= 80.0 || bars_per_label > 1;
        let first = (doc.time.start_s.max(0.0) / beat).floor() as i64;
        let last = ((doc.time.start_s + doc.time.width_s).min(dur) / beat).ceil() as i64;
        for i in first..=last {
            let t = i as f64 * beat;
            if t < 0.0 || t > dur + 1e-9 {
                continue;
            }
            let x = x_of(t, rect, doc);
            let is_bar = i.rem_euclid(4) == 0;
            let bar_no = i.div_euclid(4) as u32;
            let on_label = if is_bar {
                bar_labels_ok && bar_no.is_multiple_of(bars_per_label)
            } else {
                label_beats
            };
            let (h, color) = if is_bar {
                (16.0, Color32::from_gray(170))
            } else {
                (8.0, Color32::from_gray(100))
            };
            p.line_segment(
                [Pos2::new(x, rect.bottom() - h), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, color),
            );
            if on_label {
                let text = if is_bar {
                    format!("{}   {:.2} s", bar_no + 1, t)
                } else {
                    format!("{:.2} s", t)
                };
                let font = if is_bar {
                    egui::FontId::proportional(13.0)
                } else {
                    egui::FontId::monospace(11.0)
                };
                let pos = Pos2::new(x + 4.0, rect.top() + 3.0);
                let galley = p.layout_no_wrap(text, font, Color32::from_gray(225));
                let bg = Rect::from_min_size(
                    pos - Vec2::new(2.0, 1.0),
                    galley.size() + Vec2::new(4.0, 2.0),
                );
                p.rect_filled(bg, 2.0, Color32::from_black_alpha(140));
                p.galley(pos, galley, Color32::WHITE);
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

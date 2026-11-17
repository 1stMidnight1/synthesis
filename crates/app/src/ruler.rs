//! Time ruler above the canvas (TRANS-01 scrub, TRANS-02 loop region).
//!
//! Click or drag to move the playhead. Shift-drag sets the loop region.
//! The region is drawn as a shaded band with its edges marked.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::document::{Document, TimeView};

/// Height of the ruler strip in points.
pub const HEIGHT: f32 = 26.0;

/// Map a screen x to a time, given the ruler rect and visible time window.
pub fn x_to_time(x: f32, rect: Rect, time: TimeView) -> f64 {
    let fx = f64::from((x - rect.left()) / rect.width());
    time.start_s + fx * time.width_s
}

/// Map a time to a screen x.
pub fn time_to_x(t: f64, rect: Rect, time: TimeView) -> f32 {
    let fx = ((t - time.start_s) / time.width_s) as f32;
    rect.left() + fx * rect.width()
}

/// Draw the ruler and handle scrub and loop input. Returns a seek request
/// (in seconds) if the user scrubbed.
pub fn show(ui: &mut Ui, doc: &mut Document, playhead_s: f64) -> Option<f64> {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(30));

    // Loop region band.
    if let Some(r) = doc.loop_region {
        let a = time_to_x(r.start_s, rect, doc.time);
        let b = time_to_x(r.end_s, rect, doc.time);
        let band = Rect::from_min_max(Pos2::new(a, rect.top()), Pos2::new(b, rect.bottom()));
        painter.rect_filled(
            band,
            0.0,
            Color32::from_rgba_unmultiplied(120, 170, 255, 60),
        );
        painter.line_segment(
            [Pos2::new(a, rect.top()), Pos2::new(a, rect.bottom())],
            Stroke::new(2.0, Color32::from_rgb(120, 170, 255)),
        );
        painter.line_segment(
            [Pos2::new(b, rect.top()), Pos2::new(b, rect.bottom())],
            Stroke::new(2.0, Color32::from_rgb(120, 170, 255)),
        );
    }

    // Second tick labels every whole second.
    let mut t = doc.time.start_s.ceil();
    while t <= doc.time.start_s + doc.time.width_s {
        let x = time_to_x(t, rect, doc.time);
        painter.line_segment(
            [
                Pos2::new(x, rect.bottom() - 6.0),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0, Color32::from_gray(120)),
        );
        painter.text(
            Pos2::new(x + 2.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{t:.0}s"),
            egui::FontId::monospace(10.0),
            Color32::from_gray(150),
        );
        t += 1.0;
    }

    // Playhead marker.
    let px = time_to_x(playhead_s, rect, doc.time);
    painter.line_segment(
        [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
        Stroke::new(1.5, Color32::WHITE),
    );

    let shift = ui.input(|i| i.modifiers.shift);
    let mut seek = None;
    if let Some(pos) = response.interact_pointer_pos() {
        let t = x_to_time(pos.x, rect, doc.time).clamp(0.0, doc.scene.duration_s);
        if shift {
            // Shift-drag builds the loop: anchor at drag start, extend to pointer.
            if response.drag_started() {
                doc.loop_anchor = Some(t);
            }
            if let Some(anchor) = doc.loop_anchor
                && (response.dragged() || response.drag_stopped())
            {
                doc.set_loop(anchor, t, 0.05);
            }
            if response.drag_stopped() {
                doc.loop_anchor = None;
            }
        } else if response.dragged() || response.clicked() {
            seek = Some(t);
        }
    }
    seek
}

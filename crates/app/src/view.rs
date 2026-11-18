//! Screen <-> (time, pitch) mapping and navigation for the canvas and ruler
//! (CANVAS-01, R8).
//!
//! All hit-testing and snapping happen in screen pixels through this mapping,
//! so tolerances feel the same at every zoom level.

use eframe::egui::{Pos2, Rect, Vec2};
use synthesis_engine::Point;
use synthesis_engine::pitch::{hz_to_st, st_to_hz};
use synthesis_engine::pitchscale::PitchScale;

use crate::document::{PitchView, TimeView};

pub const MIN_WIDTH_S: f64 = 0.05;
pub const MAX_WIDTH_S: f64 = 120.0;
const MIN_HZ: f64 = 5.0;
const MAX_HZ: f64 = 24_000.0;
/// Narrowest pitch range: two semitones.
const MIN_LOG_SPAN: f64 = std::f64::consts::LN_2 / 6.0;
const MIN_LINEAR_SPAN_HZ: f64 = 10.0;

/// A canvas viewport: the screen rectangle and the time and pitch it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub rect: Rect,
    pub time: TimeView,
    pub pitch: PitchView,
    pub a4: f64,
    /// What the vertical axis means. The canvas has one set of tools and one
    /// grid; this decides whether Y is pitch (Tone) or level (Volume).
    pub mode: VerticalMode,
}

/// The two tabs of the one canvas. Time is on X in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalMode {
    /// Y is pitch (semitones, Hz). The default.
    #[default]
    Tone,
    /// Y is level in percent, 0 at the bottom to `LEVEL_MAX` at the top (R5).
    Volume,
}

/// Top of the volume axis, in percent (R5, D-10).
pub const LEVEL_MAX: f64 = 200.0;

impl View {
    pub fn x_of(&self, t: f64) -> f32 {
        let f = (t - self.time.start_s) / self.time.width_s;
        self.rect.left() + f as f32 * self.rect.width()
    }

    pub fn t_of(&self, x: f32) -> f64 {
        let f = f64::from((x - self.rect.left()) / self.rect.width());
        self.time.start_s + f * self.time.width_s
    }

    fn frac_of_hz(&self, hz: f64) -> f64 {
        let (lo, hi) = (self.pitch.lo_hz, self.pitch.hi_hz);
        match self.pitch.scale {
            PitchScale::LinearHz => (hz - lo) / (hi - lo),
            PitchScale::Semitones | PitchScale::LogHz => {
                (hz.max(1e-9).ln() - lo.ln()) / (hi.ln() - lo.ln())
            }
        }
    }

    fn hz_of_frac(&self, f: f64) -> f64 {
        let (lo, hi) = (self.pitch.lo_hz, self.pitch.hi_hz);
        match self.pitch.scale {
            PitchScale::LinearHz => lo + f * (hi - lo),
            PitchScale::Semitones | PitchScale::LogHz => (lo.ln() + f * (hi.ln() - lo.ln())).exp(),
        }
    }

    pub fn y_of_hz(&self, hz: f64) -> f32 {
        self.rect.bottom() - self.frac_of_hz(hz) as f32 * self.rect.height()
    }

    pub fn hz_of_y(&self, y: f32) -> f64 {
        self.hz_of_frac(f64::from((self.rect.bottom() - y) / self.rect.height()))
    }

    /// Screen Y of a vertical value: a pitch in semitones (Tone) or a level
    /// in percent (Volume).
    pub fn y_of(&self, v: f64) -> f32 {
        match self.mode {
            VerticalMode::Tone => self.y_of_hz(st_to_hz(v, self.a4)),
            VerticalMode::Volume => {
                self.rect.bottom() - (v / LEVEL_MAX) as f32 * self.rect.height()
            }
        }
    }

    /// The vertical value at screen Y: a pitch in semitones (Tone) or a level
    /// in percent (Volume), clamped to the axis in Volume.
    pub fn st_of(&self, y: f32) -> f64 {
        match self.mode {
            VerticalMode::Tone => hz_to_st(self.hz_of_y(y).max(1e-6), self.a4),
            VerticalMode::Volume => {
                let f = f64::from((self.rect.bottom() - y) / self.rect.height());
                (f * LEVEL_MAX).clamp(0.0, LEVEL_MAX)
            }
        }
    }

    pub fn point_to_screen(&self, p: Point) -> Pos2 {
        Pos2::new(self.x_of(p.t), self.y_of(p.st))
    }

    pub fn screen_to_point(&self, pos: Pos2) -> Point {
        Point::new(self.t_of(pos.x), self.st_of(pos.y))
    }

    /// Seconds per screen point.
    pub fn s_per_px(&self) -> f64 {
        self.time.width_s / f64::from(self.rect.width())
    }

    /// Semitones per screen point at height `y` (constant in the log modes).
    pub fn st_per_px(&self, y: f32) -> f64 {
        (self.st_of(y - 0.5) - self.st_of(y + 0.5)).abs().max(1e-9)
    }

    /// Seconds that look as long on screen as one semitone, near height `y`.
    /// Sketch tools use it so that circles and regular polygons look round.
    pub fn k_at(&self, y: f32) -> f64 {
        self.s_per_px() / self.st_per_px(y)
    }
}

/// Timeline start (t = 0) is the left edge of the view, always. The view never
/// shows time before the timeline: that area would be blank, with no labels.
/// The view may be narrower than the whole timeline, and may end past it.
fn clamp_time(time: &mut TimeView) {
    time.width_s = time.width_s.clamp(MIN_WIDTH_S, MAX_WIDTH_S);
    time.start_s = time.start_s.max(0.0);
}

fn clamp_pitch(pitch: &mut PitchView) {
    match pitch.scale {
        // `max` before `min` (not `clamp`): rounding can make the upper bound
        // a hair below the lower one at full zoom-out, and clamp panics then.
        PitchScale::LinearHz => {
            let span = (pitch.hi_hz - pitch.lo_hz).clamp(MIN_LINEAR_SPAN_HZ, MAX_HZ - MIN_HZ);
            let lo = pitch.lo_hz.min(MAX_HZ - span).max(MIN_HZ);
            pitch.lo_hz = lo;
            pitch.hi_hz = lo + span;
        }
        PitchScale::Semitones | PitchScale::LogHz => {
            let span = (pitch.hi_hz / pitch.lo_hz)
                .ln()
                .clamp(MIN_LOG_SPAN, (MAX_HZ / MIN_HZ).ln());
            let lo = pitch.lo_hz.ln().min(MAX_HZ.ln() - span).max(MIN_HZ.ln());
            pitch.lo_hz = lo.exp();
            pitch.hi_hz = (lo + span).exp();
        }
    }
}

/// Pan so the content moves by `delta` screen points (it follows the pointer).
pub fn pan(time: &mut TimeView, pitch: &mut PitchView, rect: Rect, delta: Vec2) {
    time.start_s -= f64::from(delta.x / rect.width()) * time.width_s;
    let fy = f64::from(delta.y / rect.height());
    match pitch.scale {
        PitchScale::LinearHz => {
            let shift = fy * (pitch.hi_hz - pitch.lo_hz);
            pitch.lo_hz += shift;
            pitch.hi_hz += shift;
        }
        PitchScale::Semitones | PitchScale::LogHz => {
            let f = (fy * (pitch.hi_hz / pitch.lo_hz).ln()).exp();
            pitch.lo_hz *= f;
            pitch.hi_hz *= f;
        }
    }
    clamp_time(time);
    clamp_pitch(pitch);
}

/// Zoom about the screen point `at`, which stays over the same time and pitch.
/// Factors above 1 zoom in. `ft` applies to time, `fp` to pitch.
pub fn zoom_at(time: &mut TimeView, pitch: &mut PitchView, rect: Rect, at: Pos2, ft: f64, fp: f64) {
    if ft.is_finite() && ft > 0.0 {
        let t0 = time.start_s + f64::from((at.x - rect.left()) / rect.width()) * time.width_s;
        let w = (time.width_s / ft).clamp(MIN_WIDTH_S, MAX_WIDTH_S);
        time.start_s = t0 - (t0 - time.start_s) * (w / time.width_s);
        time.width_s = w;
    }
    if fp.is_finite() && fp > 0.0 {
        let f = f64::from((rect.bottom() - at.y) / rect.height());
        match pitch.scale {
            PitchScale::LinearHz => {
                let span = pitch.hi_hz - pitch.lo_hz;
                let h0 = pitch.lo_hz + f * span;
                let new = (span / fp).clamp(MIN_LINEAR_SPAN_HZ, MAX_HZ - MIN_HZ);
                pitch.lo_hz = h0 - f * new;
                pitch.hi_hz = pitch.lo_hz + new;
            }
            PitchScale::Semitones | PitchScale::LogHz => {
                let (lo, hi) = (pitch.lo_hz.ln(), pitch.hi_hz.ln());
                let u0 = lo + f * (hi - lo);
                let new = ((hi - lo) / fp).clamp(MIN_LOG_SPAN, (MAX_HZ / MIN_HZ).ln());
                pitch.lo_hz = (u0 - f * new).exp();
                pitch.hi_hz = (u0 + (1.0 - f) * new).exp();
            }
        }
    }
    clamp_time(time);
    clamp_pitch(pitch);
}

/// Fit the view to a time range and pitch range (semitones), with a margin.
pub fn fit(time: &mut TimeView, pitch: &mut PitchView, t: (f64, f64), st: (f64, f64), a4: f64) {
    let (t0, t1) = (t.0.min(t.1).max(0.0), t.0.max(t.1).max(0.0));
    let w = (t1 - t0).max(0.5);
    time.width_s = w * 1.15;
    time.start_s = (t0 - w * 0.075).max(0.0);
    let (s0, s1) = (st.0.min(st.1), st.0.max(st.1));
    let pad = ((s1 - s0) * 0.15).max(2.0);
    pitch.lo_hz = st_to_hz(s0 - pad, a4);
    pitch.hi_hz = st_to_hz(s1 + pad, a4);
    clamp_time(time);
    clamp_pitch(pitch);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(scale: PitchScale) -> View {
        View {
            rect: Rect::from_min_size(Pos2::new(40.0, 20.0), Vec2::new(800.0, 400.0)),
            time: TimeView {
                start_s: 1.0,
                width_s: 4.0,
            },
            pitch: PitchView {
                lo_hz: 100.0,
                hi_hz: 1600.0,
                scale,
            },
            a4: 440.0,
            mode: VerticalMode::default(),
        }
    }

    const SCALES: [PitchScale; 3] = [
        PitchScale::Semitones,
        PitchScale::LogHz,
        PitchScale::LinearHz,
    ];

    #[test]
    fn mapping_round_trips_and_is_oriented() {
        for scale in SCALES {
            let v = view(scale);
            let p = Point::new(2.25, 64.0);
            let back = v.screen_to_point(v.point_to_screen(p));
            assert!((back.t - p.t).abs() < 1e-4 && (back.st - p.st).abs() < 1e-3);
            let lo = v.point_to_screen(Point::new(1.5, 50.0));
            let hi = v.point_to_screen(Point::new(3.0, 70.0));
            assert!(hi.x > lo.x && hi.y < lo.y, "{scale:?}");
        }
    }

    #[test]
    fn k_makes_one_unit_equally_long_on_both_axes() {
        for scale in SCALES {
            let v = view(scale);
            let c = Point::new(2.0, 69.0);
            let y = v.y_of(c.st);
            let k = v.k_at(y);
            let r = 1.0; // one semitone up, and k seconds right
            let up = v.point_to_screen(Point::new(c.t, c.st + r * 0.01));
            let right = v.point_to_screen(Point::new(c.t + k * r * 0.01, c.st));
            let cs = v.point_to_screen(c);
            let (dy, dx) = ((cs.y - up.y).abs(), (right.x - cs.x).abs());
            assert!((dx / dy - 1.0).abs() < 0.01, "{scale:?}: {dx} vs {dy}");
        }
    }

    #[test]
    fn pan_moves_content_with_the_pointer() {
        for scale in SCALES {
            let mut v = view(scale);
            let p = Point::new(2.0, 67.0);
            let before = v.point_to_screen(p);
            let d = Vec2::new(37.0, -21.0);
            pan(&mut v.time, &mut v.pitch, v.rect, d);
            let after = v.point_to_screen(p);
            assert!(
                (after - before - d).length() < 0.05,
                "{scale:?}: {before:?} -> {after:?}"
            );
        }
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        for scale in SCALES {
            let mut v = view(scale);
            let at = Pos2::new(300.0, 150.0);
            let p = v.screen_to_point(at);
            zoom_at(&mut v.time, &mut v.pitch, v.rect, at, 1.7, 1.3);
            let after = v.point_to_screen(p);
            assert!((after - at).length() < 0.05, "{scale:?}: {after:?}");
            assert!((v.time.width_s - 4.0 / 1.7).abs() < 1e-9);
        }
    }

    /// Regression: the view never shows negative time, however it is zoomed or
    /// panned, so the area before t = 0 can't appear as a blank bar.
    #[test]
    fn view_never_shows_time_before_zero() {
        let mut v = view(PitchScale::Semitones);
        v.time.start_s = 0.5;
        pan(&mut v.time, &mut v.pitch, v.rect, Vec2::new(-5000.0, 0.0));
        assert!(
            v.time.start_s >= 0.0,
            "panned back before 0: {}",
            v.time.start_s
        );
        // Zoom out at the left edge: the anchor stays at t >= 0.
        zoom_at(
            &mut v.time,
            &mut v.pitch,
            v.rect,
            v.rect.left_center(),
            0.2,
            1.0,
        );
        assert!(v.time.start_s >= 0.0, "zoomed past 0: {}", v.time.start_s);
        let t_left = v.screen_to_point(v.rect.left_center()).t;
        assert!(t_left >= -1e-9, "left edge shows {t_left}");
    }

    #[test]
    fn zoom_and_pan_are_clamped() {
        let mut v = view(PitchScale::Semitones);
        zoom_at(&mut v.time, &mut v.pitch, v.rect, v.rect.center(), 1e6, 1e6);
        assert!(v.time.width_s >= MIN_WIDTH_S);
        assert!((v.pitch.hi_hz / v.pitch.lo_hz).ln() >= MIN_LOG_SPAN - 1e-12);
        zoom_at(
            &mut v.time,
            &mut v.pitch,
            v.rect,
            v.rect.center(),
            1e-6,
            1e-6,
        );
        assert!(v.time.width_s <= MAX_WIDTH_S);
        assert!(v.pitch.lo_hz >= MIN_HZ - 1e-9 && v.pitch.hi_hz <= MAX_HZ + 1e-6);
        pan(&mut v.time, &mut v.pitch, v.rect, Vec2::new(1e6, 0.0));
        assert!(v.time.start_s >= 0.0);
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    fn view(mode: VerticalMode) -> View {
        View {
            rect: Rect::from_min_size(Pos2::new(40.0, 20.0), Vec2::new(800.0, 400.0)),
            time: TimeView {
                start_s: 0.0,
                width_s: 4.0,
            },
            pitch: PitchView::default(),
            a4: 440.0,
            mode,
        }
    }

    /// Tone is unchanged: a point's Y is its pitch, and round-trips.
    #[test]
    fn tone_round_trips_as_before() {
        let v = view(VerticalMode::Tone);
        let p = Point::new(1.5, 64.0);
        let back = v.screen_to_point(v.point_to_screen(p));
        assert!((back.st - 64.0).abs() < 1e-3);
    }

    /// Volume: 0% is the bottom edge, LEVEL_MAX is the top edge, and 100% is the middle.
    #[test]
    fn volume_axis_runs_0_to_200_percent_bottom_to_top() {
        let v = view(VerticalMode::Volume);
        assert!((v.y_of(0.0) - v.rect.bottom()).abs() < 1e-3);
        assert!((v.y_of(LEVEL_MAX) - v.rect.top()).abs() < 1e-3);
        assert!((v.y_of(100.0) - v.rect.center().y).abs() < 1e-3);
        let back = v.st_of(v.y_of(75.0));
        assert!((back - 75.0).abs() < 1e-6);
    }

    /// Volume: the value is clamped to the axis when the pointer is outside it.
    #[test]
    fn volume_value_is_clamped_to_the_axis() {
        let v = view(VerticalMode::Volume);
        assert_eq!(v.st_of(v.rect.top() - 100.0), LEVEL_MAX);
        assert_eq!(v.st_of(v.rect.bottom() + 100.0), 0.0);
    }
}

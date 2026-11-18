//! Editable document state for the M3 canvas (CANVAS-05, CANVAS-06).
//!
//! The document is the engine `Scene` plus UI-only state (current tool, pitch
//! view, selection). Undo keeps snapshots of the scene, which is cheap at
//! this size and avoids per-edit inverse logic. Snapshots are taken before
//! each edit, so undo and redo are whole-state swaps.

use synthesis_engine::{Scene, Settings, Shape, Voice};

/// Snapshot-based undo history.
#[derive(Default)]
pub struct History {
    undo: Vec<Scene>,
    redo: Vec<Scene>,
}

impl History {
    /// Record the state before an edit. Clears redo.
    pub fn push(&mut self, before: &Scene) {
        self.undo.push(before.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Swap the current scene with the previous snapshot.
    pub fn undo(&mut self, current: &mut Scene) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(current, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, current: &mut Scene) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(current, next));
                true
            }
            None => false,
        }
    }
}

/// Visible pitch range and scale (R8, GRID-03).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchView {
    pub lo_hz: f64,
    pub hi_hz: f64,
    pub scale: synthesis_engine::PitchScale,
}

impl Default for PitchView {
    fn default() -> Self {
        Self {
            lo_hz: 110.0,
            hi_hz: 1760.0,
            scale: synthesis_engine::PitchScale::Semitones,
        }
    }
}

/// Time view: visible start and width in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeView {
    pub start_s: f64,
    pub width_s: f64,
}

impl Default for TimeView {
    fn default() -> Self {
        Self {
            start_s: 0.0,
            width_s: 4.4,
        }
    }
}

/// The app's document: one scene plus view state.
pub struct Document {
    pub scene: Scene,
    /// User loop region in seconds (TRANS-02). `None` loops the whole timeline.
    pub loop_region: Option<synthesis_engine::LoopRegion>,
    pub history: History,
    pub pitch: PitchView,
    pub time: TimeView,
    pub selected_voice: usize,
}

impl Default for Document {
    fn default() -> Self {
        let mut scene = Scene::new(10.0, Settings::default());
        scene.voices.push(Voice::new("Voice 1"));
        Self {
            scene,
            loop_region: None,
            history: History::default(),
            pitch: PitchView::default(),
            time: TimeView::default(),
            selected_voice: 0,
        }
    }
}

impl Document {
    /// Apply `edit` to the scene as one undoable step. The edit runs only if
    /// it leaves the scene valid; otherwise the scene is restored (AGENTS:
    /// never leave an invalid document).
    pub fn edit<R>(&mut self, edit: impl FnOnce(&mut Scene) -> R) -> Option<R> {
        let before = self.scene.clone();
        let result = edit(&mut self.scene);
        if self.scene.validate().is_err() {
            self.scene = before;
            return None;
        }
        self.history.push(&before);
        Some(result)
    }

    /// Set the loop region, clamped to the timeline and at least `min_len_s` long.
    /// Returns false if the range is empty after clamping.
    pub fn set_loop(&mut self, start_s: f64, end_s: f64, min_len_s: f64) -> bool {
        let dur = self.scene.duration_s;
        let (a, b) = if start_s <= end_s {
            (start_s, end_s)
        } else {
            (end_s, start_s)
        };
        let a = a.clamp(0.0, dur);
        let b = b.clamp(0.0, dur).max(a + min_len_s);
        if b > dur {
            return false;
        }
        match synthesis_engine::LoopRegion::new(a, b) {
            Some(r) => {
                self.loop_region = Some(r);
                true
            }
            None => false,
        }
    }

    pub fn clear_loop(&mut self) {
        self.loop_region = None;
    }

    pub fn undo(&mut self) -> bool {
        self.history.undo(&mut self.scene)
    }

    pub fn redo(&mut self) -> bool {
        self.history.redo(&mut self.scene)
    }

    /// Add a validated shape to the selected voice.
    pub fn add_shape(&mut self, shape: Shape) -> bool {
        let idx = self.selected_voice;
        self.edit(|s| {
            if let Some(v) = s.voices.get_mut(idx) {
                v.shapes.push(shape);
            }
        })
        .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synthesis_engine::{Geometry, Point};

    fn line(t0: f64, t1: f64) -> Shape {
        Shape::outline(Geometry::Line {
            a: Point::new(t0, 60.0),
            b: Point::new(t1, 60.0),
        })
        .unwrap()
    }

    #[test]
    fn edit_then_undo_then_redo_restores_state() {
        let mut doc = Document::default();
        assert!(doc.add_shape(line(0.0, 1.0)));
        assert!(doc.add_shape(line(1.0, 2.0)));
        assert_eq!(doc.scene.voices[0].shapes.len(), 2);
        assert!(doc.undo());
        assert_eq!(doc.scene.voices[0].shapes.len(), 1);
        assert!(doc.undo());
        assert_eq!(doc.scene.voices[0].shapes.len(), 0);
        assert!(doc.redo());
        assert_eq!(doc.scene.voices[0].shapes.len(), 1);
    }

    #[test]
    fn loop_region_is_clamped_and_ordered() {
        let mut doc = Document::default(); // 10 s timeline
        assert!(doc.set_loop(6.0, 2.0, 0.1)); // reversed input is reordered
        let r = doc.loop_region.unwrap();
        assert_eq!((r.start_s, r.end_s), (2.0, 6.0));
        assert!(doc.set_loop(-3.0, 4.0, 0.1));
        assert_eq!(doc.loop_region.unwrap().start_s, 0.0, "clamped at zero");
        assert!(doc.set_loop(9.0, 50.0, 0.1));
        assert!(
            doc.loop_region.unwrap().end_s <= 10.0,
            "clamped at duration"
        );
    }

    #[test]
    fn loop_region_has_minimum_length() {
        let mut doc = Document::default();
        assert!(doc.set_loop(3.0, 3.0, 0.5));
        let r = doc.loop_region.unwrap();
        assert!(
            (r.len_s() - 0.5).abs() < 1e-12,
            "collapsed range is widened to min"
        );
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut doc = Document::default();
        doc.add_shape(line(0.0, 1.0));
        doc.undo();
        assert!(doc.history.can_redo());
        doc.add_shape(line(2.0, 3.0));
        assert!(!doc.history.can_redo());
    }

    /// An edit that would make the scene invalid is rolled back and not recorded.
    #[test]
    fn invalid_edit_is_rolled_back() {
        let mut doc = Document::default();
        let result = doc.edit(|s| {
            s.duration_s = 999.0; // exceeds the 60 s limit (R10)
        });
        assert!(result.is_none());
        assert_eq!(doc.scene.duration_s, 10.0);
        assert!(!doc.history.can_undo());
    }
}

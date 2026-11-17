//! Offline and block-wise stereo render (ENGINE-01, R5 to R7).
//!
//! Each voice produces one mono signal from its shapes and micro. Left and
//! right then differ only by their lane gains (R6). Voices sum linearly (R7),
//! and master gain applies after the sum.
//!
//! [`SceneRenderer`] keeps oscillator and gate state across calls, so playback
//! can pull consecutive blocks without restarting phase. [`render_scene`] is
//! one long block, so export and playback share the same code path.

use crate::error::Result;
use crate::model::{Scene, Settings, Voice};
use crate::synth::VoiceSynth;

/// A rendered stereo buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub sample_rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Rendered {
    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }
}

/// Stateful renderer that produces consecutive stereo blocks of a scene.
pub struct SceneRenderer {
    sample_rate: u32,
    master: f64,
    settings: Settings,
    voices: Vec<Voice>,
    synths: Vec<VoiceSynth>,
    next_index: usize,
}

impl SceneRenderer {
    /// Build a renderer for `scene`, validating it first. The renderer keeps
    /// its own copies of the voices, so it does not borrow the scene.
    pub fn new(scene: &Scene) -> Result<Self> {
        scene.validate()?;
        let voices: Vec<Voice> = scene.voices.iter().filter(|v| !v.muted).cloned().collect();
        let mut this = Self {
            sample_rate: scene.settings.sample_rate,
            master: scene.settings.master_gain,
            settings: scene.settings.clone(),
            voices,
            synths: Vec::new(),
            next_index: 0,
        };
        this.rebuild_synths();
        Ok(this)
    }

    fn rebuild_synths(&mut self) {
        self.synths = self
            .voices
            .iter()
            .map(|v| VoiceSynth::owned(v.clone(), &self.settings))
            .collect();
        self.next_index = 0;
    }

    /// Sample rate of produced blocks.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Render the next `len` frames, continuing from where the last call stopped.
    pub fn next_block(&mut self, len: usize) -> Rendered {
        let sr = f64::from(self.sample_rate);
        let mut left = vec![0.0f64; len];
        let mut right = vec![0.0f64; len];
        for (voice, synth) in self.voices.iter().zip(self.synths.iter_mut()) {
            for (k, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                let n = self.next_index + k;
                let s = synth.process(n);
                let t = n as f64 / sr;
                *l += s * voice.lane_left.gain_at(t);
                *r += s * voice.lane_right.gain_at(t);
            }
        }
        self.next_index += len;
        let m = self.master;
        Rendered {
            sample_rate: self.sample_rate,
            left: left.iter().map(|&x| (x * m) as f32).collect(),
            right: right.iter().map(|&x| (x * m) as f32).collect(),
        }
    }

    /// Restart from time zero, resetting phases and gates.
    pub fn reset(&mut self) {
        self.rebuild_synths();
    }
}

/// Render a scene offline at its settings' sample rate.
///
/// Muted voices are skipped. Output is not clipped here; integer export clamps.
pub fn render_scene(scene: &Scene) -> Result<Rendered> {
    let sr = scene.settings.sample_rate;
    let n = (scene.duration_s * f64::from(sr)).ceil() as usize;
    let mut renderer = SceneRenderer::new(scene)?;
    Ok(renderer.next_block(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Geometry, Point, Shape};
    use crate::micro::{DepthUnit, Micro, Preset};
    use crate::pitch::hz_to_st;

    fn scene() -> Scene {
        let st = |hz: f64| hz_to_st(hz, 440.0);
        let mut voice = Voice::new("v")
            .with_shape(
                Shape::filled(Geometry::Rectangle {
                    a: Point::new(0.0, st(200.0)),
                    b: Point::new(0.6, st(400.0)),
                })
                .unwrap(),
            )
            .with_shape(
                Shape::outline(Geometry::Line {
                    a: Point::new(0.2, st(450.0)),
                    b: Point::new(0.9, st(520.0)),
                })
                .unwrap(),
            );
        voice.micro = Some(Micro::preset(Preset::Triangle, 10.0, 10.0, DepthUnit::Hz));
        let mut s = Scene::new(1.0, Settings::default());
        s.settings.master_gain = 1.0;
        s.voices.push(voice);
        s
    }

    /// Blocks rendered in pieces must equal one long render: playback and export agree.
    #[test]
    fn block_rendering_matches_offline_render() {
        let s = scene();
        let whole = render_scene(&s).unwrap();
        let mut r = SceneRenderer::new(&s).unwrap();
        let mut left = Vec::new();
        let mut right = Vec::new();
        let total = whole.len();
        let mut produced = 0;
        for chunk in [1000usize, 4096, 333, 9000] {
            let take = chunk.min(total - produced);
            let b = r.next_block(take);
            left.extend_from_slice(&b.left);
            right.extend_from_slice(&b.right);
            produced += take;
        }
        let rest = r.next_block(total - produced);
        left.extend_from_slice(&rest.left);
        right.extend_from_slice(&rest.right);
        assert_eq!(left.len(), total);
        assert_eq!(left, whole.left, "blocks must match offline render exactly");
        assert_eq!(right, whole.right);
    }

    #[test]
    fn reset_restarts_from_time_zero() {
        let s = scene();
        let mut r = SceneRenderer::new(&s).unwrap();
        let first = r.next_block(4800);
        r.next_block(9000);
        r.reset();
        let again = r.next_block(4800);
        assert_eq!(first, again);
    }
}

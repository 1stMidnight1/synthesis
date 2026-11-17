//! Offline stereo render (ENGINE-01, R5 to R7).
//!
//! Each voice produces one mono signal from its shapes and micro. Left and
//! right then differ only by their lane gains (R6). Voices sum linearly (R7),
//! and master gain applies after the sum.

use crate::error::Result;
use crate::model::Scene;
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

/// Render a scene offline at its settings' sample rate.
///
/// Muted voices are skipped. Output is not clipped here; integer export clamps.
pub fn render_scene(scene: &Scene) -> Result<Rendered> {
    scene.validate()?;
    let sr = scene.settings.sample_rate;
    let sr_f = f64::from(sr);
    let n = (scene.duration_s * sr_f).ceil() as usize;
    let mut left = vec![0.0f64; n];
    let mut right = vec![0.0f64; n];

    for voice in scene.voices.iter().filter(|v| !v.muted) {
        let mut synth = VoiceSynth::new(voice, &scene.settings);
        for i in 0..n {
            let s = synth.process(i);
            let t = i as f64 / sr_f;
            left[i] += s * voice.lane_left.gain_at(t);
            right[i] += s * voice.lane_right.gain_at(t);
        }
    }

    let master = scene.settings.master_gain;
    Ok(Rendered {
        sample_rate: sr,
        left: left.iter().map(|&x| (x * master) as f32).collect(),
        right: right.iter().map(|&x| (x * master) as f32).collect(),
    })
}

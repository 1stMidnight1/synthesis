//! Offline stereo render (ENGINE-01, R6, R7).

use crate::error::Result;
use crate::model::Scene;
use crate::synth::VoiceSynth;

/// A rendered stereo buffer. Left and right carry identical partials (R6).
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
/// Voices sum linearly (R7). Muted voices are skipped. Master gain is applied
/// after the sum. Output is not clipped here; integer export clamps.
pub fn render_scene(scene: &Scene) -> Result<Rendered> {
    scene.validate()?;
    let sr = scene.settings.sample_rate;
    let n = (scene.duration_s * f64::from(sr)).ceil() as usize;
    let mut left = vec![0.0f64; n];
    let mut right = vec![0.0f64; n];

    for voice in scene.voices.iter().filter(|v| !v.muted) {
        let mut synth = VoiceSynth::new(voice, &scene.settings);
        for i in 0..n {
            let s = synth.process(i);
            left[i] += s;
            right[i] += s;
        }
    }

    let master = scene.settings.master_gain;
    Ok(Rendered {
        sample_rate: sr,
        left: left.iter().map(|&x| (x * master) as f32).collect(),
        right: right.iter().map(|&x| (x * master) as f32).collect(),
    })
}

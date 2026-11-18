//! Offline and block-wise stereo render (ENGINE-01, R5 to R7).
//!
//! Each voice produces one mono signal from its shapes and micro. Left and
//! right then differ only by their lane gains (R6). Voices sum linearly (R7),
//! and master gain applies after the sum.
//!
//! [`SceneRenderer`] keeps oscillator and gate state across calls, so playback
//! can pull consecutive blocks without restarting phase. [`render_scene`] is
//! one long block, so export and playback share the same code path.

use crate::error::{EngineError, Result};
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

/// Blocks shorter than this render voices on the calling thread: spawning
/// threads costs more than it saves on a short live-playback block.
pub const PARALLEL_MIN_FRAMES: usize = 8_192;

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

    /// Build a renderer for live playback at the output device's rate.
    ///
    /// Export is limited to 44.1 and 48 kHz (OUT-03), but an output device
    /// may run at another rate. Rendering at the device rate keeps pitch and
    /// tempo correct without resampling.
    pub fn for_playback(scene: &Scene, sample_rate: u32) -> Result<Self> {
        scene.validate()?;
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(EngineError::InvalidSettings(format!(
                "playback rate {sample_rate} Hz is out of range"
            )));
        }
        let mut s = scene.clone();
        s.settings.sample_rate = 48_000;
        let mut this = Self::new(&s)?;
        this.sample_rate = sample_rate;
        this.settings.sample_rate = sample_rate;
        this.rebuild_synths();
        Ok(this)
    }

    /// Timeline position of the next frame, in seconds.
    pub fn position_s(&self) -> f64 {
        self.next_index as f64 / f64::from(self.sample_rate)
    }

    /// Index of the next frame on the timeline.
    pub fn position_frames(&self) -> usize {
        self.next_index
    }

    /// Jump to frame `index` (see [`Self::seek`]).
    pub fn seek_frames(&mut self, index: usize) {
        self.next_index = index;
        for synth in &mut self.synths {
            synth.note_jump();
        }
    }

    /// Jump to `t_s` without rendering the skipped audio. Oscillator phases
    /// and gate levels carry over, so the jump is click-free; gates then ramp
    /// toward what sounds at the new time.
    pub fn seek(&mut self, t_s: f64) {
        self.seek_frames((t_s.max(0.0) * f64::from(self.sample_rate)).round() as usize);
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
    ///
    /// Each voice is rendered into its own buffer, on its own thread when
    /// there is more than one voice and the block is long enough to be worth
    /// it. The voice buffers are then summed in voice order, which is the same
    /// floating-point order as rendering them one after another, so the result
    /// does not depend on threading (R7).
    pub fn next_block(&mut self, len: usize) -> Rendered {
        let sr = f64::from(self.sample_rate);
        let start = self.next_index;
        let render_voice = |voice: &Voice, synth: &mut VoiceSynth| {
            let mut l = vec![0.0f64; len];
            let mut r = vec![0.0f64; len];
            for k in 0..len {
                let n = start + k;
                let s = synth.process(n);
                let t = n as f64 / sr;
                l[k] = s * voice.lane_left.gain_at(t);
                r[k] = s * voice.lane_right.gain_at(t);
            }
            (l, r)
        };
        let parts: Vec<(Vec<f64>, Vec<f64>)> =
            if self.synths.len() > 1 && len >= PARALLEL_MIN_FRAMES {
                std::thread::scope(|scope| {
                    let handles: Vec<_> = self
                        .voices
                        .iter()
                        .zip(self.synths.iter_mut())
                        .map(|(voice, synth)| scope.spawn(move || render_voice(voice, synth)))
                        .collect();
                    handles
                        .into_iter()
                        .map(|h| h.join().expect("voice render thread panicked"))
                        .collect()
                })
            } else {
                self.voices
                    .iter()
                    .zip(self.synths.iter_mut())
                    .map(|(voice, synth)| render_voice(voice, synth))
                    .collect()
            };
        let mut left = vec![0.0f64; len];
        let mut right = vec![0.0f64; len];
        for (l, r) in &parts {
            for k in 0..len {
                left[k] += l[k];
                right[k] += r[k];
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
                Shape::outline(Geometry::Rectangle {
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

    /// Seeking skips audio without rendering it. After the gate ramp, the
    /// audio is what sounds at the new time: a steady tone at its exact level,
    /// and silence in a gap. (A filled band's short-window level depends on
    /// the partials' relative phases, which a seek changes, so it is not
    /// compared sample-for-sample.)
    #[test]
    fn seek_lands_on_the_new_time() {
        let st = crate::pitch::hz_to_st(450.0, 440.0);
        let tone = |t0: f64, t1: f64| {
            Shape::outline(Geometry::Line {
                a: Point::new(t0, st),
                b: Point::new(t1, st),
            })
            .unwrap()
        };
        let mut s = Scene::new(2.0, Settings::default());
        s.settings.master_gain = 1.0;
        s.voices.push(
            Voice::new("v")
                .with_shape(tone(0.0, 0.5))
                .with_shape(tone(1.0, 2.0)),
        );
        let rms = |x: &[f32]| {
            (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
        };
        let mut r = SceneRenderer::new(&s).unwrap();
        r.next_block(4_800); // playing the first tone
        r.seek(0.7); // into the gap
        assert!((r.position_s() - 0.7).abs() < 1e-9);
        let gap = r.next_block(4_800);
        assert!(rms(&gap.left[480..]) < 1e-6, "gap after seek is silent");
        r.seek(1.5); // into the second tone
        let on = r.next_block(4_800);
        let level = rms(&on.left[480..]);
        // A unit sine has RMS 1/sqrt(2); a new voice plays through its default
        // 75% volume lane (DEFAULT_STROKE_PERCENT), so the expected level is 0.75 of that.
        let expected =
            std::f64::consts::FRAC_1_SQRT_2 * crate::lanes::DEFAULT_STROKE_PERCENT / 100.0;
        assert!(
            (level - expected).abs() < 0.01,
            "tone level {level}, expected {expected}"
        );
        // The first samples ramp in rather than jump (ENGINE-05).
        assert!(on.left[0].abs() < 0.01);
    }

    /// Playback at a device rate other than 48 kHz keeps pitch correct.
    #[test]
    fn playback_rate_keeps_pitch() {
        let st = crate::pitch::hz_to_st(441.0, 440.0);
        let mut s = Scene::new(0.5, Settings::default());
        s.voices.push(
            Voice::new("v").with_shape(
                Shape::outline(Geometry::Line {
                    a: Point::new(0.0, st),
                    b: Point::new(0.5, st),
                })
                .unwrap(),
            ),
        );
        let mut r = SceneRenderer::for_playback(&s, 44_100).unwrap();
        assert_eq!(r.sample_rate(), 44_100);
        let b = r.next_block(44_100 / 2);
        // Count rising zero crossings over the steady part: about 441 Hz * 0.4 s.
        let x = &b.left[2_205..];
        let ups = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let secs = x.len() as f64 / 44_100.0;
        let hz = ups as f64 / secs;
        assert!((hz - 441.0).abs() < 4.0, "measured {hz} Hz");
        assert!(SceneRenderer::for_playback(&s, 1_000).is_err());
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

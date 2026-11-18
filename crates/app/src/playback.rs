//! Background render thread and lock-free handoff to the audio callback
//! (TRANS-01, TRANS-02, TRANS-04).
//!
//! The render thread owns a `SceneRenderer` at the output device's sample
//! rate and keeps an `rtrb` ring about 0.25 s ahead of the audio callback.
//! Looping happens here, in the audio: at the loop end the renderer jumps back
//! to the loop start, so a loop is heard as a loop.
//!
//! Every frame carries the generation of the play command that produced it.
//! The audio callback plays only frames of the current generation, counts
//! them (the UI derives the playhead from that count, so the playhead shows
//! what is heard), and fades out the previous generation's frames for a few
//! milliseconds instead of cutting them, so stop and seek do not click.
//!
//! Edits during playback (`Command::Update`) swap the scene and continue from
//! the same timeline position, without flushing queued audio.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use synthesis_engine::{LoopRegion, Scene, SceneRenderer};

/// Frames per block pushed by the render thread.
pub const BLOCK_FRAMES: usize = 512;

/// Target audio buffered ahead of the callback, in seconds.
const AHEAD_S: f64 = 0.25;

/// One stereo frame in the ring, tagged with its play generation.
#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub l: f32,
    pub r: f32,
    pub generation: u32,
}

/// Commands from the UI thread to the render thread.
pub enum Command {
    /// Play `scene` from `start_s` at the device rate. `generation` must be
    /// new; the UI matches it against `Shared::played` for the playhead.
    Play {
        scene: Box<Scene>,
        start_s: f64,
        looping: Option<LoopRegion>,
        sample_rate: u32,
        generation: u32,
    },
    /// Replace the scene while playing, keeping the timeline position.
    Update {
        scene: Box<Scene>,
    },
    /// Stop; `generation` must be new.
    Stop {
        generation: u32,
    },
    Shutdown,
}

/// State shared by the UI, render thread, and audio callback. Atomics only.
pub struct Shared {
    /// Generation of the current play or stop command.
    pub generation: AtomicU32,
    /// `generation << 40 | frames played`, written by the audio callback.
    pub played: AtomicU64,
}

impl Shared {
    /// Frames played for `generation`, or 0 if the audio has not started it yet.
    pub fn played_frames(&self, generation: u32) -> u64 {
        let v = self.played.load(Ordering::Acquire);
        if (v >> 40) as u32 == generation & 0xFF_FFFF {
            v & ((1 << 40) - 1)
        } else {
            0
        }
    }

    pub fn pack(generation: u32, frames: u64) -> u64 {
        (u64::from(generation & 0xFF_FFFF) << 40) | (frames & ((1 << 40) - 1))
    }
}

/// Handle owned by the UI. Dropping it stops the render thread.
pub struct RenderHandle {
    commands: std::sync::mpsc::Sender<Command>,
    join: Option<JoinHandle<()>>,
    pub shared: Arc<Shared>,
}

impl RenderHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.commands.send(cmd);
    }
}

impl Drop for RenderHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Create the ring and spawn the render thread. Returns the UI handle and the
/// consumer end for the audio callback.
pub fn spawn(capacity: usize) -> (RenderHandle, Consumer<Frame>) {
    let (producer, consumer) = RingBuffer::<Frame>::new(capacity.max(BLOCK_FRAMES * 4));
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    let shared = Arc::new(Shared {
        generation: AtomicU32::new(0),
        played: AtomicU64::new(0),
    });
    let thread_shared = shared.clone();
    let join = thread::Builder::new()
        .name("synthesis-render".into())
        .spawn(move || render_loop(producer, rx, thread_shared))
        .expect("spawn render thread");
    (
        RenderHandle {
            commands: tx,
            join: Some(join),
            shared,
        },
        consumer,
    )
}

/// Playback position bookkeeping on the render thread, in frames.
struct Playing {
    renderer: SceneRenderer,
    /// Renderer for the scene before the last edit, faded out over
    /// `xfade_len` frames while the new one fades in, so edits do not click.
    old: Option<SceneRenderer>,
    xfade_left: usize,
    generation: u32,
    /// Loop start and end, if playback wraps.
    wrap: Option<(usize, usize)>,
    end: usize,
}

/// Crossfade length for live edits: 10 ms.
fn xfade_len(sample_rate: u32) -> usize {
    (sample_rate as usize / 100).max(1)
}

impl Playing {
    /// Mix the pre-edit renderer into a freshly rendered block, fading it out
    /// while the new audio fades in (equal-gain linear crossfade).
    fn crossfade(&mut self, left: &mut [f32], right: &mut [f32]) {
        let Some(old) = self.old.as_mut() else {
            return;
        };
        let len = xfade_len(self.renderer.sample_rate()) as f32;
        let n = left.len().min(self.xfade_left);
        let prev = old.next_block(n);
        for i in 0..n {
            let g_old = (self.xfade_left - i) as f32 / len;
            let g_new = 1.0 - g_old;
            left[i] = left[i] * g_new + prev.left[i] * g_old;
            right[i] = right[i] * g_new + prev.right[i] * g_old;
        }
        self.xfade_left -= n;
        if self.xfade_left == 0 {
            self.old = None;
        }
    }

    /// Frames to render next, wrapping at the loop end. 0 when finished.
    fn next_len(&mut self) -> usize {
        let mut pos = self.renderer.position_frames();
        if let Some((ls, le)) = self.wrap
            && pos >= le
        {
            self.renderer.seek_frames(ls);
            if let Some(old) = self.old.as_mut() {
                old.seek_frames(ls);
            }
            pos = ls;
        }
        let limit = match self.wrap {
            Some((_, le)) => le - pos,
            None => self.end.saturating_sub(pos),
        };
        limit.min(BLOCK_FRAMES)
    }
}

fn render_loop(
    mut producer: Producer<Frame>,
    rx: std::sync::mpsc::Receiver<Command>,
    shared: Arc<Shared>,
) {
    let mut playing: Option<Playing> = None;
    loop {
        loop {
            match rx.try_recv() {
                Ok(Command::Play {
                    scene,
                    start_s,
                    looping,
                    sample_rate,
                    generation,
                }) => {
                    shared.generation.store(generation, Ordering::Release);
                    playing = SceneRenderer::for_playback(&scene, sample_rate).ok().map(
                        |mut renderer| {
                            let sr = f64::from(sample_rate);
                            let frames = |t: f64| (t.max(0.0) * sr).round() as usize;
                            renderer.seek_frames(frames(start_s));
                            let start = frames(start_s);
                            let wrap = looping
                                .map(|r| (frames(r.start_s), frames(r.end_s)))
                                .filter(|&(ls, le)| le > ls && start < le);
                            Playing {
                                renderer,
                                old: None,
                                xfade_left: 0,
                                generation,
                                wrap,
                                end: frames(scene.duration_s),
                            }
                        },
                    );
                }
                Ok(Command::Update { scene }) => {
                    if let Some(p) = playing.as_mut()
                        && let Ok(mut r) =
                            SceneRenderer::for_playback(&scene, p.renderer.sample_rate())
                    {
                        r.seek_frames(p.renderer.position_frames());
                        p.old = Some(std::mem::replace(&mut p.renderer, r));
                        p.xfade_left = xfade_len(p.renderer.sample_rate());
                        p.end = (scene.duration_s * f64::from(p.renderer.sample_rate())).round()
                            as usize;
                    }
                }
                Ok(Command::Stop { generation }) => {
                    shared.generation.store(generation, Ordering::Release);
                    playing = None;
                }
                Ok(Command::Shutdown) => return,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        let mut worked = false;
        if let Some(p) = playing.as_mut() {
            let ahead = (AHEAD_S * f64::from(p.renderer.sample_rate())) as usize;
            let buffered = producer.buffer().capacity() - producer.slots();
            if buffered < ahead && producer.slots() >= BLOCK_FRAMES {
                let n = p.next_len();
                if n > 0 {
                    let mut block = p.renderer.next_block(n);
                    p.crossfade(&mut block.left, &mut block.right);
                    for (l, r) in block.left.iter().zip(block.right.iter()) {
                        let f = Frame {
                            l: *l,
                            r: *r,
                            generation: p.generation,
                        };
                        if producer.push(f).is_err() {
                            break;
                        }
                    }
                    worked = true;
                }
            }
        }
        if !worked {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Audio-callback side of the ring. Real-time safe: no allocation, no locks,
/// bounded work per frame.
pub struct Player {
    consumer: Consumer<Frame>,
    shared: Arc<Shared>,
    generation: u32,
    played: u64,
    /// Remaining fade-out frames for the previous generation.
    fade_left: u32,
    fade_len: u32,
    fade_generation: u32,
}

impl Player {
    pub fn new(consumer: Consumer<Frame>, shared: Arc<Shared>, sample_rate: u32) -> Self {
        let fade_len = (sample_rate / 200).max(1); // 5 ms
        Self {
            consumer,
            shared,
            generation: 0,
            played: 0,
            fade_left: 0,
            fade_len,
            fade_generation: 0,
        }
    }

    /// Fill `out` with interleaved samples for `channels` channels.
    /// Channel 0 is left, channel 1 right; further channels get left.
    pub fn fill(&mut self, out: &mut [f32], channels: usize) {
        let current = self.shared.generation.load(Ordering::Acquire);
        if current != self.generation {
            self.fade_generation = self.generation;
            self.fade_left = self.fade_len;
            self.generation = current;
            self.played = 0;
        }
        for frame in out.chunks_mut(channels.max(1)) {
            let (l, r) = self.next_frame();
            for (ch, s) in frame.iter_mut().enumerate() {
                *s = if ch == 1 { r } else { l };
            }
        }
        self.shared.played.store(
            Shared::pack(self.generation, self.played),
            Ordering::Release,
        );
    }

    fn next_frame(&mut self) -> (f32, f32) {
        // Bounded: at most the ring's capacity of stale frames is skipped.
        loop {
            let Ok(f) = self.consumer.peek().copied() else {
                return (0.0, 0.0);
            };
            if f.generation == self.generation {
                let _ = self.consumer.pop();
                self.played += 1;
                return (f.l, f.r);
            }
            let _ = self.consumer.pop();
            if f.generation == self.fade_generation && self.fade_left > 0 {
                let g = self.fade_left as f32 / self.fade_len as f32;
                self.fade_left -= 1;
                return (f.l * g, f.r * g);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use synthesis_engine::pitch::hz_to_st;
    use synthesis_engine::{Geometry, Point, Settings, Shape, Voice};

    const SR: u32 = 48_000;

    fn tone(t0: f64, t1: f64, hz: f64) -> Shape {
        let st = hz_to_st(hz, 440.0);
        Shape::outline(Geometry::Line {
            a: Point::new(t0, st),
            b: Point::new(t1, st),
        })
        .unwrap()
    }

    /// Master gain 100% and both volume lanes at 100%, so a tone has RMS 0.707
    /// and the assertions measure playback. A new voice's default lane is 75%
    /// (see `Voice::new`), which is tested on its own; here it is set to unity.
    fn scene(shapes: Vec<Shape>, dur: f64) -> Scene {
        let mut s = Scene::new(dur, Settings::default());
        s.settings.master_gain = 1.0;
        let mut v = Voice::new("v");
        v.shapes = shapes;
        v.lane_left = synthesis_engine::Lane::flat(100.0, dur);
        v.lane_right = synthesis_engine::Lane::flat(100.0, dur);
        s.voices.push(v);
        s
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Pull `frames` of mono-left output through the real audio-side Player,
    /// waiting for the render thread as a real device would.
    fn pull(player: &mut Player, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames);
        let mut buf = vec![0.0f32; 256 * 2];
        let deadline = Instant::now() + Duration::from_secs(10);
        while out.len() < frames {
            assert!(Instant::now() < deadline, "render thread too slow");
            if player.consumer.slots() < 256 {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            player.fill(&mut buf, 2);
            out.extend(buf.iter().step_by(2));
        }
        out.truncate(frames);
        out
    }

    fn start(
        scene: Scene,
        start_s: f64,
        looping: Option<LoopRegion>,
        generation: u32,
    ) -> (RenderHandle, Player) {
        let (h, c) = spawn(SR as usize);
        let player = Player::new(c, h.shared.clone(), SR);
        h.send(Command::Play {
            scene: Box::new(scene),
            start_s,
            looping,
            sample_rate: SR,
            generation,
        });
        // Wait for the render thread to accept the command.
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.shared.generation.load(Ordering::Acquire) != generation {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        (h, player)
    }

    #[test]
    fn plays_from_start_and_counts_frames() {
        let (h, mut p) = start(scene(vec![tone(0.0, 2.0, 450.0)], 2.0), 0.0, None, 1);
        let out = pull(&mut p, 9_600);
        assert!(rms(&out[960..]) > 0.6, "tone audible");
        // `pull` fills whole 256-frame device buffers.
        assert_eq!(h.shared.played_frames(1), 9_728);
        assert_eq!(h.shared.played_frames(2), 0, "other generations read 0");
    }

    /// The loop is in the audio: a tone only inside [0.2, 0.3) of a loop
    /// [0.2, 0.4) keeps coming back, every 0.2 s.
    #[test]
    fn loop_wraps_in_the_audio() {
        let sc = scene(vec![tone(0.2, 0.3, 450.0)], 2.0);
        let (_h, mut p) = start(sc, 0.2, LoopRegion::new(0.2, 0.4), 1);
        let out = pull(&mut p, 48_000); // 1 s = five loop passes
        for pass in 0..5 {
            let base = pass * 9_600;
            let on = rms(&out[base + 960..base + 3_840]);
            let off = rms(&out[base + 5_760..base + 9_120]);
            assert!(on > 0.5, "pass {pass}: tone in first half ({on})");
            assert!(off < 0.01, "pass {pass}: silence in second half ({off})");
        }
    }

    /// Stopping fades out over about 5 ms instead of cutting to zero.
    #[test]
    fn stop_fades_out_without_a_click() {
        let (h, mut p) = start(scene(vec![tone(0.0, 2.0, 450.0)], 2.0), 0.0, None, 1);
        pull(&mut p, 4_800);
        h.send(Command::Stop { generation: 2 });
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.shared.generation.load(Ordering::Acquire) != 2 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let mut buf = vec![0.0f32; 2 * 480];
        p.fill(&mut buf, 2);
        let left: Vec<f32> = buf.iter().step_by(2).copied().collect();
        // Largest step between samples stays small: no discontinuity.
        let max_step = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.1, "step {max_step}");
        assert!(rms(&left[300..]) < 1e-6, "silent after the fade");
    }

    /// Update swaps the scene without restarting: the same generation keeps
    /// counting (the playhead does not jump), and the edit is heard within
    /// the render-ahead latency (0.25 s).
    #[test]
    fn update_changes_sound_without_restarting() {
        let (h, mut p) = start(scene(vec![tone(0.0, 2.0, 450.0)], 2.0), 0.0, None, 1);
        let before = pull(&mut p, 4_800);
        assert!(rms(&before[960..]) > 0.6);
        h.send(Command::Update {
            scene: Box::new(scene(vec![], 2.0)),
        });
        let out = pull(&mut p, 24_000);
        assert!(rms(&out[14_400..]) < 1e-4, "edit heard within 0.3 s");
        assert!(
            h.shared.played_frames(1) >= 28_800,
            "same generation, no restart"
        );
        // No click at the switch: the tone ramps out (ENGINE-05).
        let max_step = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.1, "step {max_step}");
    }

    /// Non-looping playback stops producing audio at the end of the timeline.
    #[test]
    fn playback_ends_at_duration() {
        let (_h, mut p) = start(scene(vec![tone(0.0, 0.5, 450.0)], 0.5), 0.4, None, 1);
        let mut buf = vec![0.0f32; 2 * 256];
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut total = 0;
        while Instant::now() < deadline && total < 4_800 {
            if p.consumer.slots() >= 256 {
                p.fill(&mut buf, 2);
                total += 256;
            } else if total > 0 && p.consumer.slots() == 0 {
                thread::sleep(Duration::from_millis(20));
                if p.consumer.slots() == 0 {
                    break;
                }
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
        // 0.1 s remained: 4800 frames, rendered in whole blocks.
        let played = p.shared.played_frames(1);
        assert!(played <= 4_800 + 256, "played {played}");
    }
}

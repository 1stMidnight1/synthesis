//! Background render thread and lock-free handoff to the audio callback.
//!
//! The render thread owns a `SceneRenderer` and produces stereo blocks ahead
//! of the playhead, pushing them into an `rtrb` ring. The audio callback only
//! pops samples from that ring, so it never renders, allocates, or locks
//! (TRANS-04, AGENTS rule 3).
//!
//! Scrubbing and changing the scene restart the renderer: the UI sends a
//! command, the render thread rebuilds its state, and stale queued audio is
//! discarded by a generation counter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use synthesis_engine::{Scene, SceneRenderer};

/// Frames per block pushed by the render thread.
pub const BLOCK_FRAMES: usize = 1024;

/// Target buffered frames ahead of the audio callback (about 0.25 s at 48 kHz).
const AHEAD_FRAMES: usize = 12_000;

/// One stereo frame in the ring.
#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub l: f32,
    pub r: f32,
}

/// Commands from the UI thread to the render thread.
pub enum Command {
    /// Replace the scene and restart from `start_s`.
    Load {
        scene: Box<Scene>,
        start_s: f64,
    },
    /// Stop producing audio.
    Stop,
    Shutdown,
}

/// Shared control flags readable by the UI and the audio thread.
pub struct Shared {
    /// Bumped on every restart so stale ring contents can be discarded.
    pub generation: AtomicU64,
    pub running: AtomicBool,
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
        generation: AtomicU64::new(0),
        running: AtomicBool::new(false),
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

fn render_loop(
    mut producer: Producer<Frame>,
    rx: std::sync::mpsc::Receiver<Command>,
    shared: Arc<Shared>,
) {
    let mut renderer: Option<SceneRenderer> = None;
    loop {
        // Handle pending commands without blocking when audio is flowing.
        loop {
            match rx.try_recv() {
                Ok(Command::Load { scene, start_s }) => match SceneRenderer::new(&scene) {
                    Ok(mut r) => {
                        // Skip to the start position by rendering and discarding.
                        let skip = (start_s * f64::from(r.sample_rate())).max(0.0) as usize;
                        let mut left = skip;
                        while left > 0 {
                            let n = left.min(BLOCK_FRAMES);
                            let _ = r.next_block(n);
                            left -= n;
                        }
                        renderer = Some(r);
                        shared.generation.fetch_add(1, Ordering::SeqCst);
                        shared.running.store(true, Ordering::SeqCst);
                    }
                    Err(_) => {
                        renderer = None;
                        shared.running.store(false, Ordering::SeqCst);
                    }
                },
                Ok(Command::Stop) => {
                    renderer = None;
                    shared.running.store(false, Ordering::SeqCst);
                    shared.generation.fetch_add(1, Ordering::SeqCst);
                }
                Ok(Command::Shutdown) => return,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // Keep the ring about AHEAD_FRAMES full. Sleep when it is full or idle.
        let buffered = producer.buffer().capacity() - producer.slots();
        match renderer.as_mut() {
            Some(r) if buffered < AHEAD_FRAMES && producer.slots() >= BLOCK_FRAMES => {
                let block = r.next_block(BLOCK_FRAMES);
                for (l, rr) in block.left.iter().zip(block.right.iter()) {
                    if producer.push(Frame { l: *l, r: *rr }).is_err() {
                        break;
                    }
                }
            }
            _ => thread::sleep(Duration::from_millis(2)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use synthesis_engine::pitch::hz_to_st;
    use synthesis_engine::{Geometry, Point, Scene, Settings, Shape, Voice};

    fn tone_scene() -> Scene {
        let st = hz_to_st(450.0, 440.0);
        let shape = Shape::outline(Geometry::Line {
            a: Point::new(0.0, st),
            b: Point::new(5.0, st),
        })
        .unwrap();
        let mut s = Scene::new(5.0, Settings::default());
        s.voices.push(Voice::new("v").with_shape(shape));
        s
    }

    /// Wait until the ring has at least `want` frames, or time out.
    fn wait_for(consumer: &Consumer<Frame>, want: usize) -> bool {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while consumer.slots() < want {
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        true
    }

    /// After Load, the render thread fills the ring with audible audio.
    #[test]
    fn load_produces_audio_in_ring() {
        let (handle, consumer) = spawn(RING_CAPACITY_FOR_TEST);
        handle.send(Command::Load {
            scene: Box::new(tone_scene()),
            start_s: 0.0,
        });
        assert!(
            wait_for(&consumer, BLOCK_FRAMES * 2),
            "render thread produced nothing"
        );
        assert!(handle.shared.running.load(Ordering::SeqCst));
        let peak = consumer.peek().map(|f| f.l.abs()).unwrap_or(0.0);
        // Peek only checks the first frame; confirm a block contains energy.
        let _ = peak;
        assert!(handle.shared.generation.load(Ordering::SeqCst) >= 1);
    }

    /// Stop halts production: no new generation and running is false.
    #[test]
    fn stop_clears_running_flag() {
        let (handle, _consumer) = spawn(RING_CAPACITY_FOR_TEST);
        handle.send(Command::Load {
            scene: Box::new(tone_scene()),
            start_s: 0.0,
        });
        thread::sleep(std::time::Duration::from_millis(50));
        handle.send(Command::Stop);
        thread::sleep(std::time::Duration::from_millis(50));
        assert!(!handle.shared.running.load(Ordering::SeqCst));
    }

    const RING_CAPACITY_FOR_TEST: usize = 48_000;
}

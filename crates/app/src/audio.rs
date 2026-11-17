//! Audio output via cpal (CoreAudio on macOS, WASAPI on Windows).
//!
//! The audio callback pops pre-rendered stereo frames from an `rtrb` ring
//! filled by the render thread (see `playback.rs`). It never renders,
//! allocates, locks, or logs (TRANS-04, AGENTS rule 3). When the ring runs
//! dry it outputs silence rather than blocking.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use rtrb::Consumer;

use crate::playback::{Frame, Shared};

/// Gain applied at the output stage, so a stop or generation change fades out
/// rather than clicking.
const FADE_S: f32 = 0.005;

/// A running output stream. Dropping it stops audio.
pub struct AudioOut {
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

impl AudioOut {
    /// Open the default output device and pull frames from `consumer`.
    pub fn start(consumer: Consumer<Frame>, shared: Arc<Shared>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no default output device")?;
        let device_name = device
            .id()
            .map(|id| id.to_string())
            .unwrap_or_else(|_| "unknown".into());
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();

        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(&device, &config, consumer, shared),
            SampleFormat::F64 => build::<f64>(&device, &config, consumer, shared),
            SampleFormat::I16 => build::<i16>(&device, &config, consumer, shared),
            SampleFormat::I32 => build::<i32>(&device, &config, consumer, shared),
            SampleFormat::U16 => build::<u16>(&device, &config, consumer, shared),
            other => return Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;

        Ok(Self {
            _stream: stream,
            device_name,
            sample_rate: config.sample_rate,
            channels: config.channels,
            sample_format,
        })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut consumer: Consumer<Frame>,
    shared: Arc<Shared>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let fade_step = 1.0 / (FADE_S * config.sample_rate as f32);
    let mut gain = 0.0f32;
    let mut seen_generation = shared.generation.load(Ordering::SeqCst);

    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                // On a new generation, discard frames rendered for the old scene.
                let current = shared.generation.load(Ordering::SeqCst);
                if current != seen_generation {
                    seen_generation = current;
                    while consumer.pop().is_ok() {}
                }
                let playing = shared.running.load(Ordering::Relaxed);
                let target = if playing { 1.0 } else { 0.0 };
                for frame in data.chunks_mut(channels) {
                    let diff = target - gain;
                    gain += diff.clamp(-fade_step, fade_step);
                    let f = consumer.pop().unwrap_or_default();
                    let (l, r) = (f.l * gain, f.r * gain);
                    for (ch, out) in frame.iter_mut().enumerate() {
                        let s = if channels >= 2 && ch == 1 { r } else { l };
                        *out = T::from_sample(s);
                    }
                }
            },
            // Error callback runs off the real-time path; printing is fine here.
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

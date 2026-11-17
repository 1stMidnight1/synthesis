//! Audio output via cpal (CoreAudio on macOS, WASAPI on Windows).
//!
//! The callback hands the device buffer to `playback::Player`, which only pops
//! pre-rendered frames from a lock-free ring (TRANS-04, AGENTS rule 3). For
//! non-f32 devices the callback converts from a fixed scratch buffer that is
//! allocated once, before the stream starts.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use rtrb::Consumer;

use crate::playback::{Frame, Player, Shared};

/// Largest device buffer converted per chunk, in samples.
const SCRATCH: usize = 8192;

/// A running output stream. Dropping it stops audio.
pub struct AudioOut {
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

impl AudioOut {
    /// Open the default output device and play frames from `consumer`.
    /// Playback renders at the device's own rate, so no resampling is needed.
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
        let player = Player::new(consumer, shared, config.sample_rate);

        let stream = match sample_format {
            SampleFormat::F32 => build_f32(&device, &config, player),
            SampleFormat::F64 => build::<f64>(&device, &config, player),
            SampleFormat::I16 => build::<i16>(&device, &config, player),
            SampleFormat::I32 => build::<i32>(&device, &config, player),
            SampleFormat::U16 => build::<u16>(&device, &config, player),
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

fn build_f32(
    device: &cpal::Device,
    config: &StreamConfig,
    mut player: Player,
) -> Result<Stream, String> {
    let channels = usize::from(config.channels);
    device
        .build_output_stream(
            *config,
            move |data: &mut [f32], _| player.fill(data, channels),
            // Error callback runs off the real-time path; printing is fine here.
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut player: Player,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let chunk = (SCRATCH / channels) * channels;
    let mut scratch = vec![0.0f32; chunk];
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                for out in data.chunks_mut(chunk) {
                    let buf = &mut scratch[..out.len()];
                    player.fill(buf, channels);
                    for (o, s) in out.iter_mut().zip(buf.iter()) {
                        *o = T::from_sample(*s);
                    }
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

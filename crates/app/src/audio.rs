//! Audio output via cpal (CoreAudio on macOS, WASAPI on Windows).
//!
//! The UI thread talks to the audio callback only through atomics, so the
//! callback never blocks, allocates, or logs (TRANS-04, AGENTS.md rule 3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use synthesis_engine::osc::SineOsc;

/// An `f32` that can be shared lock-free between threads.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// Parameters shared between the UI and the audio callback.
#[derive(Debug)]
pub struct Shared {
    pub playing: AtomicBool,
    pub freq_hz: AtomicF32,
    /// Linear master gain, 0..1 (OUT-01).
    pub gain: AtomicF32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            freq_hz: AtomicF32::new(440.0),
            gain: AtomicF32::new(0.7),
        }
    }
}

/// A running output stream. Dropping it stops audio.
pub struct AudioOut {
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

impl AudioOut {
    /// Open the default output device and start a stream driven by `shared`.
    pub fn start(shared: Arc<Shared>) -> Result<Self, String> {
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
            SampleFormat::F32 => build::<f32>(&device, &config, shared),
            SampleFormat::F64 => build::<f64>(&device, &config, shared),
            SampleFormat::I16 => build::<i16>(&device, &config, shared),
            SampleFormat::I32 => build::<i32>(&device, &config, shared),
            SampleFormat::U16 => build::<u16>(&device, &config, shared),
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
    shared: Arc<Shared>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let sample_rate = f64::from(config.sample_rate);
    let channels = usize::from(config.channels);
    let mut osc = SineOsc::default();
    // Smoothed gain avoids clicks when toggling play or moving the slider.
    let mut current_gain = 0.0f32;
    let ramp_per_sample = 1.0 / (0.005 * sample_rate as f32); // 5 ms full-scale ramp

    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                let target = if shared.playing.load(Ordering::Relaxed) {
                    shared.gain.load()
                } else {
                    0.0
                };
                let freq = f64::from(shared.freq_hz.load());
                for frame in data.chunks_mut(channels) {
                    let diff = target - current_gain;
                    current_gain += diff.clamp(-ramp_per_sample, ramp_per_sample);
                    let s = osc.next(freq, sample_rate) as f32 * current_gain;
                    let v = T::from_sample(s);
                    for out in frame.iter_mut() {
                        *out = v;
                    }
                }
            },
            // Error callback runs off the real-time path; printing is fine here.
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

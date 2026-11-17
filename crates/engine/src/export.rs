//! WAV export (OUT-03). Stereo, 44.1 or 48 kHz, 16-bit, 24-bit, or 32-bit float.

use std::io::{Seek, Write};
use std::path::Path;

use crate::error::{EngineError, Result};
use crate::render::Rendered;

/// Sample format for export (OUT-03). Default is 24-bit (D-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitDepth {
    Int16,
    #[default]
    Int24,
    Float32,
}

/// Write a rendered buffer to a WAV file.
pub fn write_wav_file(path: &Path, rendered: &Rendered, depth: BitDepth) -> Result<()> {
    let file = std::fs::File::create(path)?;
    write_wav(std::io::BufWriter::new(file), rendered, depth)
}

/// Write a rendered buffer to any seekable writer.
pub fn write_wav<W: Write + Seek>(writer: W, rendered: &Rendered, depth: BitDepth) -> Result<()> {
    if rendered.sample_rate != 44_100 && rendered.sample_rate != 48_000 {
        return Err(EngineError::InvalidSettings(
            "export rate must be 44100 or 48000".to_string(),
        ));
    }
    let (bits, format) = match depth {
        BitDepth::Int16 => (16, hound::SampleFormat::Int),
        BitDepth::Int24 => (24, hound::SampleFormat::Int),
        BitDepth::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rendered.sample_rate,
        bits_per_sample: bits,
        sample_format: format,
    };
    let mut w = hound::WavWriter::new(writer, spec)?;
    for (l, r) in rendered.left.iter().zip(rendered.right.iter()) {
        for &x in &[*l, *r] {
            match depth {
                BitDepth::Int16 => w.write_sample(to_int(x, 15))?,
                BitDepth::Int24 => w.write_sample(to_int(x, 23))?,
                BitDepth::Float32 => w.write_sample(x)?,
            }
        }
    }
    w.finalize()?;
    Ok(())
}

/// Convert a float in -1..1 to a signed integer with `bits` magnitude bits.
/// Out-of-range values are clamped (clip).
fn to_int(x: f32, bits: u32) -> i32 {
    let max = ((1i64 << bits) - 1) as f64;
    (f64::from(x).clamp(-1.0, 1.0) * max).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn tiny() -> Rendered {
        Rendered {
            sample_rate: 48_000,
            left: vec![0.0, 0.5, -0.5, 1.0],
            right: vec![0.0, -0.25, 0.25, -1.0],
        }
    }

    fn roundtrip(depth: BitDepth) -> (hound::WavSpec, Vec<f32>) {
        let mut buf = Cursor::new(Vec::new());
        write_wav(&mut buf, &tiny(), depth).unwrap();
        buf.set_position(0);
        let mut reader = hound::WavReader::new(buf).unwrap();
        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
            hound::SampleFormat::Int => {
                let scale = ((1i64 << (spec.bits_per_sample - 1)) - 1) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.unwrap() as f32 / scale)
                    .collect()
            }
        };
        (spec, samples)
    }

    #[test]
    fn writes_requested_bit_depths() {
        let (s16, _) = roundtrip(BitDepth::Int16);
        assert_eq!((s16.bits_per_sample, s16.channels), (16, 2));
        assert_eq!(s16.sample_rate, 48_000);
        let (s24, _) = roundtrip(BitDepth::Int24);
        assert_eq!(s24.bits_per_sample, 24);
        let (sf, _) = roundtrip(BitDepth::Float32);
        assert_eq!(sf.sample_format, hound::SampleFormat::Float);
        assert_eq!(sf.bits_per_sample, 32);
    }

    #[test]
    fn sample_values_survive_round_trip() {
        for depth in [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32] {
            let (_, got) = roundtrip(depth);
            let want = [0.0, 0.0, 0.5, -0.25, -0.5, 0.25, 1.0, -1.0];
            assert_eq!(got.len(), want.len());
            let tol = match depth {
                BitDepth::Int16 => 1.0 / 32_767.0,
                _ => 1.0 / 8_388_607.0,
            };
            for (g, w) in got.iter().zip(want.iter()) {
                assert!((g - w).abs() <= tol + 1e-7, "{depth:?}: {g} != {w}");
            }
        }
    }

    #[test]
    fn rejects_unsupported_rate() {
        let mut r = tiny();
        r.sample_rate = 96_000;
        let mut buf = Cursor::new(Vec::new());
        assert!(write_wav(&mut buf, &r, BitDepth::Int24).is_err());
    }
}

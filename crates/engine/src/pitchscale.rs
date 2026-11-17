//! Pitch-axis scale modes and note labels (R8, GRID-03, GRID-04).
//!
//! The canvas maps a pitch to a vertical position in one of three modes.
//! Drawing is never snapped to notes; labels are reference only (R8).

use crate::pitch::{MIDI_A4, hz_to_st, st_to_hz};

/// Y-axis mode (R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PitchScale {
    /// Log, note labels. Default.
    #[default]
    Semitones,
    /// Log, Hz labels.
    LogHz,
    /// Linear Hz.
    LinearHz,
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Note name with octave for a MIDI note number, e.g. 69 gives "A4".
/// Octaves follow the MIDI convention (MIDI 60 is C4).
pub fn note_name(midi: i32) -> String {
    let name = NOTE_NAMES[midi.rem_euclid(12) as usize];
    let octave = midi.div_euclid(12) - 1;
    format!("{name}{octave}")
}

/// Convert a pitch (fractional MIDI) to the canvas's 0..1 vertical fraction
/// for the given scale. 0 is the bottom, 1 the top.
///
/// The visible range is `[lo_hz, hi_hz]`, both positive with `lo_hz < hi_hz`.
/// Log modes are logarithmic in Hz, so semitones and log Hz are the same
/// mapping and differ only in labels. Linear mode is linear in Hz.
pub fn to_fraction(scale: PitchScale, st: f64, lo_hz: f64, hi_hz: f64, a4_hz: f64) -> f64 {
    let hz = st_to_hz(st, a4_hz);
    match scale {
        PitchScale::Semitones | PitchScale::LogHz => {
            let lo = lo_hz.ln();
            let hi = hi_hz.ln();
            (hz.ln() - lo) / (hi - lo)
        }
        PitchScale::LinearHz => (hz - lo_hz) / (hi_hz - lo_hz),
    }
}

/// Inverse of [`to_fraction`]: pitch in fractional MIDI at a 0..1 position.
pub fn from_fraction(scale: PitchScale, f: f64, lo_hz: f64, hi_hz: f64, a4_hz: f64) -> f64 {
    let hz = match scale {
        PitchScale::Semitones | PitchScale::LogHz => {
            (lo_hz.ln() + f * (hi_hz.ln() - lo_hz.ln())).exp()
        }
        PitchScale::LinearHz => lo_hz + f * (hi_hz - lo_hz),
    };
    hz_to_st(hz, a4_hz)
}

/// Whether a MIDI note number is the reference A4 (used for emphasis).
pub fn is_a4(midi: i32) -> bool {
    midi == MIDI_A4 as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_match_midi() {
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(21), "A0");
        assert_eq!(note_name(70), "A#4");
        assert!(is_a4(69));
    }

    #[test]
    fn fraction_round_trips_in_every_mode() {
        for scale in [
            PitchScale::Semitones,
            PitchScale::LogHz,
            PitchScale::LinearHz,
        ] {
            for st in [40.0, 60.5, 69.0, 90.0] {
                let f = to_fraction(scale, st, 20.0, 20_000.0, 440.0);
                let back = from_fraction(scale, f, 20.0, 20_000.0, 440.0);
                assert!((back - st).abs() < 1e-9, "{scale:?} {st} -> {back}");
            }
        }
    }

    #[test]
    fn log_modes_are_equally_spaced_in_octaves() {
        let f1 = to_fraction(PitchScale::Semitones, 69.0, 20.0, 20_000.0, 440.0);
        let f2 = to_fraction(PitchScale::Semitones, 81.0, 20.0, 20_000.0, 440.0);
        // One octave is the same fraction everywhere on a log axis.
        let oct_a = f2 - f1;
        let f3 = to_fraction(PitchScale::Semitones, 57.0, 20.0, 20_000.0, 440.0);
        let oct_b = f1 - f3;
        assert!((oct_a - oct_b).abs() < 1e-12);
    }

    #[test]
    fn linear_mode_is_linear_in_hz() {
        let f = to_fraction(
            PitchScale::LinearHz,
            hz_to_st(600.0, 440.0),
            100.0,
            1100.0,
            440.0,
        );
        assert!((f - 0.5).abs() < 1e-9);
    }
}

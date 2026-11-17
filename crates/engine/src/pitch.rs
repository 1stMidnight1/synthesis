//! Pitch conversions.
//!
//! Geometry and the lattice use fractional MIDI note numbers ("semitones"),
//! where 69.0 is A4. Hz is derived only at synthesis time, from the A4
//! reference in settings (R8). Changing A4 shifts all sound uniformly and
//! leaves the drawing unchanged.

/// MIDI note number of A4, the pitch-space origin.
pub const MIDI_A4: f64 = 69.0;

/// Lowest audible frequency used by the lattice and tone bank.
pub const AUDIBLE_MIN_HZ: f64 = 20.0;

/// Highest audible frequency used by the lattice and tone bank.
pub const AUDIBLE_MAX_HZ: f64 = 20_000.0;

/// Convert a fractional MIDI note number to Hz.
pub fn st_to_hz(st: f64, a4_hz: f64) -> f64 {
    a4_hz * ((st - MIDI_A4) / 12.0).exp2()
}

/// Convert Hz to a fractional MIDI note number.
pub fn hz_to_st(hz: f64, a4_hz: f64) -> f64 {
    MIDI_A4 + 12.0 * (hz / a4_hz).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_round_trip() {
        assert!((st_to_hz(69.0, 440.0) - 440.0).abs() < 1e-9);
        assert!((st_to_hz(81.0, 440.0) - 880.0).abs() < 1e-9);
        let st = hz_to_st(450.0, 440.0);
        assert!((st_to_hz(st, 440.0) - 450.0).abs() < 1e-9);
    }
}

//! Grid divisions typed as `1/N` (GRID-02, GRID-03).
//!
//! The X (time) division is a fraction of a beat, so `1/4` is a quarter-beat
//! and `1/16` a sixteenth. The Y (pitch) division is a fraction of a semitone
//! in semitone mode (`1/4` is a quarter-tone step), or a fraction of an octave
//! in Hz modes, so the grid is the same as a log step either way.

use crate::transport::Tempo;

/// Smallest and largest accepted denominators. `1/1` is the beat; `1/256` is
/// finer than a single screen pixel at any zoom the app allows.
pub const MIN_DENOM: u32 = 1;
pub const MAX_DENOM: u32 = 256;

/// A grid division: one step is `1 / denom` of its unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Division {
    pub denom: u32,
}

impl Division {
    /// Create a division, rejecting denominators outside 1..=256.
    pub fn new(denom: u32) -> Option<Self> {
        (MIN_DENOM..=MAX_DENOM)
            .contains(&denom)
            .then_some(Self { denom })
    }

    /// Parse text typed as `1/N` or as just `N`. Whitespace is ignored.
    pub fn parse(text: &str) -> Option<Self> {
        let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let digits = match t.strip_prefix("1/") {
            Some(rest) => rest,
            None => t.as_str(),
        };
        let n: u32 = digits.parse().ok()?;
        Self::new(n)
    }

    /// The text shown in the field: `1/N`.
    pub fn text(self) -> String {
        format!("1/{}", self.denom)
    }

    /// X step in seconds: a `1/N` fraction of one beat at `tempo`.
    pub fn x_step_s(self, tempo: Tempo) -> f64 {
        tempo.beat_s() / f64::from(self.denom)
    }

    /// Y step in semitones: a `1/N` fraction of a semitone.
    pub fn y_step_st(self) -> f64 {
        1.0 / f64::from(self.denom)
    }
}

/// Y step in octaves for the Hz modes: a `1/N` fraction of an octave.
pub fn y_step_octaves(d: Division) -> f64 {
    1.0 / f64::from(d.denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_1_over_n_and_bare_n() {
        assert_eq!(Division::parse("1/16"), Division::new(16));
        assert_eq!(Division::parse(" 1 / 8 "), Division::new(8));
        assert_eq!(Division::parse("12"), Division::new(12));
        assert_eq!(Division::parse("1/1"), Division::new(1));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Division::parse("").is_none());
        assert!(Division::parse("1/0").is_none());
        assert!(Division::parse("1/257").is_none());
        assert!(
            Division::parse("2/8").is_none(),
            "numerator other than 1 is not a grid"
        );
        assert!(Division::parse("abc").is_none());
        assert!(Division::parse("1/-4").is_none());
    }

    #[test]
    fn text_round_trips() {
        let d = Division::new(32).unwrap();
        assert_eq!(d.text(), "1/32");
        assert_eq!(Division::parse(&d.text()), Some(d));
    }

    #[test]
    fn x_step_is_a_fraction_of_a_beat() {
        let t = Tempo::new(120.0).unwrap(); // beat = 0.5 s
        assert!((Division::new(4).unwrap().x_step_s(t) - 0.125).abs() < 1e-12);
        assert!((Division::new(1).unwrap().x_step_s(t) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn y_step_is_a_fraction_of_a_semitone() {
        assert!((Division::new(4).unwrap().y_step_st() - 0.25).abs() < 1e-12);
        assert!((y_step_octaves(Division::new(2).unwrap()) - 0.5).abs() < 1e-12);
    }
}

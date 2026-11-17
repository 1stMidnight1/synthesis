//! Engine error type. Plain enum, no UI or backend types.

use std::fmt;

#[derive(Debug)]
pub enum EngineError {
    /// A shape has invalid or degenerate geometry (CANVAS-02).
    InvalidShape(String),
    /// Settings are outside their documented ranges (ENGINE-03, OUT-03).
    InvalidSettings(String),
    /// A scene is invalid, for example longer than the 60 s limit (R10).
    InvalidScene(String),
    /// File I/O failure during WAV export.
    Io(std::io::Error),
    /// WAV encoder failure.
    Wav(hound::Error),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape(m) => write!(f, "invalid shape: {m}"),
            Self::InvalidSettings(m) => write!(f, "invalid settings: {m}"),
            Self::InvalidScene(m) => write!(f, "invalid scene: {m}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Wav(e) => write!(f, "WAV error: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<hound::Error> for EngineError {
    fn from(e: hound::Error) -> Self {
        Self::Wav(e)
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

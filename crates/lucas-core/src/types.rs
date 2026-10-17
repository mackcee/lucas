use serde::{Deserialize, Serialize};
use std::fmt;

/// Milliseconds on a caller-owned monotonic timeline, never a wall clock.
/// Map game death timestamps onto this same timeline before submitting them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreError {
    InvalidConfig(&'static str),
    OutOfOrder,
    DuplicateSample,
    OutsideSession,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(f, "invalid core configuration: {reason}"),
            Self::OutOfOrder => f.write_str("timestamps must be monotonic"),
            Self::DuplicateSample => f.write_str("gaze samples must have distinct timestamps"),
            Self::OutsideSession => f.write_str("timestamp is outside the observed session"),
        }
    }
}

impl std::error::Error for CoreError {}

pub(crate) fn elapsed(previous: Option<Timestamp>, at: Timestamp) -> Result<u64, CoreError> {
    previous.map_or(Ok(0), |previous| {
        at.0.checked_sub(previous.0).ok_or(CoreError::OutOfOrder)
    })
}

/// No-face samples use quality = 0. Nonfinite or out-of-range values are unknown,
/// not evidence that the player is looking away. Probabilities are in [0, 1].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GazeSample {
    pub at: Timestamp,
    pub p_minimap: f64,
    pub quality: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GazeState {
    #[default]
    Unknown,
    Elsewhere,
    Minimap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Glance {
    pub started_at: Timestamp,
    pub confirmed_at: Timestamp,
    pub ended_at: Timestamp,
    pub dwell_ms: u64,
}

/// Caller supplies game policy (including map/mode exclusions) via active_game.
/// A window-only adapter can set active_game/alive and game_time_ms > 90_000.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateInputs {
    pub active_game: bool,
    pub alive: bool,
    pub game_time_ms: u64,
    pub foreground: bool,
    pub face_tracked: bool,
    pub paused: bool,
}

impl GateInputs {
    pub fn resets_gap(self) -> bool {
        !self.active_game || !self.alive || self.game_time_ms <= 90_000
    }

    pub fn eligible(self) -> bool {
        !self.resets_gap() && self.foreground && self.face_tracked && !self.paused
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeathClassification {
    Blind,
    Seen,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeathEvent {
    pub at: Timestamp,
    pub classification: DeathClassification,
    pub gap_ms: Option<u64>,
    pub tracked_fraction: f64,
}

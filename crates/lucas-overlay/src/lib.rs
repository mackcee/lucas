//! Minimap-only, non-activating overlay and deterministic pulse rendering.
//!
//! Callers supply physical desktop coordinates from `lucas_geometry`, decide
//! when coaching is eligible, and poll for geometry changes. The Win32 backend
//! independently gates presentation on target-window eligibility. This crate
//! does not inspect the game process, install hooks, schedule reminders, or play
//! sound. See the crate README for lifecycle and Windows integration details.

#![deny(unsafe_op_in_unsafe_fn)]

mod recording;
pub mod render;
#[cfg(windows)]
pub mod win32;

pub use lucas_core::Salience;
pub use lucas_geometry::Rect;
pub use recording::{Event, Recording};
use std::{fmt, time::Duration};

/// Independent visual settings; no renderer-specific handles or scheduling state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PulseStyle {
    #[default]
    Ripple,
    BreathingBorder,
    CornerBrackets,
    CometSweep,
}

/// RGB colour. Alpha is controlled by the salience envelope, not a user colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour(pub u8, pub u8, pub u8);

impl Colour {
    pub const AMBER: Self = Self(255, 184, 56);
    pub const CYAN: Self = Self(67, 214, 235);
    pub const WHITE: Self = Self(255, 255, 255);
}

/// A single finite animation. Repeated calls restart it; hide cancels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pulse {
    pub style: PulseStyle,
    pub colour: Colour,
    pub salience: Salience,
}

impl Default for Pulse {
    fn default() -> Self {
        Self {
            style: PulseStyle::Ripple,
            colour: Colour::AMBER,
            salience: Salience::Medium,
        }
    }
}

impl Pulse {
    pub const DURATION: Duration = Duration::from_millis(1200);

    pub fn ripple_count(self) -> u8 {
        match self.salience {
            Salience::Minimal | Salience::Soft => 1,
            Salience::Medium | Salience::Strong => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayError {
    InvalidRect,
    TargetNotForeground,
    ExclusiveFullscreen,
    Closed,
    Platform { operation: &'static str, code: u32 },
    ThreadPanicked,
}

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRect => f.write_str("overlay rectangle is out of range or too large"),
            Self::TargetNotForeground => f.write_str("target window is not in the foreground"),
            Self::ExclusiveFullscreen => f.write_str(
                "exclusive fullscreen is unsupported; switch League to Borderless or Windowed",
            ),
            Self::Closed => f.write_str("overlay has shut down"),
            Self::Platform { operation, code } => write!(f, "{operation} failed (OS error {code})"),
            Self::ThreadPanicked => f.write_str("overlay thread panicked"),
        }
    }
}

impl std::error::Error for OverlayError {}
pub type Result<T> = std::result::Result<T, OverlayError>;

/// All mutations are acknowledged before returning. Shutdown is idempotent;
/// other operations after shutdown return `Closed`. Geometry is physical pixels.
/// Coaching eligibility belongs to the caller; Win32 enforces window eligibility.
pub trait Overlay: Send {
    fn show_pulse(&mut self, pulse: Pulse) -> Result<()>;
    fn hide(&mut self) -> Result<()>;
    fn set_target_rect(&mut self, rect: Rect) -> Result<()>;
    /// Call on the application's foreground poll, never from an OS hook.
    fn reassert_topmost(&mut self) -> Result<()>;
    fn shutdown(&mut self) -> Result<()>;
}

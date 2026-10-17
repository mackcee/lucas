//! Pure, clock-free minimap habit training.
//!
//! [`Session`] composes the detector, eligible-time gap clock, manual ramp/fade
//! programme, pulse policy, and metrics. All inputs use caller-owned monotonic
//! millisecond timestamps. No I/O, wall clocks, camera images, OS entropy, or
//! automatic promotion/probe decisions are used here.
//!
//! # Example
//! ```
//! use lucas_core::{GateInputs, GazeSample, SeededRng, Session, SessionConfig,
//!                  Timestamp, TRAINING_LEVELS};
//! let mut session = Session::new("game-1", SessionConfig::default(),
//!                                TRAINING_LEVELS[1], SeededRng::new(42))?;
//! let gate = GateInputs { active_game: true, alive: true, game_time_ms: 91_000,
//!                         foreground: true, face_tracked: true, paused: false };
//! let update = session.observe(GazeSample { at: Timestamp(0), p_minimap: 0.1,
//!                                          quality: 1.0 }, gate)?;
//! assert!(update.eligible);
//! let record = session.finish(Timestamp(100))?;
//! assert_eq!(record.schema_version, 1);
//! # Ok::<(), lucas_core::CoreError>(())
//! ```

#![forbid(unsafe_code)]

mod detector;
mod gap;
mod metrics;
mod policy;
mod session;
mod types;

pub use detector::*;
pub use gap::*;
pub use metrics::*;
pub use policy::*;
pub use session::*;
pub use types::*;

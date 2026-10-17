//! Gaze coaching, explicit timer fallback, post-game summaries and opt-in diagnostics.
#![deny(unsafe_op_in_unsafe_fn)]

#[forbid(unsafe_code)]
pub mod calibration;
pub mod eval;
pub mod postgame;
pub mod preferences;
pub mod runtime;
pub mod script;
#[forbid(unsafe_code)]
pub mod settings;
pub mod timer;
pub mod trace;

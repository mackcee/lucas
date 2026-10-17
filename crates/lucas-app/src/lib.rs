//! Timer mode, standalone post-game summary, and opt-in numeric gaze diagnostics.
#![deny(unsafe_op_in_unsafe_fn)]

#[forbid(unsafe_code)]
pub mod postgame;
#[forbid(unsafe_code)]
pub mod settings;
pub mod script;
pub mod timer;
pub mod trace;

//! Shared tray/geometry host with gaze coaching and an explicit timer fallback.
pub mod alignment;
#[cfg(windows)]
mod alignment_panel;
pub mod alignment_store;
#[cfg(windows)]
mod alignment_window;
#[cfg(windows)]
mod native;
pub mod report;
pub mod scheduling;
pub mod settings;
#[cfg(windows)]
pub use native::{run, run_timer};

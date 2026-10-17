//! Timer-mode orchestration. No gaze or per-game storage is involved.
pub mod alignment;
pub mod alignment_store;
#[cfg(windows)]
mod alignment_window;
#[cfg(windows)]
mod native;
pub mod report;
pub mod scheduling;
pub mod settings;
#[cfg(windows)]
pub use native::run;

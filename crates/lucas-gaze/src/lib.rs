//! Offline webcam-to-feature pipeline. No network inference, frame recording,
//! or pixel logging. UniGaze's owned local child receives RGB over anonymous pipes.
//!
//! Coordinates refer to the **unmirrored** camera image, not the screen. Feature
//! samples require downstream calibration/classification before `lucas-core` can
//! apply its dwell detector.

// Only the argument-free MF clock read in camera is allowed to use unsafe.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]

pub mod calibration;
pub mod camera;
pub mod decision;
pub mod features;
pub mod filter;
pub mod landmarks;
pub mod latest;
pub mod models;
pub mod runtime;
pub mod source;
#[cfg(feature = "ort")]
pub mod tracker;
pub mod unigaze;

pub use camera::{Camera, Frame, FrameRing, ImageCamera};
pub use features::{GazeFeatures, HeadPose};
pub use landmarks::{FaceLandmarks, Landmark, Landmarker, MediaPipeLandmarker};
pub use source::{GazeSample, GazeSource, WebcamGazeSource};

/// Errors deliberately contain no pixels, tensors, or upstream debug dumps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidFrame,
    InvalidTimestamp,
    CameraUnavailable,
    CameraTimestamp,
    CaptureFailed,
    ImageReadFailed,
    ModelReadFailed,
    ModelHashMismatch,
    InvalidModel,
    InferenceFailed,
    InvalidOutput,
    InvalidFilter,
    ThreadPriority,
    Source(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidFrame => "invalid frame dimensions or RGB buffer",
            Self::InvalidTimestamp => "sample timestamps must increase",
            Self::CameraUnavailable => "camera unavailable or requested format unsupported",
            Self::CameraTimestamp => {
                "camera supplied no usable acquisition timestamps for 3 seconds; reconnect the webcam or update its driver"
            }
            Self::CaptureFailed => "camera frame capture failed",
            Self::ImageReadFailed => "fixture image could not be read",
            Self::ModelReadFailed => "model missing or unreadable; run cargo xtask models prepare",
            Self::ModelHashMismatch => "model SHA-256 does not match the compiled model lock",
            Self::InvalidModel => "unsupported model contract",
            Self::InferenceFailed => "inference failed",
            Self::InvalidOutput => "invalid model output",
            Self::InvalidFilter => "invalid One-Euro filter parameters",
            Self::ThreadPriority => "could not lower capture/inference thread priority",
            Self::Source(message) => message,
        })
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

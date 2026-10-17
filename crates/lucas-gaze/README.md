# lucas-gaze

Offline, single-player webcam → landmarks → filtered gaze features → per-user
minimap classifier. Camera capture remains independent of calibration and decisions.

- `Camera::capture` fills caller-owned RGB8 storage; `false` means end-of-source.
- `NokhwaCamera` (Windows/default `camera` feature) uses synchronous Media
  Foundation capture, explicit 1280×720 MJPEG/30fps for the C270, pure-Rust JPEG
  decoding, and below-normal priority on its calling thread. Acquisition timestamps
  are mapped relative to a fixed epoch at stream opening, not decoding completion;
  missing or pre-epoch timestamps fail capture rather than inventing fresh timing.
  Unsupported modes return a recoverable error rather than silently changing
  resolution/frame rate.
- `ImageCamera` reads existing still-image fixtures. `TraceGazeSource` replays
  numeric samples from an iterator; it is not a recorder or file-format promise.
- `MediaPipeLandmarker<OrtBackend>` runs a CPU-only, one-thread pipeline with
  rotated crops and single-face weighted NMS. Detection runs only initially or
  after tracking loss, an error, a frame-size change, or a >250 ms frame gap.
  Face presence is a sigmoid, not the raw model logit. All 478 points, including
  ten iris points, are projected to the unmirrored full-camera coordinate system.
- `WebcamGazeSource` owns a fixed three-frame ring. Processing is synchronous;
  there is no producer queue to accumulate delayed images. Caller scheduling
  should drop work instead of enqueueing frames, and drop the source to release
  the camera when inactive. It clears pixels after inference on success/error.
- `GazeSample { features: None, quality: 0.0, .. }` is **unknown**, not a glance.
  An outer `None` is end-of-source. Eyes closed/occluded, invalid geometry, and
  out-of-frame eyes become unknown. Quality is a heuristic, not calibrated
  accuracy or a guarantee the player is looking at the screen.
- Feature vectors contain each iris in its eye-corner/lid coordinate system,
  aperture, canonical-fit yaw/pitch/roll and inverse relative face scale. The
  latter is not distance in metres. One-Euro filters use monotonic timestamps
  and reset on loss/gaps. `calibration` maps these unmirrored features into screen
  coordinates and minimap probability; `decision` bridges into the core detector.

## Construct on a dedicated worker thread (Windows)

```rust,no_run
# #[cfg(all(windows, feature = "camera", feature = "ort"))]
# fn demo() -> lucas_gaze::Result<()> {
use lucas_gaze::{GazeSource, MediaPipeLandmarker, WebcamGazeSource};
use lucas_gaze::{camera::NokhwaCamera, models::VerifiedModels, runtime::OrtBackend};
let models = VerifiedModels::load(std::path::Path::new("models"))?;
let camera = NokhwaCamera::open_c270(0)?;
let landmarker = MediaPipeLandmarker::new(OrtBackend::new(models)?);
let mut source = WebcamGazeSource::new(camera, landmarker)?;
while let Some(sample) = source.next_sample()? {
    // Send numeric samples (never Frame) to calibration/core.
    let _ = sample;
}
# Ok(())
# }
```

Use `--no-default-features` for math/replay only. Default Linux builds run fixture
inference but never open a live camera; the MVP capture backend is Windows only.

## Per-user calibration (M5.1)

`calibration::fit(training, validation, screen, options)` standardizes the ten
features using **training-only** means/stddevs and fits an L2-regularized,
class-balanced logistic classifier plus two ridge regressors (screen x/y).
Supply distinct target occurrences to fit and threshold validation; never split
adjacent frames from one fixation between them. Both classes need at least 12
usable samples in each set. Defaults: regularization 0.01, empirical target
per-frame FPR 0.001. Thresholds are at least 0.5; boundary ties are excluded.

The held-out mean angular error is measured between screen rays using physical
monitor dimensions and an explicitly recorded assumed viewing distance. The
minimap zone is inflated by that error via `lucas-geometry`, clipped to screen.
`predict` returns `None` for no features, closed eyes, low quality (<0.5), extreme
head angles, or an off-screen ridge estimate. The inflated zone is a guard on
logistic positives, **never** a second way to create checks. Point estimates are
diagnostic, not precise screen gaze. Webcam off-screen rejection is a heuristic;
real recordings, especially boundary looks, remain essential.

`decision::CalibratedDetector` uses the existing `lucas-core::GlanceDetector`:
>=3 consecutive samples AND >=100 ms, exit at 75% of the fitted enter threshold,
250 ms maximum sample gap. Unknown interrupts a candidate/confirmed glance.
No core configuration or game-loop behavior is changed by calibration.

`CalibrationModel::{to_bytes,from_bytes}` encode a **553-byte**, little-endian
`LUCASC01` version-1 payload: standardization, classifier/regression weights,
geometry/distance assumption, threshold, target FPR and validation ROC/error/counts.
The existing store calibration blob wraps these bytes and desktop geometry; JSON
byte-array expansion makes the containing file larger. No training samples persist.
Unknown versions, wrong lengths, invalid geometry and nonfinite weights fail closed.

Short calibration validation passes at >=90% per-frame sensitivity, empirical
FPR <= target, >=80% usable frames and <=5° mean error. This is a local quality
screen, **not G2 acceptance**: short, correlated samples cannot establish an
unseen-population false-positive bound. `cargo xtask eval` measures held-out
**glance events** and false detections/minute on longer recordings instead.

## Privacy boundary

There is no frame writer, serializer, networking, or pixel logging. `Frame` debug
output is redacted and owned RGB allocations are scrubbed on clear/drop. Model
input staging buffers are scrubbed after inference. Drivers, decoders, and runtime
arenas may allocate transient working memory; this is not a claim of OS-level
locked memory or protection from swap/process dumps. Do not configure a crash
reporter to upload process memory. Models are always loaded locally with compiled
SHA-256 verification and runtime telemetry disabled.

See [models](../../models/README.md) for provenance, conversion, tests and the CPU
guard. Linux fixture tests and an MSVC cross-check do not replace live Windows
C270/low-light validation at the M4.2 gate.

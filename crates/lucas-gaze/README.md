# lucas-gaze (M4.1)

Offline, single-player webcam → landmarks → filtered gaze features. This crate
**does not classify minimap looks**: its samples are inputs to later calibration.

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
  and reset on loss/gaps. Mirroring, screen coordinates, calibration, off-screen
  classification and dwell detection are the caller's later-stage responsibilities.

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

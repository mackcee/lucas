# Offline gaze models

UniGaze-H + SFD/FAN is the optional default NVIDIA tracker after its
[one-time setup](unigaze/README.md). Its weights are **not committed or bundled**;
the installer verifies published assets and installs a private CUDA runtime.

The two checked-in ONNX files are the selectable MediaPipe CPU fallback:

| File | Input (RGB, NHWC) | Outputs |
| --- | --- | --- |
| `face_detector.onnx` | `[1,128,128,3]`, `[-1,1]`, square letterbox | `regressors [1,896,16]`, `classificators [1,896,1]` (logits) |
| `face_landmarks_detector.onnx` | `[1,256,256,3]`, `[0,1]`, rotated ROI | `Identity [1,1,1,1434]` (478 xyz points in crop pixels), `Identity_1` (presence logit), `Identity_2` (unused) |

**No models are fetched at application runtime.** Ship these files in the
directory passed to `VerifiedModels::load`, which checks them against the
SHA-256 lock compiled into `lucas-gaze`, then hands the **same verified bytes** to
ONNX Runtime. Missing or modified files fail closed; restore the model files
shipped with the matching application build rather than changing the lock.
Replacing the on-disk lock cannot bypass verification. ONNX Runtime telemetry is
explicitly disabled, and its runtime model-fetching feature is not enabled.

## Provenance and attribution

Google MediaPipe, Face Landmarker **float16/version 1** task bundle, retrieved
2026-10-17:

<https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task>

- Bundle SHA-256: `64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff`
- `face_detector.tflite`: `b4578f35940bf5a1a655214a1cce5cab13eba73c1297cd78e1a04c2380b0152f`
- `face_landmarks_detector.tflite`: `c7d54204ce0448474c7f3fa9af494787c0965cbdd6f20fc72867e43046bd43d5`
- Converted files: [SHA256SUMS](SHA256SUMS).

Copyright The MediaPipe Authors / Google LLC, **Apache-2.0**; see
[NOTICE](../NOTICE) and [LICENSE-APACHE](../LICENSE-APACHE). ONNX conversion changes
format/precision representation, not weights through training. The bundle's
blendshape model is not included or used in M4.1. Head pose is a weak-perspective
similarity fit to seven MediaPipe canonical vertices, not camera-calibrated metric
3D reconstruction. See the [calibration reference](../crates/lucas-gaze/README.md#per-user-calibration-m51)
for the personal screen mapping and minimap decision.

Reference decoder contracts:
[detector graph](https://github.com/google-ai-edge/mediapipe/blob/v0.10.32/mediapipe/tasks/cc/vision/face_detector/face_detector_graph.cc),
[landmark graph](https://github.com/google-ai-edge/mediapipe/blob/v0.10.32/mediapipe/tasks/cc/vision/face_landmarker/face_landmarks_detector_graph.cc),
[canonical mesh](https://github.com/google-ai-edge/mediapipe/blob/v0.10.32/mediapipe/modules/face_geometry/data/canonical_face_model.obj).

## Verify or convert (development only)

These checks use the committed MediaPipe models, without model downloads. See the
[root build guide](../README.md#build-and-check) for tool prerequisites, including
Python for the UniGaze IPC tests run by `cargo test` on Unix:

```sh
cargo xtask models verify
cargo test -p lucas-gaze
cargo test -p lucas-gaze --no-default-features # portable math/trace and IPC tests
cargo bench -p lucas-gaze --bench pipeline
```

`ort` downloads its native **runtime library at build time**, not models. Releases
must package the matching runtime library where required by the target. Defaults
for the MediaPipe fallback use one CPU inference thread, no parallel operators,
no idle spin and no GPU. UniGaze uses its own explicit CUDA child, not this ORT runtime.

To convert once, on Linux x86_64 with Python 3.12 (use a temporary venv):

```sh
python3.12 -m venv /tmp/lucas-model-venv
/tmp/lucas-model-venv/bin/pip install -r models/requirements.txt
cargo xtask models prepare --python /tmp/lucas-model-venv/bin/python --compare models
cargo xtask models verify
```

The development command downloads only the fixed official bundle, verifies the
archive and both source hashes, converts with pinned tf2onnx/TensorFlow/ONNX and
opset 17, and records the resulting SHA-256s alongside the ONNX files.
**Re-conversion is not bit-reproducible:** the lock pins the exact shipped files,
not the result of a future conversion. No equality with the old byte hashes is
required. The optional `--compare models` checks both networks numerically on
portrait-derived inputs before installation (absolute tolerance 1e-3, relative
1e-4). This checks conversion parity, not eye-tracking accuracy.

Prefer `--output /tmp/new-models --compare models` to review changes separately.
Review the new models and lock and commit them together. Rebuild the app to trust
a changed lock. Never accept hashes from an untrusted sidecar at runtime.
The converter removes embedded temporary source paths before distribution.

## CPU guard

The benchmark uses process CPU time, 10 warm-up frames, and 100 measured frames.
It separately measures raw mesh inference and the tracked pipeline (crop + mesh +
features), asserts only one detection is needed for a stable face, and fails if
the tracked pipeline exceeds a 33.333 ms one-core/30fps budget. The portable mesh
baseline is 20 ms with a 25% regression allowance; this is not a claim that every
CPU matches the scout's approximately 3.5 ms. On the original class of CPU, run:

```sh
LUCAS_MESH_BASELINE_MS=3.5 cargo bench -p lucas-gaze --bench pipeline
```

Pin hardware/power policy for comparisons. Wall-clock timing on shared CI is not a
3.5 ms performance guarantee. Capture/decode time and real C270 accuracy must be
measured on Windows; they are not included in the Linux fixture benchmark.

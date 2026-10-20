# UniGaze-H local CUDA tracker

This optional install adds the official **UniGaze-H/14 joint** model (632M
parameters) with the **SFD + 2D-FAN** GPU face frontend. It does not establish G2
accuracy. Real-webcam calibration and G2 acceptance are **pending**.

## One-time Windows install

From the extracted LUCAS bundle, with a current NVIDIA driver and roughly 12 GB
free disk space (downloads, Python/CUDA wheels, caches and weights):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\models\unigaze\setup.ps1
if ($LASTEXITCODE -ne 0) { throw 'UniGaze setup failed' }
.\lucas.exe --check-gaze-model | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'CUDA model diagnostic failed' }
.\lucas.exe --calibrate
```

Review the downloaded build/scripts before running them. The execution-policy
exception applies only to this PowerShell process. No administrator rights,
system Python, CUDA SDK, manual sidecar launch or camera access is needed for
setup/check. Setup installs Python 3.12.11, PyTorch 2.7.1 + CUDA 12.8, torchvision
0.22.1, face-alignment 1.4.1, timm 0.3.2 and hash-locked transitive dependencies
under this directory. Keep the bundle at that location: virtual environments are
not relocatable. Rerun setup after moving it. Do not run setup while LUCAS is open.

`setup.ps1 -InstallDirectory <directory>` supports isolated installation testing.
For app discovery that directory must be `<models-directory>\unigaze`; pass the
parent using `lucas --models <models-directory>` in diagnostics. Normal tray use
looks beside `lucas.exe`. Deleting this directory removes the optional runtime
and weights, not saved app data. Keep the installer files to reinstall later.

The installer is the only networked part. App startup verifies all asset hashes
and reads the source directly from its pinned ZIP. There are **no runtime model
downloads**, uploads or remote inference. The `READY` marker is created only when
setup succeeds. A missing install uses the visibly labelled MediaPipe fallback;
an installed but broken CUDA/model stack fails with an explicit error instead of
silently running UniGaze on the CPU. `LUCAS_GAZE_SOURCE=unigaze` requires UniGaze;
`LUCAS_GAZE_SOURCE=mediapipe` explicitly selects the original tracker. Changing
source requires a new full calibration.

## Reproducibility and provenance

[`assets.json`](assets.json) pins SHA-256 for every source/weight/diagnostic asset:

- Official code: [ut-vision/UniGaze](https://github.com/ut-vision/UniGaze) revision
  `9c240fbe33f3d6146970a77b7c8fa06a7e60019e`.
- H/14 joint checkpoint:
  `a336e7234738e9a9517fc6af7a9bc69cee16958388ad648d48c0f6b0df42ac8f`.
- Published SFD `s3fd-619a316812.pth` and 2D-FAN `2DFAN4-cd938726ad.zip`.
- Public MediaPipe portrait, **not a webcam recording**; see
  [fixture attribution](../../crates/lucas-gaze/tests/fixtures/README.md).

`prepare.py` downloads to temporary files and verifies before replacing them.
The original safetensors checkpoint is loaded strictly, then converted to FP16
in memory with `model.eval().half().to('cuda:0')`; there is no retraining or
nonreproducible ONNX conversion. SFD/FAN stay FP32 on CUDA. Source weights and
installed runtimes are ignored by Git and excluded from the release allowlist.
Do not edit a hash to make a failed download pass.

Regenerate the Windows dependency lock deliberately and review the change:

```sh
uv pip compile models/unigaze/requirements.in --python-version 3.12 \
  --python-platform x86_64-pc-windows-msvc \
  --extra-index-url https://download.pytorch.org/whl/cu128 \
  --index-strategy unsafe-best-match --generate-hashes \
  -o models/unigaze/requirements-windows.lock
python3 -m unittest discover -s models/unigaze -p 'test_*.py'
```

Published upstream code/weights retain their own licenses. UniGaze uses
MG-NC-RAI-2.0 (license retained inside `source.zip`); its normalization code carries
CC BY-NC-SA 4.0 and the Zhang/Sugano/Bulling ETRA 2018 citation. UniGaze citation:
Jiawei Qin, Xucong Zhang and Yusuke Sugano, *UniGaze: Towards Universal Gaze
Estimation via Large-scale Pre-Training*, WACV 2026. face-alignment and its
SFD/FAN model provenance are described by
[1adrianb/face-alignment](https://github.com/1adrianb/face-alignment).
These downloads are not relicensed under LUCAS's MIT option.

## Runtime contract

LUCAS starts and owns a local Python child, communicating only through anonymous
pipes. It waits at most 120 s for verified model loading and full-path warmup,
5 s for a frame response, and 60 s for the diagnostic. Timeout, malformed output,
CUDA failure or process death is a visible tracking error. Stopping during
startup cancels the wait; on Windows the child runs below normal priority without
a console window. Drop kills/reaps the
child and joins its pipe worker. The separate camera-driver capture timeout
contract still applies; it is not a hard deadline for a blocked driver.

Capture independently replaces **one latest frame slot** while inference is
busy. Only one RGB frame is in flight; no frame queue or timestamp rewriting.
The child echoes the original nanosecond acquisition timestamp, checked by Rust.
Frames are never written to disk. Rust frame buffers and received Python byte
buffers are scrubbed; temporary NumPy/OpenCV/PyTorch allocator copies, driver
memory, swap and crash dumps are not secure-erasure guarantees.

The official six-point face template, solvePnP refinement, 2x face crop, camera
surrogate, 960 focal/600 distance/224×224 normalization, RGB ImageNet layout and
inverse-rotation gaze denormalization are retained. The adapter starts with RGB,
so it does **not** repeat the reference video's BGR-to-RGB channel swap. Missing,
ambiguous, invalid or closed-eye faces remain unknown. Only two camera-space gaze
angles enter the personal ridge fit; apertures, pose and face centre remain
quality/diagnostic features. Calibration identity and traces distinguish these
from MediaPipe iris ratios.

Execution is explicit `cuda:0`; CPU fallback is not supported. cuDNN autotuning
is disabled again after face-alignment initialization (upstream enables it), and
the CUDA caching allocator is capped at 8 GiB or device capacity, whichever is
smaller. The diagnostic reports allocated/reserved memory and the peak including
startup, full-pipeline and model-only median/p95 latency. It runs 3 measured-path
warmups plus 20 inferences on the public portrait, never on a camera.

For GPU development verification, run the installed Python on `validate.py
--root <install-directory>` from this source checkout. It compares FP16 against
the original FP32 checkpoint on the public portrait and synthetic dim/shift
variants (maximum 0.25° camera-ray difference), and requires exact RGB/BGR warp
layout parity. It prints numeric JSON only. This proof and the native Windows
headless check must be recorded on the target GPU; CPU-only CI cannot establish
them or real-webcam accuracy.

A native Rust/ORT port is follow-up work. The sidecar is deliberate: preserving
the reference frontend/normalization matters more for this calibration experiment
than reimplementing it. FAN ONNX export is feasible; this release does not claim
otherwise or claim ONNX parity tests for a runtime it does not use.

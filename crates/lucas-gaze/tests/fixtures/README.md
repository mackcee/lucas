# Gaze fixtures

`portrait.jpg` is the MediaPipe test asset (Apache-2.0, The MediaPipe Authors / Google LLC),
retrieved 2026-10-17 from <https://storage.googleapis.com/mediapipe-assets/portrait.jpg>.
SHA-256: `a6f11efaa834706db23f275b6115058fa87fc7f14362681e6abe14e82749de3e`.
See the repository NOTICE and LICENSE-APACHE. It is a public upstream still, not
camera data captured or persisted by LUCAS.

Tests check full-image detection, all 478 landmarks, iris-within-eye geometry,
tracking without repeated detection, loss on black pixels, reacquisition, frame
gap reset, and end-of-file. Unit tests add synthetic
anchors, padding, rotation, canonical-pose recovery, eye closure and numeric traces.
These fixtures do **not** establish minimap classification accuracy, robustness to
all skin tones, reflections or low light, or live Windows camera compatibility.

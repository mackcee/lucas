# nokhwa-bindings-windows 0.4.6 (LUCAS patch)

Vendored from the crates.io release of `nokhwa-bindings-windows` 0.4.6,
https://github.com/l1npengtul/nokhwa. Copyright 2022 l1npengtul and the
Nokhwa Contributors; Apache-2.0 (see LICENSE-APACHE).

LUCAS modifies `src/lib.rs` to expose `raw_sample_bytes`, which returns the
unshifted Media Foundation `ReadSample` time in 100-nanosecond units converted
to `Duration`. Missing, nonpositive and overflowing timestamps remain absent.
The existing `raw_bytes` API retains its epoch-added timestamp contract for
other callers.

This avoids trying to reconstruct the private backend epoch from a later wall
clock read. LUCAS consumes the raw sample API directly; its
[acquisition timing contract](../../crates/lucas-gaze/README.md#acquisition-timing)
owns clock mapping, frame rejection and timeout behavior.

LUCAS also fixes `format_refreshed` to decode `MF_MT_FRAME_RATE` as a packed
numerator/denominator rather than truncating it to the denominator. The pure
`src/frame_rate.rs` helper follows `compatible_format_list`: use the high 32-bit
numerator only when the low 32-bit denominator is 1; otherwise return 0 (unsupported).
This preserves the exact-format check, including 1280×720 MJPEG at 30 fps.
Its regression tests also run on Linux through
`cargo test -p lucas-gaze --test windows_frame_rate` in the workspace test suite.

The root Cargo patch applies to both the capture and device-enumeration paths.
Keep these patches until upstream exposes equivalent raw timing information and
correctly decodes the refreshed frame rate.

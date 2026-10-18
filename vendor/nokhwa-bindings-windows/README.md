# nokhwa-bindings-windows 0.4.6 (LUCAS patch)

Vendored from the crates.io release of `nokhwa-bindings-windows` 0.4.6,
https://github.com/l1npengtul/nokhwa. Copyright 2022 l1npengtul and the
Nokhwa Contributors; Apache-2.0 (see LICENSE-APACHE).

LUCAS modifies `src/lib.rs` to expose `raw_sample_bytes`, which returns the
unshifted Media Foundation `ReadSample` time in 100-nanosecond units converted
to `Duration`. Missing, nonpositive and overflowing timestamps remain absent. The existing
`raw_bytes` API retains its epoch-added timestamp contract for other callers.

This avoids trying to reconstruct the private backend epoch from a later wall
clock read. LUCAS consumes the raw sample API directly; QPC-based timestamps
are mapped using the Media Foundation system clock. Only samples aged 0–1 second
are accepted. Stream-relative timestamps have no established acquisition origin
and receive no special handling: times outside that age bound are skipped
without being labeled. Three continuous seconds of invalid timestamps stop
capture with an actionable error. No receipt-time fallback is used.

The root Cargo patch applies to both the capture and device-enumeration paths.
Keep this patch until upstream exposes equivalent raw timing information.

# Post-game summary window

This chunk adds `lucas_app::postgame`, independent of the tray, camera debug
window, and future game loop. It never opens automatically during play.

See the root README's [viewer commands](../../README.md#preview-the-post-game-card)
for synthetic and saved-game previews.

An empty or missing database produces an explanatory error without launching a
window. The native UI targets Windows; Linux builds/headless tests and Xvfb are
useful for development. **Open data folder** launches Explorer on Windows. Demo
mode disables data controls and identifies all its data as synthetic.

## Integration at game end

After finalizing and saving a summary with `lucas-store`:

```rust,ignore
let card = lucas_app::postgame::PostGameCard::from_store(store, &summary.id)?;
lucas_app::postgame::show_postgame(card)?;
```

`show_postgame` starts an eframe event loop and belongs on the main/UI thread.
An already-running UI should embed `PostGameCard::show(ui)` instead. Game-end
routing, suppressing the card during play, and pause-aware event clock mapping
remain the later loop's responsibility. Follow the storage reference's
[deletion prerequisites](../lucas-store/README.md#privacy-controls) first:
ownership transfers into the card so **Delete all data** can close SQLite before
deleting files. Deletion requires a second,
explicit confirmation and clears the in-memory card on success. If deleting
fails, keep the card open, close other LUCAS windows, and choose **Delete all data**
again. The card can retry even if the database or summary has already been removed.

## Reading the card

- C uses core's **eligible time**, not all elapsed game time. Undefined C is shown
  as unavailable; tracking quality and the amount of eligible time remain visible.
- Blind / total includes unknown deaths in the total but never in the blind count.
  Each expandable death row gives its elapsed evidence gap, classification-window
  coverage, and timeline of checks/pulses in game seconds relative to death.
  The elapsed gap is shown separately, not as a band on the pause-aware timeline.
  Unknown gaps are explicitly uncertain; no earlier evidence is not fabricated
  as a zero gap.
- Probe lines require at least two measured probes. Lines join only consecutive
  fixed-standard probes with the same goal; missing or changed goals break the
  line. Low tracking gets a warning. These are raw observations, not a promotion
  assessment or a streak.
- Wheels text is derived from `lucas-core::TrainingLevel`. The next target is
  phase/level-specific guidance for the planned programme. Core currently exposes
  only **manual** levels, so the card explicitly does not claim to evaluate or
  perform automatic promotions.

The UI uses eframe/egui 0.36 and egui_plot 0.37 (the latter targets egui 0.36).
The default font notices are retained under `licenses/`; distributions should
include that directory. The dependency policy exception is scoped to egui's
bundled font package. No advisory exception was added.

```sh
cargo test -p lucas-app
cargo clippy -p lucas-app --all-targets -- -D warnings
```

# Gaze diagnostics and numeric traces

M4.2 provides the Windows `lucas` debug window and an opt-in fullscreen glance
recorder. The library contains portable script, trace-format and replay-summary
logic; it does not start a camera or touch disk merely by being linked. See
[Windows testing](../../docs/testing-on-windows.md) for the tester checklist.
Calibration, classification, tray controls and the live pulse loop are not here.

## Data boundary

The UI and recorder receive only `GazeSample`, derived feature arrays and numeric
telemetry. RGB never crosses that boundary; `lucas-gaze` clears its in-memory frame
ring after inference. There is no screenshot, video, image serializer, upload,
background trace or crash-dump facility. Model inputs stay in the gaze pipeline.
Starting the camera alone writes no trace. Every new recording requires consent.

A dedicated below-normal-priority thread captures and infers. The UI reads a
single latest numeric sample, so it cannot queue camera frames. Trace writing
happens for every acquired sample during a recording, independently of the UI
refresh rate. CPU time is capture/inference **thread CPU time**, not blocking
camera wall time. FPS is measured over the latest 60 acquisition timestamps.
The quality value is a heuristic, never a claim about detection accuracy.

## JSON-lines schema v1

One UTF-8 JSON object per line, in this order:

- `record: "header"`: `schema_version`, `app_version`, `started_unix_ms`, `camera`
  (`name`, negotiated `resolution: [width,height]`, requested/negotiated `fps`),
  `monitor_resolution`, `dpi_scale`, `lighting_note`, `script_seconds`, and `minimap`
  (`source`: `persisted_settings` or `default`, `minimap_scale`, `flipped`).
- `record: "frame"`: `timestamp_ns` since test start, `source_timestamp_ns` since
  camera-open, `features`, `raw_features`, `face_found`, `quality` (0..1), `cpu_ms`,
  `phase` and `target`.
- `record: "end"`: `frames` and `reason` (`completed`, `cancelled`, `tracking_error`).

Feature arrays have exactly ten finite numbers, in `trace::FEATURE_NAMES` order:
right eye horizontal, vertical, aperture; left eye horizontal, vertical, aperture;
head yaw, pitch, roll (radians), relative distance (inverse fit scale, not metres).
Right/left are anatomical; eye coordinates are in the unmirrored camera image.
`raw_features` is extraction before smoothing; `features` is the M4.1 One-Euro
output. Both are null for unknown/closed/occluded eyes, even when a face is found.
Unknown frames always have zero quality and are never silently counted as checks.

`target` is null outside focused `hold` phases. During a hold it contains a unique
trial `id`, normalized `position: [x,y]` (top-left origin), and `kind` (`minimap` or
`elsewhere`). Multiply by header monitor resolution for physical pixels. The
`countdown` and `unfocused` phases are unlabelled; ignore them for fixation fitting
and false-positive evaluation. No classifier decisions are recorded in M4.2.

**Timing:** acquisition timestamps, not inference completion, label frames. The
camera's Unix origin is mapped once to a monotonic clock. Each UI target
submission is timestamped, and a late frame uses the presentation preceding its
acquisition, not the latest target at inference completion. These are UI-submission
times, not measured display scanout; allow at least a display frame of uncertainty
and a human reaction/settling interval at target transitions during later fitting.
No claim is made that every frame immediately after a dot appears is fixation.
The header wall clock identifies the session; durations use monotonic time.
Minimizing/unfocusing is unlabelled and does not pause the script. On completion or
cancellation, in-flight samples not delivered before finalization are not stored.

The writer uses create-new filenames, checks finite values and increasing times,
flushes every 30 records, and writes/syncs an explicit end record on orderly stop.
The reader rejects unsupported schemas, extra fields (including pixel payloads),
nonfinite data, invalid targets, backwards/duplicate times, oversized lines,
excessive frame counts, truncated recordings and data after the end record. It
accepts cancelled/error traces and reports that status. Limits: 16 KiB per JSON
line, 120,000 frames (one hour at 30 fps plus headroom), and a 2 KiB note.

## Script and replay

`Script::at` is pure: 2 seconds of countdown then 4 seconds of hold per trial,
alternating elsewhere/minimap, 600 seconds by default. Elsewhere positions rotate
through centre, corners/edges and near-map negatives, excluding positions inside
or within 4% of screen height of the minimap. Geometry uses the main monitor's
physical dimensions and the resolved `MinimapScale` and `FlipMiniMap` from
`PersistedSettings.json`, without calibration. Discovery uses `lucas-game`'s
Windows installation locator and reader; only a missing file permits
`MinimapSettings::default()`. Read, parse and missing-field errors block recording.
The trace header records the source and resolved values, not the local path.
Durations are 60–600 seconds in multiples of six. Shorter tests retain alternation
but may not visit every calibration position.

```sh
cargo test -p lucas-app
cargo xtask replay /path/to/glance.jsonl
```

Replay validates the file and consumes all numeric samples through
`TraceGazeSource`. Valid-frame share and observed fps cover the entire recording;
per-kind mean, population standard deviation, min and max include only valid,
focused hold frames, using the smoothed features. An empty group is explicitly
reported. This is **not** the detection-rate evaluation; M5.1 will fit and validate
using the raw/filtered inputs and target ground truth.

## Windows bundle

CI builds on GitHub-hosted Windows/MSVC, copies pinned models and native DLLs,
adds app-local VC runtimes and notices, zips the bundle, then runs
`--check-runtime` from the extracted ZIP with a system-only PATH. ONNX Runtime
1.31 is statically linked by ort rc.14; its DirectML DLL is still included even
though LUCAS explicitly selects only the CPU provider. Font and binary runtime
licenses remain their upstream licenses; LUCAS source remains MIT OR Apache-2.0.

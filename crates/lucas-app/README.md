# Post-game summary window

`lucas_app::postgame` remains usable independently of the tray and gaze debug
window. The [gaze runtime](src/runtime/README.md) now opens it after game end,
never as an in-game score.

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
An already-running UI should embed `PostGameCard::show(ui)` instead. The tray
runtime hosts the card in a subprocess and owns game-end routing, in-game
suppression and pause-aware event clock mapping. Follow the storage reference's
[deletion prerequisites](../lucas-store/README.md#privacy-controls) first:
ownership transfers into the card so **Delete all data** can close SQLite before
deleting files. Deletion requires a second, explicit confirmation.

- In the standalone viewer or an embedded card, success clears the in-memory card.
  If deleting fails, keep the card open, close other LUCAS windows, and choose
  **Delete all data** again. The card can retry even if the database or summary
  has already been removed.
- In the tray-opened card, confirmation closes the card and requests tray shutdown.
  The tray releases its workers, children and cached state before deleting files.
  Close independently launched diagnostics/viewers first as required by the
  storage contract above. Restart LUCAS to begin with fresh settings and calibration.

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
Calibration/classification is described below. The tray starts gaze coaching
with no arguments; `--timer` is the explicit no-gaze option. Use `lucas --calibrate`
or `lucas --seconds 600` for the standalone shared gaze window.

## Data boundary

The UI and recorder receive only `GazeSample`, derived feature arrays and numeric
telemetry. RGB never crosses that UI/recorder boundary; `lucas-gaze` clears its
owned frame buffers after inference. UniGaze receives the current RGB frame only
in its LUCAS-owned local process, through anonymous pipes. There is no screenshot, video, image serializer, upload,
background trace or crash-dump facility. Model inputs stay in the gaze pipeline.
Starting the camera alone writes no trace. Every new recording requires consent.

A below-normal-priority thread captures into a single replaceable frame slot;
a separate worker infers the latest frame with the selected gaze source. The UI reads a
single latest numeric sample, so it cannot queue camera frames. Trace writing
happens on the inference/recording worker, independently of the UI refresh rate. Samples
acquired before the first target presentation or outside the script duration
are not written. CPU time is the Rust **inference/recording thread CPU time**,
not camera wall time; it excludes the separate capture thread and UniGaze process/GPU work. Use
`--check-gaze-model` for full-pipeline/model timing. FPS is measured over the latest 60 acquisition timestamps.
The quality value is a heuristic, never a claim about detection accuracy.

## JSON-lines schema v2 (v1 remains readable)

One UTF-8 JSON object per line: a header, frame records, then an end record.
The authoritative field names, types, enum spellings and validation limits are
the serde types and validators in [`trace.rs`](src/trace.rs), with target/phase
types in [`script.rs`](src/script.rs) and minimap metadata in
[`settings.rs`](src/settings.rs). Use those definitions rather than a copied
schema. `timestamp_ns` is relative to test start; `source_timestamp_ns` preserves
the camera-open origin.

Each v2 frame carries `feature_kind`: `media_pipe` or `uni_gaze`. Legacy v1
frames default to MediaPipe; mixing sources in one trace is rejected. Arrays
follow `trace::feature_names(kind)` in [`trace.rs`](src/trace.rs). MediaPipe eye
ratios/relative distance retain their original units; UniGaze uses camera-space
pitch/yaw in radians, apertures, normalized head pose, and face centre in mm.
Right/left eyes are anatomical; images are unmirrored. `raw_features` precedes
smoothing; `features` is One-Euro output (only the two angles for UniGaze). Both are null for unknown/closed/occluded eyes, even when a face is found.
Unknown frames always have zero quality and are never silently counted as checks.

`target` is null outside focused `hold` phases. During a hold it contains a unique
trial `id`, normalized `position: [x,y]` (top-left origin), and `kind` (`minimap` or
`elsewhere`). Multiply by header monitor resolution for physical pixels. The
`countdown` and `unfocused` phases are unlabelled; ignore them for fixation fitting
and false-positive evaluation. No classifier decisions are recorded in M4.2.

**Timing:** acquisition timestamps, not inference completion, label frames. The
camera supplies monotonic acquisition times under the
[`lucas-gaze` timing contract](../lucas-gaze/README.md#acquisition-timing).
Each UI target submission is timestamped, and a late frame uses the presentation
preceding its acquisition, not the latest target at inference completion. These are UI-submission
times, not measured display scanout; allow at least a display frame of uncertainty
and a human reaction/settling interval at target transitions during later fitting.
No claim is made that every frame immediately after a dot appears is fixation.
The header wall clock identifies the session; durations use monotonic time.
Minimizing/unfocusing is unlabelled and does not pause the script. On completion or
cancellation, in-flight samples not delivered before finalization are not stored.

The recording worker uses create-new filenames and syncs the file after the trace
writer flushes an explicit end record on orderly stop. The writer checks finite
values and increasing times, and periodically flushes frames.
The reader rejects unsupported schemas, extra fields (including pixel payloads),
nonfinite data, invalid targets, backwards/duplicate times, oversized lines,
excessive frame counts, truncated recordings and data after the end record. It
accepts cancelled/error traces and reports that status. See the constants and
validators in [`trace.rs`](src/trace.rs) for the enforced size limits.

## Script and replay

`Script::at` is pure; [`script.rs`](src/script.rs) owns the schedule constants,
duration validation and target construction. See the
[recording guide](../../docs/testing-on-windows.md#record-the-test-about-10-minutes)
for session timing, settings discovery/fallback and target geometry. Shorter tests
retain alternation but may not visit every calibration position.

```sh
cargo test -p lucas-app
cargo xtask replay /path/to/glance.jsonl
```

Replay validates the file and consumes all numeric samples through
`TraceGazeSource`. Valid-frame share and observed fps cover the entire recording;
per-kind mean, population standard deviation, min and max include only valid,
focused hold frames, using the smoothed features. An empty group is explicitly
reported. This is **not** the detection-rate evaluation; use `cargo xtask eval`
for fitted classification and held-out G2 scoring (see [xtask](../../xtask/README.md#g2-calibration-and-glance-evaluation)).

## In-app calibration

Open `lucas --calibrate`, connect your camera, pick **Screen size (diagonal)**
(24 in, 27 in, 32 in or 34 in), check the assumed eye-to-screen distance, then
**Start calibration**. Dimensions are calculated from the main calibration
monitor's resolution/aspect ratio, including ultrawide, and rounded to whole mm:
27″ at 16:9 gives 598×336 mm. Choose **Custom** to enter measured width and height
instead (for example, 597×336 mm). Windows DPI is not physical size.

Screen size, custom dimensions and viewing distance are remembered locally when
full calibration starts, even if it later fails or is cancelled. Standalone,
setup and tray calibration all reopen with those choices; finishing setup or
changing unrelated settings (such as the tray level or Live Client gate) does
not reset them. New users and older settings without this preference start at
24″ and 650 mm (531×299 mm at 16:9). These dimensions affect reported degrees,
not the minimap decision or pass thresholds.
Use your usual playing position and glasses. The same main-monitor fullscreen
window/amber dot renderer as the glance test presents a 46-second sequence:

- Nine grid points in a comfortable position.
- From 18 seconds, five dense map points interleaved with centre/ability bar/
  scoreboard/lane-above/lane-beside negatives. Shift naturally through this block:
  lean, turn and move a little closer or farther while following the dots.
- Four independent validation occurrences: two map looks and points just above
  and beside the map.

Saved calibration identifies both the source and the rectangular decision.
Older half-plane fits must be repeated, including when continuing with MediaPipe.
Repeat full calibration after changing source, camera position, lighting or map
geometry; a MediaPipe fit cannot be reused with UniGaze.

The map comes from `lucas-geometry` using the League settings and the saved
alignment profile for the main-monitor resolution, MinimapScale and flip. See the
[recording guide](../../docs/testing-on-windows.md#record-the-test-about-10-minutes)
for settings discovery, fallback and installation overrides, and
[alignment instructions](../../docs/testing-on-windows.md#one-time-alignment-repeat-whenever-a-patch-moves-the-hud)
for saving corrections. A missing profile uses the geometry seed; an invalid
alignment file blocks calibration. The corrected rectangle determines target
placement, labels and saved-model geometry comparisons. Changing that rectangle
requires a passing full calibration again before that model can be used.

Each two-second calibration target is visible through a 500 ms settling phase
and the following hold. Collection requires at least 200 ms of hold time and
700 ms of continuous target visibility, including after focus returns or a
delayed UI submission. Target labels are matched to capture acquisition times,
not UI receipt. Losing focus leaves frames unlabelled and resets visibility
settling; it does not pause the schedule. Missing target coverage fails
calibration. Escape cancels without replacing previous weights.

Tracking quality is visible, and sampled mean luma below 0.15 asks for a light
**in front** of the player. Luma is computed in RAM; no image is stored. A result
shows mean angular error, sensitivity/FPR and pass/fail. In standalone or tray
calibration, only passing fits save weights/geometry/validation aggregates through
`lucas-store` and update the saved camera selection; failures retain the previous
saved profile on disk. The tray automatically uses a passing fit matching the
current geometry; optional testing never revokes it. See the
[runtime contract](src/runtime/README.md#before-play). First-run setup instead uses the draft and
Finish contract in the same runtime guide.
Full calibration samples are discarded by default. For the separate opt-in
export, its contents and privacy controls, see the
[one-run diagnostic instructions](../../docs/testing-on-windows.md#one-run-numeric-calibration-diagnostics-explicit-opt-in).
The separate glance recorder still requires explicit opt-in for each recording.

**Test gaze** is optional: one minimap and one centre target in four seconds.
It neither refits nor saves and must use the saved calibrated camera. A failed or
cancelled test never revokes a passing matching fit; its feedback can suggest
recalibration if tracking seems wrong. For integrations:

1. Construct `CalibrationScript::new` with the current settings and loaded
   `AlignmentStore` as above. Pass its `screen().minimap` to
   `calibration::load(store, current_screen, current_minimap)`, which rejects
   changed desktop origin/resolution/DPI/map geometry or unsupported model bytes.
2. Use a passing matching model with `lucas_gaze::decision::CalibratedDetector`
   automatically, subject to ordinary tracking/foreground/game eligibility.
3. Only when the user requests a test, present `CalibrationScript::quick(&model)`
   and collect `Observation`s by displayed target and acquisition time.
4. `quick_revalidate(&model, observations)` and
   `validation.passes(model.target_false_positive_rate())` determine the optional
   test result, not coaching permission. Do not replace or revoke the saved fit.

The [runtime guide](src/runtime/README.md#before-play) owns game-start routing,
calibration snapshot matching and failure handling for the tray integration.

## Windows bundle

CI builds on GitHub-hosted Windows/MSVC, copies pinned models and native DLLs,
adds app-local VC runtimes and notices, zips the bundle, then runs
`--check-runtime` from the extracted ZIP with a system-only PATH. ONNX Runtime
1.31 is statically linked by ort rc.14; its DirectML DLL is still included even
though the MediaPipe fallback selects only the CPU provider. The optional
UniGaze installer/scripts are packaged through an explicit allowlist, never
installed weights, Python environments or caches. See
[UniGaze setup](../../models/unigaze/README.md) for CUDA and diagnostic details. Font and binary runtime
licenses remain their upstream licenses; LUCAS source remains MIT OR Apache-2.0.

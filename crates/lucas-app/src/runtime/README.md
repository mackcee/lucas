# Gaze coaching runtime

`lucas` starts gaze coaching in the Windows notification area. `--timer` (or
**Timer mode (no gaze)** in the tray) explicitly selects the fixed ten-second
fallback. The stored training level and density cap drive gaze coaching;
pulse appearance and optional sound apply in both modes. For the user controls,
see [Getting started](../../../../README.md#getting-started).

## Before play

- Until `setup_completed` is true in settings, normal launch opens a six-step
  first-run wizard: privacy, camera/quality, skippable Borderless check, the existing
  calibration UI, next-game alignment instructions, and starting level.
  **First-run setup…** reopens it with current settings. Close discards the in-memory
  draft, except for the [remembered display preferences](../../README.md#in-app-calibration).
  Finish commits the remaining settings, calibration and the completion flag. Draft
  camera choices/weights cannot replace the live pair before Finish. Finish is not
  a multi-file transaction: it removes the old calibration, saves settings with
  completion false, saves the new calibration, then marks completion true. A failed
  Finish can leave partial disk changes; retry while the draft window is open.
  Ordinary settings saves do not complete setup. Explicit `--timer` or the tray's
  timer mode bypasses setup for no-camera diagnostics, but an open settings/setup
  window still pauses coaching.
  Switching to gaze invokes setup when required.
- **Calibrate once, then play.** A passing saved fit matching the live screen and
  minimap enables coaching automatically in every game, including after restart
  or reconnection. There is no required QuickCheck state or per-game approval.
  Foreground, tracking and normal game eligibility still gate pulses.
- **Test gaze (optional, 4 seconds)** presents one minimap and one centre target.
  It never refits or saves. Its weights/camera fingerprint must match the tray's
  snapshot, but pass, failure, cancellation, stale inputs or a window error only
  produce feedback: none can revoke a usable fit or authorize a mismatched one.
  A failed result may suggest recalibration if tracking seems wrong.
- Calibration, testing and setup release the runtime camera and pause coaching
  while their window is open. Temporal gaze evidence resets at the handoff so
  neither dwell nor reminder time bridges the interruption. Closing full
  calibration/setup re-assesses the actual saved profile, even on cancellation;
  a previous passing, matching fit remains usable. Only passing full fits save.
  After completed setup, no calibration/test window opens automatically.
- Screen origin, resolution, DPI and the aligned minimap rectangle must match
  the saved fit. Use Borderless on the calibration display (currently the main
  monitor). Changed alignment/scale/flip needs a matching calibration. The exact
  matching rules, 1e-9 minimap tolerance and validation thresholds are unchanged.
  Startup geometry is provisional; the first live geometry is assessed afresh.
  Subsequent real geometry changes are reassessed, including returning to the
  saved layout. Unchanged geometry does not repeatedly reload or toggle readiness.
- **Calibration needed** gives one plain reason only when no usable saved fit is
  available or geometry does not match. **Calibration details** holds read/model
  errors, failed validation, or changed geometry fields with saved/current values.
  Window/test outcomes are separate hints, not calibration failures. These are
  local menu text, not a log or upload. Private child exit codes remain 0 (pass),
  3 (cancel), 4 (failed validation/tracking), and 5 (changed inputs); no result
  weakens full-calibration acceptance, geometry matching or the 100 ms dwell.
- **No face / tracking lost** means pulses and eligible gap time are paused.
  Closed eyes, low quality, off-screen estimates and stale samples are unknown,
  never minimap checks. Add front lighting or reconnect the camera; switching
  timer mode on and off retries a camera that failed to open.

## Manual controls and solo games

- Level labels and delays come from `lucas-core::TRAINING_LEVELS`, excluding the
  compatibility-only Baseline level from both selectors. A choice is
  saved through `lucas-store` before changing the pending preference. The current
  match keeps its policy, standard and density-cap snapshot, including across a
  Live Client/window-only gate switch. There is no automatic level progression.
- **Next game solo (no pulses)** is one-shot, held in memory until the next game
  starts (not retained across process exit). Its tooltip announces both arming
  and the active solo game. No pulse, sound or alignment preview appears in that
  game; measurement and optional gaze testing still operate. Timer selection cannot
  re-enable assistance during a solo game. Its stored `probe_game` flag produces
  the existing card's **Solo probe** label. Arming during play selects the next
  match, not the current one. Gate switches cannot turn a probe into assisted play.
- **Settings…** exposes level, style, RGB colour and optional sound, plus data
  folder and confirmed deletion. Settings/setup run in one child process; the
  tray releases the camera and pauses coaching while it is open. Camera preview
  and calibration share the debug capture code but have no trace-recording UI.
  Settings saves are reloaded when the window closes; style/colour/sound apply
  immediately, level at the next match. Other settings writers are disabled while
  this child owns the draft. Deletion uses the existing parent-shutdown handshake;
  independently launched writers must still be closed as required by the
  [storage deletion contract](../../../lucas-store/README.md#privacy-controls).

## Threads, eligibility and privacy

`Runtime` is a portable loop tested without Win32. The tray/core ticks at
approximately 30 Hz, a dedicated inference worker uses the selected
[`ModelGazeSource`](../../../lucas-gaze/README.md) fed by `LatestCamera`, the API
worker polls at about 1 Hz, and the overlay retains its own foreground-gated
presentation thread. The runtime receives a single latest **numeric** sample
slot, not a frame queue. Camera timing follows the
[`lucas-gaze` acquisition contract](../../../lucas-gaze/README.md#acquisition-timing);
the worker expresses accepted acquisition times relative to the app's `Instant`
origin. Each frame is consumed once by
`CalibratedDetector` (which owns `GlanceDetector`); checks, completed dwell, gates
and policy pulses feed `MetricsAccumulator`. Metrics apply at the next tick
(normally within one tick of delivery). Replayed/future frames cannot complete
dwell; streams expire after 250 ms. Unserviced tray-menu intervals receive at
most that same bounded tracking credit.

Only alive, supported Summoner's Rift play after 1:30, League foreground,
validated geometry and usable gaze advance the clock. Foreground/tracking loss
freeze it; death/loading/end reset it. Sustained minimap viewing hides the pulse
and does not create repeated checks. A check restarts the reminder interval;
while looking away, a new reminder opportunity occurs every interval, including
after a skipped probabilistic reminder. Ramp 5 s therefore repeats around 5, 10,
15 s without a check, with independently sampled ±10% jitter per interval. Late
ticks never replay missed reminders in a burst. Baseline/Graduated and solo games
remain silent. The core policy, not a new app timer, supplies this schedule.

Ramp keeps its selected cadence even with the density cap enabled. The adaptive
+750 ms per pulse / −250 ms per unprompted check staircase now applies only in
Fade, where existing delays, probabilities and jitter still apply. Strong and
Medium strokes scale with minimap size and are brighter; their two ripple waves
fade more slowly within the same 1.2-second animation. Soft/Minimal, the transparent
map interior, click-through behaviour, style/colour selection and optional sound
are preserved. Each emitted repeat increments the summary's pulse count and is
stored as its own event; pulse-to-glance latency uses the most recent pulse.

If Settings says 5 s but a completed game's card says 10 s, check the game's
level snapshot: changing the level during that game only affects the next match.

Runtime capture requires an available calibration model and a running game in
gaze mode or a solo game. Without a model, the tray continues ticking with
**Calibration needed** and keeps runtime capture closed so recalibration remains
available. Explicit setup tracking checks, calibration and diagnostics open their
own capture independently of that requirement. Game end, API loss and exit
release runtime capture; timer mode releases it outside solo play. Settings pauses
capture; setup diagnostics and calibration wait for the runtime worker to release
the device before opening their own capture.
Synchronous driver acquisition must return before shutdown can join the thread;
a wedged camera driver remains a native hardware-test limitation.

API loss fails closed immediately. To avoid a focus-stealing false post-game
card on transient connection loss, a session finishes on authoritative GameEnd
or API loss **plus disappearance of the game window**. Reconnection before
finalization keeps the same session/death deduplication. If the window disappears
and the session finishes, returning to the same active match starts a new session
using the matching saved calibration automatically, while retaining death deduplication.
The explicit **Use Live Client gate** tray switch selects window-only fallback;
there is no silent permissive fallback.
Window-only cannot know loading, death, map/mode or game-clock pauses: pause it
manually. Its summaries use session elapsed time and mark death data unavailable.

## End-of-game storage

Whole-game poll anchors map checks/pulses to game time and delayed deaths back
to observed monotonic history. Pauses have flat anchors; outages are not
interpolated. Checks after the latest poll clamp conservatively to that poll
(~one poll uncertainty). Deaths before observation or in unmappable outages are
unknown, not blind. Role is unknown because the subset API supplies no role;
champion, game length, level and valid/total frame quality are retained. Only
summaries/check/pulse/death timestamps go through `lucas-store`, never frames or
per-frame runtime features. Timer fallback pulses also count as assistance.

At game end the summary/events are saved, then a separate post-game process opens
the exact saved game. Gate-mode changes defer presentation until game end.
For deletion and restart behavior, see the
[card integration contract](../../README.md#integration-at-game-end).
Cards are suppressed/closed when another game starts. Calibration and post-game
children close on tray exit.

Portable replay coverage is in `tests/runtime.rs`: synthetic gaze plus scripted
API states, checks and jitter bounds, no-face/off-screen/closed-eye spans,
blind/seen/unknown deaths, SQLite round-trip, game-start calibration routing and
camera-source open/drop counts. `tests/calibration_readiness.rs` also executes
setup persistence → multiple games/restart → active coaching without a test,
non-revoking optional outcomes, and genuine mismatch/recovery through the same
loader/completion path used by the desktop. Synthetic fixtures are not real-webcam accuracy
or native Windows UI acceptance evidence.

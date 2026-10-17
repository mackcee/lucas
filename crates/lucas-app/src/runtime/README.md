# Gaze coaching runtime

`lucas` starts gaze coaching in the Windows notification area. `--timer` (or
**Timer mode (no gaze)** in the tray) explicitly selects the fixed ten-second
fallback. The stored training level, density cap, pulse style/colour and optional
sound drive gaze coaching; no level selector or automatic promotion is added.

## Before play

- With no passing calibration, launch opens the existing calibration surface
  **only if League is not running**. Connect the camera and run full calibration.
- At every game start, choose **Quick check required (4 seconds)** in the tray.
  This deliberately uses a player-initiated window rather than stealing focus
  from League. Do it on the loading screen or before play; until it passes,
  coaching remains off. It presents one minimap and one centre target.
- A failed/cancelled quick check latches **Calibration needed / recalibrate**.
  Old weights stay on disk but cannot enable coaching. Complete a new full fit,
  then the game's quick check. There is no implicit recalibration.
- Screen origin, resolution, DPI and the aligned minimap rectangle must match
  the saved fit. Use Borderless on the calibration display (currently the main
  monitor). Changed alignment/scale/flip needs matching calibration again.
- **No face / tracking lost** means pulses and eligible gap time are paused.
  Closed eyes, low quality, off-screen estimates and stale samples are unknown,
  never minimap checks. Add front lighting or reconnect the camera; switching
  timer mode on and off retries a camera that failed to open.

## Threads, eligibility and privacy

`Runtime` is a portable loop tested without Win32. The tray/core ticks at
approximately 30 Hz, a dedicated below-normal-priority thread owns synchronous
camera acquisition and CPU inference, the API worker polls at about 1 Hz, and the
overlay retains its own foreground-gated presentation thread. The camera has a
single latest **numeric** sample slot, not a frame queue. Acquisition timestamps
are mapped once to the app's `Instant` epoch. Each frame is consumed once by
`CalibratedDetector` (which owns `GlanceDetector`); checks, completed dwell, gates
and policy pulses feed `MetricsAccumulator`. Metrics apply at the next tick
(normally within one tick of delivery). Replayed/future frames cannot complete
dwell; streams expire after 250 ms. Unserviced tray-menu intervals receive at
most that same bounded tracking credit.

Only alive, supported Summoner's Rift play after 1:30, League foreground,
validated geometry and usable gaze advance the clock. Foreground/tracking loss
freeze it; death/loading/end reset it. Sustained minimap viewing hides the pulse
and does not create repeated checks. The core policy supplies level-dependent,
jittered delays and the density cap, not a new app timer.

The camera opens only for a running game in gaze mode or explicit calibration/
diagnostics. Game end, API loss, timer mode and exit release it; calibration
waits for the runtime capture thread to release it before opening its own device.
Synchronous driver acquisition must return before shutdown can join the thread;
a wedged camera driver remains a native hardware-test limitation.

API loss fails closed immediately. To avoid a focus-stealing false post-game
card on transient connection loss, a session finishes on authoritative GameEnd
or API loss **plus disappearance of the game window**. Reconnection keeps the
same session/death deduplication. The explicit **Use Live Client gate** tray
switch selects window-only fallback; there is no silent permissive fallback.
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
The card's deletion control requests tray shutdown; the tray releases workers,
children and cached state before deleting data. Restart LUCAS to begin fresh.
Cards are suppressed/closed when another game starts. Calibration and post-game
children close on tray exit.

Portable replay coverage is in `tests/runtime.rs`: synthetic gaze plus scripted
API states, checks and jitter bounds, no-face/off-screen/closed-eye spans,
blind/seen/unknown deaths, SQLite round-trip, game-start calibration routing and
camera-source open/drop counts. Synthetic fixtures are not real-webcam accuracy
or native Windows UI acceptance evidence.

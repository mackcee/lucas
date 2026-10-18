# Testing LUCAS on Windows

## G1 — timer mode and minimap alignment (about 15 minutes)

This test needs a Windows 10/11 PC, a real Summoner's Rift game and League in
**Borderless** mode. No webcam, models or gaze calibration are needed. Use a
non-critical game for alignment: while editing, LUCAS temporarily reserves the
arrow, Shift+arrow, Enter and Esc keys. It does not send input to League.

### Start and gate checks

1. Launch `lucas.exe --timer` for this no-camera test. It starts in the notification
   area with an amber square-ring icon and no application window. Check the hidden
   tray icons if needed. Starting it again should not create a second icon.
2. Right-click the icon. **Use Live Client gate (recommended)** is checked by
   default. The status/tooltip should say waiting until League is foreground,
   the player is alive in a supported Summoner's Rift game, and game time is
   **past 1:30**. Once eligible, the first pulse is after ten more seconds.
3. Confirm pulses around the minimap every **10 seconds**, using the saved
   [pulse appearance and sound choices](../README.md#getting-started).
   The ring must never take focus or obstruct minimap clicks.
   Check normal movement/clicking through the ring, not just its transparent centre.
4. Check loading, death, respawn, alt-tab, minimizing and game end. There should
   be no pulse while ineligible; a visible pulse should disappear on alt-tab.
   After respawn/return, expect a fresh ten-second interval, not a catch-up pulse.
5. **Pause pulses** hides the ring; **Resume pulses** starts a fresh interval.
   Unchecking the Live Client gate explicitly selects **window-only** fallback.
   This only knows whether the game window is foreground: it **cannot suppress
   death, loading, other maps or the end screen while that window remains**.
   Pause manually in those situations. Re-check the gate for normal G1 tests.
   API errors never silently enable window-only mode.

If the status says HUD settings are unavailable, set the `LUCAS_LEAGUE_ROOT`
environment variable to the installation folder containing `Config`, then
restart LUCAS. Automatic discovery tries Riot's local install manifest and
common install folders. Settings are read every second, preferring
`Config/PersistedSettings.json` with per-field `Config/game.cfg` fallback.
LUCAS never modifies League's files. Exclusive fullscreen is unsupported.

### One-time alignment (repeat whenever a patch moves the HUD)

1. With an eligible game running, choose **Align minimap...**, then return to
   League yourself. LUCAS never brings League or the ring to the foreground.
   Timer pulses pause while editing; a steady amber border marks the target.
2. **Arrows** nudge by one physical pixel. **Shift+Right/Up** grows the square
   one pixel; **Shift+Left/Down** shrinks it. Resizing keeps the bottom and outer
   corner anchored. Align the inner edge of the border just outside the map.
3. **Enter** saves; **Esc** cancels without changing the saved profile. If saving
   fails, the tray menu explains the error; retry Enter or cancel. The ring hides
   and its hotkeys release when League loses foreground. Returning resumes the
   draft. Changing resolution, scale, flip or game window cancels the draft.
4. Verify that alignment survives quitting/restarting LUCAS. Corrections live in
   `%LOCALAPPDATA%\lucas\alignment.toml`, separately for each **client resolution,
   exact MinimapScale value and flip**. Moving the game to another monitor does
   not change that profile. Edit another profile and confirm the first survives.
   A malformed alignment file is not overwritten; move it aside to reset it.

### Capture the geometry numbers

For your normal client resolution, repeat with **MinimapScale 0, 1, 2 and 3**,
**unflipped and flipped** (eight combinations). These are stored settings values,
not the in-game slider percentage. Confirm the actual value in each JSON report.
Do not change settings during an alignment draft. Wait at least one second after
changing League's settings so LUCAS can re-read them.

1. Align and save each profile. Use **Save minimap numbers** from the tray.
2. **Open data folder**, then `reports`. Each `minimap-*.json` contains the game
   client rectangle, resolution, MinimapScale, flip, seed/formula rectangle,
   saved/aligned rectangle, coordinate-space description and app version.
   Reports use physical desktop pixels and may have negative origins on a
   secondary monitor. Edges are half-open, not x/y/width/height.
3. Take screenshots locally for your own comparison at each setting. LUCAS
   does not capture or store screenshots. Send back the eight JSON files plus
   brief notes on placement, Windows display scaling, focus/click-through and
   any missing gate transition. **Only measured numbers**, not League artwork,
   become repository geometry fixtures. A report before alignment is a formula
   estimate, not a ground-truth measurement; label it as such.
4. Choose **Quit**. Verify the tray icon and any alignment border disappear,
   and all alignment keys work normally again.

Record whether the Live Client gate actually became active. If the local API or
TLS certificate is unavailable, fallback is useful for placement/click-through,
but **does not pass** the alive/dead/loading portion of G1.

## G3 — three-game MVP check

Use the [Windows artifact download steps](#download-and-run), a connected webcam,
League **Borderless**, and ordinary supported Summoner's Rift games. Keep
**Use Live Client gate** on and **Timer mode** off. Practice Tool is deliberately
not eligible for coaching. This is real-device acceptance; portable tests and
synthetic UI previews do not establish gaze accuracy, camera-driver behavior or
native overlay correctness.

### Before the three games

- Launch `lucas.exe` normally. Check the [setup routing rules](../crates/lucas-app/src/runtime/README.md#before-play),
  including upgrades with settings but no completed setup. If setup was already
  completed, choose **First-run setup…** to repeat it. Check the privacy note,
  select your camera and **Check tracking**. Confirm live face/eye status, fps
  and the quality indicator respond; stopping or closing releases the camera.
- On an installation where setup has never completed, close halfway through setup,
  then select **Timer mode (no gaze)** from the tray: eligible timer reminders
  should work without calibration. While setup/settings
  is open, reminders must remain suppressed even in timer mode. Change a level or
  the Live Client gate in timer mode, then restart normally: setup must still open.
  Reopening setup starts at the first step with current saved preferences, not the
  discarded draft. Try **Skip check for now** at Borderless, then complete calibration.
  Failed/cancelled calibration must not let you continue. Choose **10 s (start)**
  and Finish; the window should close to the tray. Restart once and confirm the
  wizard does not repeat. `--timer` remains an explicit setup-free G1 diagnostic.
- In **Settings…**, try each pulse style and a distinct colour. Save/reopen to
  verify persistence. Sound starts off; leave it off unless intentionally testing
  it. Settings should open only when requested, not steal focus automatically
  during play. Keep automatic promotion/probe scheduling expectations out of this
  test: neither is implemented.

### Game 1 — assisted start

- Use **Align minimap…** once eligible, return to League, nudge/resize and save.
  If this changes the calibration zone, explicitly recalibrate; then complete
  **Quick check required (4 seconds)** from the tray before continuing play.
- Check pulses after eligible gaps, no pulse during a sustained map look, and
  suppression when dead, alt-tabbed, tracking lost or outside supported play.
  Delays have jitter and a density cap, so they are not exact metronome intervals.
  Confirm click-through and no focus stealing. Note tracking quality and comfort.
- Mid-game select **8 s**. The submenu check should change, but this game's goal
  and policy must stay at 10 s. At game end, the card should retain the old level
  and show checks, tracking quality and any known/unknown blind deaths.

### Game 2 — the selected level

- Confirm **8 s** survived restart if you restarted. Do the game's quick check.
  This game should use the newly selected level; the end card should agree.
- Try a different pulse style/colour and optional sound through Settings, then
  restore sound off. Saving appearance must not change the game goal. Closing
  Settings resumes coaching, without opening another calibration automatically.
- After play, arm **Next game solo (no pulses)**. Confirm the menu is checked and
  the tray tooltip announces the upcoming solo game. Disarm/rearm once to check
  both paths. Do not quit LUCAS after arming: the one-shot is session-local.

### Game 3 — solo

- At game start, the tray tooltip should say **Solo game — no pulses or sound;
  summary after play**. Do the usual quick check. No minimap outline, reminder,
  sound or live score should appear, even over long gaps. **Align minimap…** is
  unavailable during solo play. Timer selection must not bypass this suppression.
- At game end, the card should say **Solo probe**, with zero pulses but measured
  checks/gaps/tracking and death classifications where evidence exists. Unknown
  tracking must not inflate blind deaths. The one-shot should no longer be armed;
  the next game returns to the selected manual level.

### Evidence and cleanup

Record the build commit, Windows/display details, camera/lighting, selected levels,
any missed/false checks, overlay/focus problems, and all three card results. Use
**Open data folder** and the [storage reference](../crates/lucas-store/README.md#files-and-schema)
to locate saved results. Quit before copying the SQLite database. The app stores
no webcam images; share only what you choose.

After retaining any desired evidence, test **Settings… → Delete all data…**:
first cancel and confirm data remains, then confirm deletion if you intend to
remove it. Follow the [deletion prerequisites and scope](../crates/lucas-store/README.md#privacy-controls);
the tray must shut down before removing the managed data. Relaunch should start
fresh setup. This is irreversible; do not delete the three-game results before
checking them.

## G2 — Windows webcam test

This session uses the gaze debug window and opt-in glance recorder, not timer
mode or gaze-driven coaching. Use Windows 10/11 x64 and an AVX2-capable CPU.
No Rust, Python, model downloads or developer tools are needed on the test PC.

### Download and run

1. Open [GitHub Actions](https://github.com/mackcee/lucas/actions/workflows/ci.yml),
   choose the successful run for the intended commit on **main** (or the test PR),
   and download **lucas-windows-x64** under Artifacts. You may need to sign in.
   Builds are available on both main pushes and PRs, retained for 14 days.
2. Extract the artifact download, then extract the inner `lucas-windows-x64.zip`.
   Keep `lucas.exe`, **all DLLs**, `models/` and `licenses/` together. Do not run
   the executable from inside a ZIP.
3. From a terminal in the extracted folder, run `.\lucas.exe --seconds 600` to
   open gaze diagnostics rather than the default [coaching mode](../README.md#status).
   Builds are unsigned:
   after confirming you downloaded the intended repository/commit, SmartScreen
   may require **More info → Run anyway**.
4. Select your Logitech C270 and click **Connect camera**. Close Discord/OBS or
   other apps using the camera if it cannot open. Allow desktop camera access in
   Windows Settings → Privacy (or Privacy & security) → Camera.

The requested format is 1280×720 MJPEG at 30 fps. The debug view shows **Face
found**, whether the eyes are usable, raw eye/head features, observed fps, capture /
inference thread CPU time per frame, and heuristic tracking quality. The marker
uses camera-space iris ratios: it is **not calibrated**, not a screen position,
and not a minimap detection. No camera preview is shown or stored. Low quality,
unknown eyes, or low fps are reasons to adjust lighting and placement first.

### Record the test (about 10 minutes)

- Sit at your normal playing distance. Leave League closed for this test; it
  displays its own targets on the **Windows main monitor**. Disable notifications
  and keep the test focused. It uses borderless fullscreen, not exclusive mode.
- Enter a lighting and placement note (for example, “daylight, ceiling light on,
  top-centre C270, glasses”). Avoid personal details you do not want to share.
- Leave duration at **600 seconds** for 100 trials: 50 minimap targets alternating
  with 50 elsewhere targets. Each trial has a 2-second centre countdown and a
  4-second target hold. Centre, corners and edges provide calibration inputs;
  positions inside or close to the minimap are excluded from elsewhere targets.
  For a quick equipment check choose **60 seconds**; durations must be multiples
  of 6, from 60 to 600 seconds. A short test is not the full evaluation session.
- Check **I agree to save a numeric trace for this test**, then **Start fullscreen
  glance test**. Look at the countdown; when the dot appears, look at its centre
  and keep looking there until it disappears. Keep your head comfortable and
  mostly still. **Esc** ends early and saves a trace marked cancelled.
- The minimap target uses `lucas-geometry` at the actual main-monitor resolution.
  Before each test, LUCAS reads `MinimapScale` and `FlipMiniMap` from the discovered
  League installation's `Config/PersistedSettings.json`. For a custom installation,
  set the installation override described under [Start and gate checks](#start-and-gate-checks)
  before launching diagnostics. Only when no file is found does it use scale 1
  and bottom-right placement. An unreadable, malformed, or incomplete settings
  file blocks recording rather than silently defaulting.
  The trace header records the source and values, without the installation path.
  Elsewhere targets stay outside the minimap plus a margin of 4% of screen height.
  This is a geometry seed, not an in-game alignment measurement.
- At completion, click **Open traces folder**. Files are at
  `%LOCALAPPDATA%\lucas\traces\glance-<timestamp>-<pid>.jsonl`.
  **The file contains no images**, only numeric features/timestamps/targets and
  the small metadata header (including your note). No trace is made until you
  opt in. Nothing is uploaded; explicitly send the chosen file back yourself.
  Delete local traces in Explorer whenever you want.

Repeat once at night because lighting varies. If offline evaluation misses the
G2 criterion (at least 90% of deliberate minimap glances detected, at most one
false detection per minute of not looking), try the C270 near the minimap corner
and repeat, identifying the new position in the note. Run `cargo xtask eval
<path-to-trace.jsonl>` for detection-rate evaluation; see the
[G2 evaluator](../xtask/README.md#g2-calibration-and-glance-evaluation) for separate
calibration recordings, physical monitor dimensions and scoring rules. Synthetic
tests do not establish real-webcam G2 acceptance.

For in-app calibration and **Quick check**, follow the
[calibration guide](../crates/lucas-app/README.md#in-app-calibration), including
physical monitor measurements, saved alignment and when to recalibrate.

### Troubleshooting / development

- Missing model or hash error: re-extract the complete matching bundle. The app
  never downloads replacement models. Do not change `SHA256SUMS` to bypass it.
- Camera stopped / trace save error: check camera permissions, close other camera
  apps, and check disk space. Reconnect and start a new recording. A crash or
  forced shutdown can leave an incomplete trace; replay reports that explicitly.
- No usable acquisition timestamps: reconnect the webcam; if the error persists,
  update its driver. This is a timing failure, not a lighting/eye-quality failure.
  Start a new recording after reconnecting. See the
  [camera timing contract](../crates/lucas-gaze/README.md#acquisition-timing)
  for skipped frames and the timeout policy.
- Alt-tab/minimizing marks samples unlabelled while unfocused; the clock continues.
  Countdown and unfocused frames are not ground-truth fixations. Re-run if you
  were distracted. Stop/cancel uses an explicit end record; the UI reports the
  saved path only after a successful flush.
- From a source checkout: `cargo run -p lucas-app -- --models models`.
  `lucas.exe --seconds 60` opens the same setup; consent is still
  required. `lucas.exe --check-runtime` checks packaged models/runtime without
  opening the camera. `--version` and `--help` do not open a window.
- Offline, on Windows or Linux: `cargo xtask replay path/to/trace.jsonl` prints
  frames, valid-frame share, observed fps and per-target-kind feature distributions.
  See [the trace format](https://github.com/mackcee/lucas/blob/main/crates/lucas-app/README.md)
  for timestamp and label semantics (select the tested commit when using a PR build).

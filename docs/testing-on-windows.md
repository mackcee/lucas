# Testing LUCAS on Windows

## G1 — timer mode and minimap alignment (about 15 minutes)

This test needs a Windows 10/11 PC, a real Summoner's Rift game and League in
**Borderless** mode. No webcam, models or gaze calibration are needed. Use a
non-critical game for alignment: a small, focused LUCAS panel receives the
arrow, Shift+arrow, Enter and Esc keys. It does not send input to League or
register global alignment hotkeys.

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

**Practice Tool is not eligible under the Live Client gate.** To align or check
the ring there, uncheck **Use Live Client gate (recommended)** to select
**window-only** mode. This works with `--timer` and does not require calibration;
it does not validate the Live Client alive/dead/loading gates.

If the status says HUD settings are unavailable, set the `LUCAS_LEAGUE_ROOT`
environment variable to the installation folder containing `Config`, then
restart LUCAS. Automatic discovery tries Riot's local install manifest and
common install folders. Settings are read every second, preferring
`Config/PersistedSettings.json` with per-field `Config/game.cfg` fallback.
LUCAS never modifies League's files. Exclusive fullscreen is unsupported.

### One-time alignment (repeat whenever a patch moves the HUD)

1. With an eligible game running, choose **Align minimap...**. A small **LUCAS —
   Align minimap** panel should open with focus, on League's monitor and away
   from the minimap. **Keep this panel focused** while adjusting; do not return
   to League to use the keys. LUCAS never brings League to the foreground.
   Timer pulses pause while editing; a steady amber border stays over the map.
2. **Arrows** nudge by one physical pixel. **Shift+Right/Up** grows the square
   one pixel; **Shift+Left/Down** shrinks it. Resizing keeps the bottom and outer
   corner anchored. Align the inner edge of the border just outside the map.
   Confirm League's camera does **not** pan while using the panel's arrow keys.
   Also try the **Up/Left/Down/Right**, **Smaller/Larger**, **Save** and **Cancel**
   buttons without the keyboard. After clicking a nudge button, arrow shortcuts
   should still work; Tab/Shift+Tab selects buttons and Space activates them.
3. **Enter / Save** saves; **Esc / Cancel / closing the panel** cancels without
   changing the saved profile. The panel and border close together. If saving
   fails, the tray menu explains the error; retry Save or cancel in the panel.
   While the panel **or League** is foreground, the border remains visible if
   the game is otherwise eligible. Alt-tab to an unrelated app: the border must
   hide. Returning to the panel resumes the same draft; minimizing League hides
   the border too. Changing resolution, scale, flip or game window cancels the
   draft and closes the panel. Open alignment on another monitor too: the panel
   must not obscure the minimap and the border must track the correct monitor.
4. Verify that alignment survives quitting/restarting LUCAS. Corrections live in
   `%LOCALAPPDATA%\lucas\alignment.toml`, separately for each **client resolution,
   exact MinimapScale value and flip**. Moving the game to another monitor does
   not change that profile. Edit another profile and confirm the first survives.
   A malformed alignment file is not overwritten; move it aside to reset it.
5. At **3840×2160, MinimapScale 2.1, unflipped**, check right/down nudges can
   reach the client edge (including the final pixel of the fractional seed
   inset). Save the measured correction and a numeric report; do not treat the
   predicted rectangle as a fitted measurement.

Windows-only interactive evidence for this panel is deferred to the next G1
session. `windows-latest` CI builds and tests the code, but cannot establish
focus/key delivery over a running League game.

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
4. Choose **Quit**. Verify the tray icon, alignment panel and border disappear.
   Alignment keys must never be reserved globally, during or after editing.

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

- Use **Align minimap…** once eligible; keep the LUCAS panel focused to nudge/resize
  and save while watching the border over League.
  If this changes the calibration zone, explicitly recalibrate; then complete
  **Quick check required (4 seconds)** from the tray before continuing play.
- If the tray says **Calibration needed**, open **Calibration details** for the
  reason. Geometry differences list saved/current numbers; a cancelled, failed or
  stale quick check is identified separately. A passing full calibration only
  advances to **Quick check**, not active coaching. While its window is open the
  tray should say **Calibration / quick check running — coaching paused**.
  If only provisional startup geometry rejected the profile, the first matching
  live game geometry may recover to Quick check once. Cancelling a full fit or
  failing a quick check must still require full calibration; changing geometry
  back must not silently re-enable coaching.
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
The MediaPipe fallback needs no developer tools. The stronger UniGaze tracker
requires the one-time CUDA setup below; it installs its own private Python runtime.

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

### One-time UniGaze setup (NVIDIA CUDA)

Use a current NVIDIA driver (RTX 5090/Blackwell requires the included CUDA 12.8
stack). Leave League and other camera apps closed. In PowerShell, from the
extracted bundle directory, run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\models\unigaze\setup.ps1
if ($LASTEXITCODE -ne 0) { throw 'UniGaze setup failed' }
powershell -NoProfile -ExecutionPolicy Bypass -File .\models\unigaze\check.ps1
if ($LASTEXITCODE -ne 0) { throw 'CUDA check failed; do not start calibration yet' }
.\lucas.exe --calibrate
```

Setup downloads hash-verified official source/weights and pinned Python/CUDA
dependencies into `models\unigaze` beside the app. Allow about 12 GB of free disk
space. No administrator rights, system Python or CUDA SDK are needed. Setup and
the model check never open the camera. Do not move the installed bundle or run
setup while LUCAS is open; rerun setup after moving it. Details, hashes, licenses
and developer reproduction steps: [UniGaze install](../models/unigaze/README.md).

UniGaze is selected automatically when installed; its source/device appears in
the tracking panel and tray status. If not installed, the app explicitly labels
MediaPipe fallback. An installed but unavailable CUDA/model stack reports the
failure: it does not silently run UniGaze on the CPU. To explicitly select the
original tracker for a new calibration, quit all LUCAS processes, then run:

```powershell
$env:LUCAS_GAZE_SOURCE = 'mediapipe'
.\lucas.exe --calibrate
# For subsequent tray play in the same PowerShell session:
.\lucas.exe
# Restore automatic selection before a new UniGaze calibration:
Remove-Item Env:LUCAS_GAZE_SOURCE -ErrorAction SilentlyContinue
```

`LUCAS_GAZE_SOURCE=unigaze` instead requires UniGaze and errors if missing. Keep
the same selection for calibration, quick check, recording and play. Follow the
[calibration guide](../crates/lucas-app/README.md#in-app-calibration) for saved-profile
compatibility and when to repeat calibration.

### Headless GPU model check (no camera)

`lucas.exe --check-gaze-model` loads the pinned models on **cuda:0**, warms the
whole SFD/FAN + UniGaze-H FP16 path on a bundled public face image, then performs
20 timed inferences (3 measured-path warmups excluded). It prints the execution
provider, GPU name, full-pipeline/model-only median and p95 latency, and CUDA
allocated/reserved/peak memory including startup. Startup is bounded to 120 s,
and the diagnostic to a further 60 s. Exit 0 means model execution succeeded;
nonzero prints the specific missing asset/hash/CUDA/child-process error. Use the
bundled `models\unigaze\check.ps1` helper above: it uses `Start-Process -Wait
-NoNewWindow -RedirectStandardOutput -RedirectStandardError`, prints both logs,
and returns the actual process exit code. A GUI-subsystem executable can otherwise
return early or show no output in interactive PowerShell, even with `| Out-Host`.
The logs remain under `models\unigaze\checks`. `--models <directory>` can select another model root;
do not combine this diagnostic with camera or window modes.

This is a runtime/latency check, **not an accuracy result**. See the
[target-PC results](../models/unigaze/README.md#target-pc-headless-verification-2026-10-20)
for reported webcam calibration evidence and its limits.

### Calibrate with natural movement

Adjust front lighting and camera placement, connect the camera, and wait for the
source/device and live tracking indicators. First startup can take up to two
minutes; model warmup completes before the camera opens. Enter your actual screen
dimensions (for the investigated 27-inch display: 597×336 mm, 3840×2160) and your
viewing distance (about 650 mm in that setup). Do not substitute Windows DPI.

Follow the [in-app calibration guide](../crates/lucas-app/README.md#in-app-calibration)
for target timing, natural movement, validation and saved-profile behavior. The
[calibration reference](../crates/lucas-gaze/README.md#per-user-calibration-m51)
explains the rectangular score and unchanged dwell/pass thresholds. Low mean
error and high usable-frame coverage do not prove that map/elsewhere samples
are separable; do not loosen thresholds to hide that failure.

### One-run numeric calibration diagnostics (explicit opt-in)

When tracking looks good but sensitivity is low, run one full calibration with
this opt-in. It saves **gaze/pose numbers, not images**: raw and filtered features,
acquisition/visibility/settling times, labels, geometry (including DPI/canvas),
per-target scores/errors, the selected threshold/effective rectangle, and fitted
weights. Failed/cancelled full calibrations are included. Nothing is uploaded.
Files can reveal where you looked; share only the file you choose, then delete it
when no longer needed. Normal calibration does **not** export these samples.

From the test bundle, close other LUCAS instances first:

```powershell
$oldDiagnostics = $env:LUCAS_CALIBRATION_DIAGNOSTICS
$oldSource = $env:LUCAS_GAZE_SOURCE
try {
    $env:LUCAS_CALIBRATION_DIAGNOSTICS = Join-Path $PWD 'calibration-diagnostics'
    $env:LUCAS_GAZE_SOURCE = 'unigaze'
    Start-Process -FilePath (Join-Path $PWD 'lucas.exe') -ArgumentList '--calibrate' -Wait
    # After one full calibration, note the result and close the debug window.
} finally {
    $env:LUCAS_CALIBRATION_DIAGNOSTICS = $oldDiagnostics
    $env:LUCAS_GAZE_SOURCE = $oldSource
}
Get-ChildItem .\calibration-diagnostics\calibration-*.json | Sort-Object LastWriteTime -Descending | Select-Object -First 1 FullName
```

Enter the true physical screen size (e.g. 597×336 mm), keep the assumed distance
consistent, and check the selected camera and map alignment. Physical size changes
the reported angular error but **not** the current normalized rectangular score
for identical samples. A different sensitivity on a fresh run needs the numeric
evidence; it cannot be attributed to mm alone. A threshold above 0.5 erodes all
four edges; with fewer than 1,000 negative frames, the unchanged 0.1% FPR budget
allows zero calibration false positives. Early transition samples can therefore
raise that threshold substantially. Exports expose this rather than hiding it.

The requested format is 1280×720 MJPEG at 30 fps. The debug view shows **Face
found**, whether the eyes are usable, raw eye/head features, observed fps, Rust
inference-thread CPU time per frame, and heuristic tracking quality. In v2 traces the CPU figure covers the Rust
inference thread only, not the capture thread or UniGaze child/GPU. The marker
uses camera-space angles for UniGaze or iris ratios for MediaPipe: it is **not
calibrated**, not a screen position, and not a minimap detection. No camera preview is shown or stored. Low quality,
unknown eyes, or low fps are reasons to adjust lighting and placement first.

### Headless camera check (about 3 seconds)

Before the recording, close the debug window and other camera apps. In PowerShell
in the extracted bundle directory, run this standalone option without other flags:

```powershell
$logDirectory = Join-Path $PWD ('camera-check-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $logDirectory | Out-Null
$stdout = Join-Path $logDirectory 'stdout.txt'
$stderr = Join-Path $logDirectory 'stderr.txt'
$check = Start-Process -FilePath (Join-Path $PWD 'lucas.exe') -ArgumentList '--check-camera' `
    -Wait -NoNewWindow -PassThru -ErrorAction Stop `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
Get-Content $stdout
Get-Content $stderr
"Exit code: $($check.ExitCode)"
"Diagnostic logs: $logDirectory"
```

This explicitly waits for the Windows GUI-subsystem executable and captures both
output streams. Use **`$check.ExitCode`**, not `$LASTEXITCODE`, for this invocation.
The logs remain in the printed directory; no images are saved. For the no-camera
`--check-runtime`, `--version` or `--help` diagnostics, use this same pattern with
only the `-ArgumentList` value changed. For `--check-gaze-model`, use the bundled
[`check.ps1` helper](#headless-gpu-model-check-no-camera) instead. Direct invocation
or `| Out-Host` can return early or display no output in interactive PowerShell.

The `--check-camera` option opens camera **index 0** through the same
`NokhwaCamera::open_c270` / `Camera::capture`
path as the app, including acquisition timestamp validation. If several webcams
are attached, disconnect the others so the C270 is index 0. No window, model
inference, recording, or pixel output is involved; frames are cleared in RAM.

Output includes the negotiated width/height/format/fps (expected **1280×720 MJPEG
30 fps**), frames received over about three seconds, effective fps (received
frames divided by elapsed capture/decode time, excluding camera open), and
minimum/median/maximum acquisition age in milliseconds measured just after
capture/decode.
Exit **0** means capture completed; a nonzero exit prints the failing open/capture
error. Low effective fps or large ages still warrant investigation; this is not a
tracking-quality or full G2 acceptance test. The duration is not a hard deadline
for a blocked driver; see the [capture timeout contract](../crates/lucas-gaze/README.md#acquisition-timing).
After the check, reopen diagnostics and reconnect the camera using
[Download and run](#download-and-run) before recording.

**Real-camera verification is pending:** run this diagnostic from the PR's Windows
CI artifact on the affected Windows PC and record its numeric output, exit code,
and build commit before merging the camera fix. Linux unit tests and Windows CI
builds alone do not confirm C270 driver behavior.

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
  and keep looking there until it disappears. Move naturally within your calibrated
  range of positions; do not deliberately freeze your head. **Esc** ends early and saves a trace marked cancelled.
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

- Missing UniGaze model/hash/dependency: quit LUCAS and rerun its `setup.ps1`, then
  run the bundled `check.ps1` as shown in [setup](#one-time-unigaze-setup-nvidia-cuda).
  CUDA unavailable: check the NVIDIA driver and the private
  CUDA 12.8 install; no CPU fallback is used. A child crash/timeout stops tracking
  and reports an error. Reconnect only after the diagnostic succeeds.
- Missing MediaPipe model/hash: re-extract the complete matching bundle. The app
  never downloads replacements at runtime. Do not change model hashes to bypass checks.
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
  opening the camera. `--version` and `--help` do not open a window. Run these
  CLI-only options with the [waited, file-captured pattern](#headless-camera-check-about-3-seconds)
  above, replacing `-ArgumentList` with the desired option.
- Offline, on Windows or Linux: `cargo xtask replay path/to/trace.jsonl` prints
  frames, valid-frame share, observed fps and per-target-kind feature distributions.
  See [the trace format](https://github.com/mackcee/lucas/blob/main/crates/lucas-app/README.md)
  for timestamp and label semantics (select the tested commit when using a PR build).

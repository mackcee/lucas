# Testing LUCAS on Windows

## G1 — timer mode and minimap alignment (about 15 minutes)

This test needs a Windows 10/11 PC, a real Summoner's Rift game and League in
**Borderless** mode. No webcam, models or gaze calibration are needed. Use a
non-critical game for alignment: while editing, LUCAS temporarily reserves the
arrow, Shift+arrow, Enter and Esc keys. It does not send input to League.

### Start and gate checks

1. Launch `lucas.exe` (or `lucas.exe --timer`). It starts in the notification area
   with an amber square-ring icon and no application window. Check the hidden
   tray icons if needed. Starting it again should not create a second icon.
2. Right-click the icon. **Use Live Client gate (recommended)** is checked by
   default. The status/tooltip should say waiting until League is foreground,
   the player is alive in a supported Summoner's Rift game, and game time is
   **past 1:30**. Once eligible, the first pulse is after ten more seconds.
3. Confirm amber ripple pulses around the minimap every **10 seconds**. There
   is no sound. The ring must never take focus or obstruct minimap clicks.
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

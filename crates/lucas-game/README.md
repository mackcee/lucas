# lucas-game

Read-only game-state integration for the Windows MVP. Portable logic and HTTPS
tests run on Linux; `Win32WindowProbe` and the default Windows settings-path
locator are Windows-specific.

## Integration contract

- Create `LiveClient` on a dedicated blocking worker thread. Feed it into
  `Poller::new`; call `poll_if_due` with elapsed monotonic time. It polls at most
  once per second, without catch-up bursts. Four bounded subset requests fetch
  active identity, players, events and (last) game time. Each request has a 750 ms
  deadline, so a degraded poll may exceed the normal one-second cadence.
- Timestamp successful polls **at completion**, with the same monotonic origin as
  gaze samples, and call `GameState::update`. On errors call `unavailable` and
  retain the error for diagnostics. Do not log/persist player lists by default.
- `eligibility(now).eligible()` requires a fresh snapshot, game time **>90 s**,
  map 11, verified `CLASSIC` mode, resolved living player, and no inferred pause.
  Evaluate it every core tick. The adapter to lucas-core's types belongs in the
  app; this crate deliberately does not couple to that parallel implementation.
- Combine that gate with `WindowProbe::sample()?.overlay_eligible()` and face/gaze
  validity. Window-only fallback is an **explicit app policy**, never enabled by
  an HTTP/TLS failure inside this library. It cannot infer deaths, map or loading.
- A stall **>2 s** infers a pause; a successful snapshot **>=3 s** old is stale.
  `game_time_at` interpolates recent poll pairs, clamps just after the latest
  sample instead of advancing with wall time, and refuses outage gaps. Precision
  is approximately one poll; pauses are inferred, not directly exposed by Riot.
- `StateUpdate::deaths` contains exact EventTime values and event IDs, once per
  session, including catch-up events when attaching mid-game. Active identity must
  resolve to a unique player ID; champion names are accepted only for death-event
  attribution, and only when no other player's ID or champion matches. The future
  core integration must compare deaths against gaze history for the >=8 s
  blind-death rule (unknown if history or gaze quality is insufficient). No blind
  classification is guessed here.
- `StateUpdate::events` includes newly seen objective/turret events without
  changing coaching behavior. The consumer should reset its gap clock on
  `respawned`, including when a confirmed death is followed by living gameplay
  after temporary API or identity loss. That remembered life state resets at a
  session boundary. `game_end` is authoritative only for GameEnd. API loss sets
  phase Ended but can be transient; retain session data until reconnect/session
  resolution. See `GameState`'s API documentation for session-boundary rules.

## Settings and windows

`SettingsLocator::windows_defaults(explicit_install_root)` checks an explicit
root, the Riot install manifest, and standard Windows paths. Manifest
`associated_client` keys are installation directories; only roots containing
`LeagueClient.exe` are added. `candidates()` yields PersistedSettings.json before
game.cfg for each root. `SettingsFile::read()` provides text with a `SettingsFormat`
for the planned lucas-geometry parser, which is not implemented in this checkout.
The future adapter should fall through on read or parse error; this crate only
discovers and reads files, not HUD values.

The Win32 probe uses only window/monitor metadata and shell notification state:
`FindWindowW(RiotWindowClass)`, foreground and geometry checks, and
`SHQueryUserNotificationState`. It never opens a game process or installs hooks.
The app must be per-monitor DPI-aware for physical screen coordinates.
Exclusive-fullscreen detection is a **heuristic**, not a swapchain inspection;
a foreground undecorated monitor-sized window plus the shell D3D fullscreen
signal is suppressed. Borderless/Windowed remain the supported configurations.
Errors/unknown mode must suppress the overlay. Validate this heuristic on a real
Windows League session; portable tests cannot prove compositor behavior.

## Validation

```sh
cargo test -p lucas-game
cargo clippy -p lucas-game --all-targets -- -D warnings
```

See `certs/README.md` for trust-anchor provenance and
`tests/fixtures/README.md` for fixture provenance.

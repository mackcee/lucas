# lucas-overlay

A minimap-sized, click-through Windows overlay, plus a pure tiny-skia renderer
and a headless `Recording` implementation. No tray, settings UI, audio, reminder
scheduler, hooks, injection, input synthesis, game-process access, or capture
exists in the product backend. Desktop capture and input synthesis exist **only**
in the explicitly invoked synthetic-window integration test.

## Contract

`Overlay: Send` provides `show_pulse(Pulse)`, `hide()`, `set_target_rect(Rect)`,
`reassert_topmost()`, and `shutdown()`, all returning `Result<()>`.

- Pass the **minimap rect**, not the game-client rect, from `lucas-geometry`.
  Coordinates are physical desktop pixels; negative monitor origins work.
- Calls acknowledge completion. Show restarts a finite 1.2-second animation;
  hide cancels it immediately. The window hides automatically at completion.
- The application owns coaching eligibility and hides when coaching ends.
  Win32 independently checks the target window before presentation and polls it
  at 16-ms intervals during a pulse. Foreground loss or exclusive fullscreen
  cancels the pulse; returning to the game does not resume it. No hooks are used.
  Reassert topmost when an application poll indicates it is needed.
- On a geometry poll, recompute the map from the current game-client rect and
  settings and call `set_target_rect`. A visible pulse follows without restarting
  its animation; moving a hidden overlay does not show it.
- Shutdown is idempotent; other calls after shutdown fail with `Closed`.
  `Win32Overlay` also shuts down and joins on drop. Explicit shutdown reports
  worker/presentation failures; a request after a worker failure returns `Closed`.
- `Recording` validates the same geometry and logs commands in order. It does
  not advance wall-clock time or pretend to observe Windows visibility.

```rust
use lucas_overlay::{Overlay, Pulse, Recording, Rect};

let map = Rect::new(1600.0, 760.0, 1900.0, 1060.0)?;
let mut overlay = Recording::new(map)?;
overlay.show_pulse(Pulse::default())?;
overlay.hide()?;
overlay.shutdown()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

On Windows, replace the constructor with
`win32::Win32Overlay::new(map, win32::TargetWindow::new(game_hwnd), win32::Options::default())`.
Keep the target HWND valid for the overlay's lifetime. `TargetEligibility` is a
small injectable seam for an application adapter to `lucas-game` and synthetic tests;
its `check` must reject background, minimized, missing, and exclusive targets.
The native `TargetWindow` implementation uses public window metadata only.

## Visuals

Default: **amber ripple**, medium salience. `Pulse` carries a style, RGB colour,
and salience independently of scheduling. `Salience` is re-exported from
`lucas-core`; the overlay's explicit default remains Medium. Built-in colours are amber
`#ffb838`, cyan `#43d6eb`, and white; any RGB value is accepted.

| Salience | Core width (physical px) | Peak opacity multiplier | Ripples |
|---|---:|---:|---:|
| Minimal | 1 | 0.24 | 1 |
| Soft | 1.5 | 0.42 | 1 |
| Medium | 2.5 | 0.68 | 2 |
| Strong | 4 | 0.92 | 2 |

Ripples expand outward by 20% of the shorter map dimension, stagger, and fade.
Other data-selectable styles are breathing border, corner brackets, and a comet
sweep. No tint/inner-fill style is provided: map content must remain unobscured.
All styles leave every pixel intersecting the map transparent. The map bounds
round outward; the surface adds `max(24, ceil(longer_side * 0.2)) + 12` physical
pixels on each side for growth and glow. Screen edges naturally clip the ring.
Surface dimensions are capped at 4096 each and desktop edges must fit signed
32-bit coordinates; invalid requests fail before allocation/window mutation.

`render::frame` accepts explicit elapsed time and returns premultiplied RGBA.
`render::copy_bgra` swizzles without unpremultiplying for Win32 presentation.
PNG support is a dev dependency, not enabled in ordinary backend builds.

## Windows integration

The dedicated thread sets **per-monitor-v2** DPI awareness before creating its
`WS_POPUP`. It does not change the embedding application's process DPI state.
The application's geometry-polling thread must also be PMv2-aware (ideally via
its executable manifest). `WM_DPICHANGED` does not reinterpret physical target
coordinates; the application's next geometry poll supplies the new rectangle.

The window uses `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST |
WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, `SW_SHOWNOACTIVATE`, and
`WM_MOUSEACTIVATE -> MA_NOACTIVATE`. It never expands to the game-client area.
A top-down 32-bit DIB is presented with `UpdateLayeredWindow(ULW_ALPHA)`.
Rendering uses a 16-ms Win32 timer only during a pulse. A kernel event wakes the
message loop for commands without relying on the window-message queue quota;
when idle the thread sleeps in `MsgWaitForMultipleObjectsEx`.

Windows 10/11 Borderless (recommended) and Windowed are supported. True exclusive
fullscreen is not. The presentation gate calls `win32::is_exclusive_fullscreen`,
which combines the shell's
`QUNS_RUNNING_D3D_FULL_SCREEN` hint with foreground, popup style, and exact
window/monitor bounds. It is conservative, not definitive: fullscreen
optimizations can make modes indistinguishable. Query failures return errors,
not a false promise of support. Show requests return `TargetNotForeground` or
`ExclusiveFullscreen` when rejected; the latter explains **Switch League to
Borderless or Windowed** for the embedding UI to display. This presentation-time
helper remains independent of `lucas-game`'s discovery probe. See the
[game integration contract](../lucas-game/README.md#settings-and-windows) for that
probe's classification and failure handling.

`Options { exclude_from_capture: true }` opts into
`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. Default is **false**.
API failure fails construction, never silently disables the request. Full
exclusion needs Windows 10 2004+; older versions may supply WDA_MONITOR semantics.
Display affinity is a compositor preference, not a security/privacy boundary.

## Tests and evidence

```sh
cargo test -p lucas-overlay
cargo clippy -p lucas-overlay --all-targets -- -D warnings
```

The cross-platform tests cover command ordering/lifecycle, bounded signed
geometry, salience/fade/expansion, BGRA premultiplication, 21 committed PNG
snapshots, and transparent content/outer edges across every style, salience, and
21 animation phases. Golden fixtures are generated synthetic graphics, not game
screenshots. See `tests/goldens/README.md` before changing them.

On a **dedicated interactive Windows desktop** (the existing GitHub-hosted
`windows-latest` job runs this explicitly):

```powershell
$env:LUCAS_OVERLAY_ARTIFACT_DIR = "$env:TEMP/lucas-overlay-smoke"
cargo test -p lucas-overlay --test windows_smoke windows_overlay_smoke -- --ignored --exact --nocapture
```

The harness spawns a separate, known-colour borderless fake game process. It
asserts window styles/DPI, focus preservation, `WindowFromPoint`, an actual
`SendInput` click delivered to the fake game at an opaque ring pixel, ring pixels
via `BitBlt(CAPTUREBLT)`, unchanged map pixels, topmost recovery after the fake
raises itself, moved/resized map following, auto-hide, foreground-loss cancellation,
injected exclusive-fullscreen rejection, shutdown/drop, and opt-in affinity.
Screenshots contain only the synthetic fixture's current rectangle, with screen
coordinates translated to image-local coordinates. Capture requires the fixture
to be visible and foreground; cleanup never captures the desktop. CI uploads
already-saved fixture images even when assertions fail.
Input is guarded by a hit-test against the synthetic HWND;
the cursor is restored and the child is closed on exit. Do not run this harness
while using the desktop for other work. It does not require League or a webcam.

Linux can type-check/lint the Windows code with an installed MSVC Rust target;
it cannot validate Windows compositor/input behavior. Native CI and a later
real-game Borderless/DPI check remain the runtime evidence for those properties.

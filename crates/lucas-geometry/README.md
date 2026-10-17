# lucas-geometry

Pure, platform-independent minimap rectangles, settings parsing, and gaze-zone
inflation. No disk/window queries, rendering, camera access, or storage.

## Coordinate contract

Every rectangle and offset is in **physical desktop pixels**, with fractional
pixels retained. Rectangles use `[left, right) × [top, bottom)` bounds. Negative
monitor origins work. The caller obtains the game's **client** rectangle (not its
outer frame) in screen coordinates with per-monitor-v2 DPI awareness. At 4K and
150% Windows scaling, pass 3840 × 2160, **not** 2560 × 1440. There is intentionally
no DPI factor here: applying it again would misplace the map.

Rectangles reject nonfinite edges, empty/inverted bounds and unrepresentable
sizes. Geometry functions also validate settings, alignment and monitor inputs.
Rendering code chooses integer rounding (typically floor left/top, ceil
right/bottom); this crate does not round prematurely.

## HUD seed and alignment

For client width `W`, height `H` and stored `MinimapScale = s` in `[0, 3]`:

- square side: `H * (0.1651 + 0.0764 * s)`;
- outer horizontal inset: `W * 0.0039`;
- bottom inset: `H * 0.0069`;
- normally bottom-right, mirrored to bottom-left with `FlipMiniMap = 1`.

These are the **Amberstone community-calibration coefficients** selected in the
LUCAS design plan §5, not a Riot layout guarantee or measurements from the
captain's PC. Height determines HUD size across common 4:3, 16:10, 16:9 and 21:9
clients; the horizontal inset follows width. Another reported calibration
(`H * (0.196 + 0.063 * s)`) differs, especially at low scales. Frame boundaries
and future patches can also differ. The seed must be checked in a real game.
`GlobalScale` and config Width/Height do not affect this calculation.

`Alignment::default()` is identity. Its positive scale multiplier resizes about
the bottom outer corner; then offsets translate in physical pixels (positive
right/down). The result is **not clipped**, so an alignment error is visible
rather than silently changing the rectangle's shape. Profile selection/storage
belongs to the caller, keyed by client resolution, MinimapScale and flip.

## Reading settings

`parse_persisted_settings` reads supplied `PersistedSettings.json` content from
`files[name=game.cfg].sections[name=HUD].settings[{name,value}]`.
`parse_game_cfg` reads supplied `[HUD]` entries. File discovery and reading belong
to [`lucas-game`](../lucas-game/README.md#settings-and-windows); the application
owns refresh scheduling.

Both return a `SettingsPatch` with optional values: missing is different from
zero or false. Prefer the persisted patch and fill missing fields from game.cfg
with `persisted.with_fallback(cfg).resolve(explicit_defaults)`. The provided
`MinimapSettings::default()` uses scale 1 and unflipped as an application fallback,
not an asserted League default. No fallback is silently applied on parse errors;
the caller can report the error and choose the other file or explicit defaults.

- Names are case-insensitive; unrelated keys, sections and files are ignored.
- UTF-8 BOMs, LF/CRLF, whitespace and cfg `;`/`#` comments are supported.
- Scale is the **stored 0–3 number**, never slider percent. Out-of-range or
  nonfinite scales fail rather than being clamped.
- Flip accepts 0/1 or false/true; JSON can hold strings, numbers or booleans as
  appropriate. Relevant null/object/array values fail.
- Duplicate relevant keys fail, including across repeated HUD sections.
- Malformed JSON/container shapes or malformed cfg section headers fail.

## Gaze tolerance

`gaze_zone` expands each edge by an angular **radius** chosen by the caller
(e.g. twice the validated gaze error), then intersects with the monitor bounds.
It uses a local planar approximation:

```text
margin_mm = viewing_distance_mm * tan(margin_degrees)
margin_x_px = margin_mm * monitor_width_px / monitor_width_mm
margin_y_px = margin_mm * monitor_height_px / monitor_height_mm
```

Supply the full physical monitor dimensions and its desktop rectangle, not the
game window size or Windows work area. Dimensions/distance must be positive and
finite; angles must be finite in `[0, 90)` degrees. Zero only clips; no overlap
returns `Ok(None)`. This is an error-tolerance zone, not exact off-axis eye
projection. A game spanning monitors with different pixel pitches needs separate
caller handling. Dwell/hysteresis and the decision to count a check are elsewhere.

## Fixtures and validation

`tests/fixtures/minimap.csv` contains **synthetic seed-regression fixtures**, not
real-game measurements: 1080p, 1440p and 4K, scales 0/1/2/3, both flip states,
a fractional scale, 4:3/16:10/21:9, negative monitor origins and a shifted window.
Expected numbers were tabulated independently from the formula at three decimal
places. The JSON/cfg fixtures are synthetic settings examples, without user data.
Real-game measured rectangle fixtures are still needed at the alignment milestone;
no screenshots or game art are stored here.

```sh
cargo test -p lucas-geometry
cargo clippy -p lucas-geometry --all-targets -- -D warnings
```

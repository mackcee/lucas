# lucas-store

Local-only persistence. The production folder is `%LOCALAPPDATA%\lucas`;
`Store::open_in(path)` supplies a dedicated override for tests or development.
A missing `LOCALAPPDATA` is an error, not a fallback into the working directory.

## Files and schema

- `settings.toml`: serde schema version 1. Missing fields, including nested pulse
  and calibration fields, get defaults; unknown keys are tolerated. Malformed
  values and newer versions return errors and are never silently replaced. Defaults are an amber
  ripple, sound off, Live Client gate and window-only fallback on, and the first
  10-second ramp level. Settings changes replace the file atomically. The
  [`Settings` types](src/settings.rs) own field names, enum spellings and limits;
  see the [calibration guide](../lucas-app/README.md#in-app-calibration) for display
  preference usage and the [`Store` save methods](src/database.rs) for merge and
  cross-process locking contracts.
- `calibration.json`: versioned opaque model bytes (1–65536 bytes), virtual-desktop
  physical screen geometry, creation time in Unix milliseconds, and optional
  validation error in degrees. The payload is owned by
  [`lucas-gaze`](../lucas-gaze/README.md#per-user-calibration-m51).
  No calibration samples or camera frames. Writes replace the file atomically.
- `games.sqlite3`: bundled SQLite, with transactional `PRAGMA user_version`
  migrations. Version 1 has one `games` row per game ID; `snapshot_json` contains
  the complete core metrics, role/champion, length, initial/current level,
  death threshold, and aggregate tracking quality. Indexed columns expose end
  time, probe flag, C, standard, goal changes, and valid-frame fraction. The JSON
  snapshot has an empty events array: events live only in the separate table.
- `events`: game foreign key, stable ordinal, game-time milliseconds, kind
  (`check`, `pulse`, `death`), death classification, optional seconds since last
  minimap evidence, and coverage of the death-classification window. There are
  **no per-frame features, gaze transitions, glances, or images** in this table.

`save_summary` atomically upserts by game ID and replaces its events, so retries
cannot duplicate a death. Counts and finite measurements are validated before
writing; a solo probe containing assistance pulses is rejected. `summary`, `latest_summary`, `events`, and `probe_trend` query saved data;
events are sorted by game time and original ordinal. Undefined measurements stay
`None`, not zero. Probe scores include their standards and tracking quality so UI
consumers need not join incomparable goals or present poor tracking as progress.

## Game-end adapter contract

```rust,ignore
let record = session.finish(end_monotonic)?;
let summary = GameSummary::from_session(&record, context, |at| {
    clock_alignment.game_time_ms(at)
})?;
store.save_summary(&summary)?;
```

The caller supplies `GameContext` (including actual valid/total frame counts),
finalizes delayed/deduplicated deaths before finishing the core session, and
provides a **pause-aware** mapping for every retained event. Do not pass a guessed
wall-clock offset; absent mappings make `GameSummary::from_session` return an
error rather than store false game timestamps. Core owns death classification and
the elapsed evidence-gap value; this crate does not reclassify deaths or derive
frame quality from check counts.
`deaths_available = false` distinguishes window-only mode from zero deaths.
`standard_changed` keeps mid-game goal changes out of fixed-goal trend lines.
Automatic promotion, game-clock alignment, and probe scheduling are not part of
this crate.

## Privacy controls

`data_folder()` exposes the path for the app's **Open data folder** action.
`delete_all_data(self)` consumes the owner and closes SQLite first, then removes
the database and SQLite sidecars, settings, tray alignment profiles
(`alignment.toml`) and their interrupted write (`alignment.toml.tmp`), calibration,
interrupted atomic writes with the private `.lucas-write-` prefix, and the `traces`
and `reports` directories. It leaves the folder itself and unrelated files intact.
Directory symlinks are removed, not followed. No files are sent elsewhere.
Explicit numeric calibration exports outside these paths, such as the
`calibration-diagnostics` folder beside the app, are not removed by **Delete all
data**. Delete those exports separately from the directory you selected.

The caller **must close the tray (gaze or timer mode), stop other calibration,
trace/report writers, and release every other database handle before deletion**.
Discard cached settings, calibration and alignment state before any later save;
otherwise a running process could restore deleted data. Errors are returned
(deletion may be partial); reopen the same folder after releasing handles to retry.
The separate opt-in trace feature may write to `traces`, but this crate has no
trace-writing API.

```sh
cargo test -p lucas-store
cargo clippy -p lucas-store --all-targets -- -D warnings
```

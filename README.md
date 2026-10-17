# LUCAS

**League Universal Coach AI System** is an open source map-awareness trainer for
League of Legends. It aims to help you build the habit of checking the minimap,
then get out of the way as that habit becomes natural.

Webcam gaze tracking estimates whether you glanced at the minimap. After a gap
without a glance, an amber ripple pulses around the map; sound is optional and
off by default. Training levels shorten the gap, then fade the help; level changes
are currently manual. The game's local Live Client Data API gates pulses to
gameplay.

## Status

Early development: **M1.1 implements the pure training core**. See
[`lucas-core`](crates/lucas-core/README.md) for its input contract, manual programme,
and metrics. **M2.1 adds the standalone `lucas-geometry` library**; see its
[geometry and settings reference](crates/lucas-geometry/README.md). **M2.2 adds the
standalone `lucas-overlay` library**; see its [overlay and Windows integration
reference](crates/lucas-overlay/README.md). **M3.1 adds the `lucas-game` library**
for local game-state integration; see its
[integration contract](crates/lucas-game/README.md). **M4.1 adds the
[`lucas-gaze`](crates/lucas-gaze/README.md) library** with Windows webcam capture,
offline face/iris landmarks and filtered gaze features. **M3.2 wires the Windows
`lucas` binary into a tray timer app**: a ten-second amber reminder, Live Client
alive/game gating (with explicit window-only fallback), minimap alignment and
numeric geometry reports. See the [G1 Windows checklist](docs/testing-on-windows.md#g1--timer-mode-and-minimap-alignment-about-15-minutes).
**M6.1 adds local persistence in [`lucas-store`](crates/lucas-store/README.md) and a
standalone [post-game summary card](crates/lucas-app/README.md)**. Without arguments,
`lucas` now starts **gaze coaching on Windows (M5.2)**. Complete calibration and
the tray's four-second quick check at each game start; unknown/off-screen gaze
pauses coaching. Summaries are stored locally and the card opens at game end.
See the [runtime guide](crates/lucas-app/src/runtime/README.md) for eligibility,
camera lifetime and clock mapping. `--timer` remains an explicit no-camera option;
the viewer below opens synthetic or saved summaries.

**M4.2 adds a Windows gaze debug window and opt-in numeric glance-test recorder**
to `lucas`; follow the [Windows testing guide](docs/testing-on-windows.md) to
download a ready-to-run CI build. Diagnostic camera access requires clicking Connect camera;
recording requires a separate opt-in. Diagnostics use `lucas-game` to locate
minimap settings and make no network requests.

**M5.1 adds in-app calibration and a per-user minimap classifier**. Open
`lucas --calibrate` and follow the [calibration guide](crates/lucas-app/README.md#in-app-calibration).
For offline recording evaluation, see the [G2 evaluator](xtask/README.md#g2-calibration-and-glance-evaluation).

The application targets **Windows only**. League must run in **Borderless or
Windowed mode**, not exclusive fullscreen. The workspace and gaze fixture tests
can also be built and checked on Linux. Builds will initially be unsigned.

## Privacy

LUCAS has a local-only privacy contract: webcam frames never leave RAM and are
never written to disk. Nothing is sent off the machine. The only
network connection during app use is to the game's own local API at
`127.0.0.1:2999`.

Normal-use local persistence is limited to settings, calibration, and per-game summaries
(including event timestamps); see the [storage reference](crates/lucas-store/README.md)
for files and deletion scope. The tray records games; the standalone viewer only
opens saved summaries.
Demo mode uses only unsaved synthetic data. Before deleting saved data from the
viewer, follow the [deletion prerequisites](crates/lucas-store/README.md#privacy-controls).
The opt-in debug glance test additionally
stores numeric eye/head features, quality, target labels and timestamps, plus a
camera/monitor metadata header and your lighting note. **No images are stored.**
See the [recording guide](docs/testing-on-windows.md#record-the-test-about-10-minutes)
for local trace location, deletion and explicit sharing.
The tray persists alignment profiles and user-requested numeric geometry reports
in `%LOCALAPPDATA%\lucas`. Its default gate polls the local game API; window-only
mode disables polling (an already-running request may finish).
Developer build and dependency-check tools may access package registries and
advisory databases.

## Build and check

Install [Rust via rustup](https://rustup.rs/), then run from the repository root.
The toolchain and required components are pinned in
[`rust-toolchain.toml`](rust-toolchain.toml).

```sh
cargo build --workspace
cargo run --bin lucas -- --version
# Windows: start gaze coaching (requires models/ beside the built executable)
cargo run --bin lucas
# Explicit no-camera timer fallback:
cargo run --bin lucas -- --timer
# Windows diagnostics and calibration (models from this checkout):
cargo run --bin lucas -- --calibrate --models models
# Replay numeric traces on Windows or Linux:
cargo xtask replay path/to/trace.jsonl

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cargo install cargo-deny --locked
cargo deny check
```

The first build may download the pinned toolchain and the ONNX Runtime library. `cargo deny` checks dependency
licenses, sources, bans, and RustSec advisories; its initial check requires network
access to fetch the advisory database.

### Preview the post-game card

```sh
# Synthetic inputs; no camera, network or real data opened.
cargo run -p lucas-app -- --show-postgame demo

# Latest saved game, quoting paths that contain spaces:
lucas --show-postgame "C:\Users\you\AppData\Local\lucas\games.sqlite3"
# The data directory is accepted instead of the database path.
```

The saved-game command requires an existing database containing a summary.
The card provides **Open data folder** (Windows) and **Delete all data** controls;
demo mode disables both. See the [card reference](crates/lucas-app/README.md) for
metric interpretation and integration.

### Workspace

| Crate | Responsibility |
| --- | --- |
| `lucas-core` | Pure glance detector, gap clock, pulse policy, levels, metrics, session types |
| `lucas-geometry` | Pure minimap rectangle calculation and gaze zone inflation |
| `lucas-gaze` | Camera, Landmarker, and GazeSource interfaces; capture and calibration |
| `lucas-game` | Local game API, game state, settings, foreground/fullscreen detection |
| `lucas-overlay` | Minimap presentation; see the [overlay reference](crates/lucas-overlay/README.md) |
| `lucas-store` | TOML settings, calibration profiles, SQLite sessions |
| `lucas-app` | Windows gaze-coaching tray, explicit timer fallback, alignment, per-game storage/card, calibration, gaze debug UI and opt-in traces |
| `xtask` | Synthetic-player simulator, trace replay and G2 evaluation, offline model conversion and hash verification |

These are responsibility boundaries; implementation status is described
[above](#status). The gaze library uses hash-pinned MediaPipe models through ONNX
Runtime behind a `GazeSource` trait; see
[`models/README.md`](models/README.md) for attribution and development-only
conversion commands. Models are never downloaded at application runtime.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Third-party MediaPipe models remain Apache-2.0; see [NOTICE](NOTICE).
For bundled UI font notices and distribution requirements, see the
[card reference](crates/lucas-app/README.md).

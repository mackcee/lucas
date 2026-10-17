# LUCAS

**League Universal Coach AI System** is an open source map-awareness trainer for
League of Legends. It aims to help you build the habit of checking the minimap,
then get out of the way as that habit becomes natural.

The planned flow is simple: webcam gaze tracking estimates whether you glanced at
the minimap. After a gap without a glance, an amber ripple pulses around the map;
sound will be optional. The standard training progression shortens the gap from
10 to 8 to 6 to 5 seconds, then fades the help as you improve. The game's local
Live Client Data API will gate pulses to gameplay.

## Status

Early development: **M2.1 adds the standalone `lucas-geometry` library**; see its
[geometry and settings reference](crates/lucas-geometry/README.md). It is not yet
integrated into the `lucas` binary, which still only prints its version. Gaze
tracking, calibration, overlays, storage, and training are not implemented yet.
No camera is opened and no network requests are made by the current application.

The application targets **Windows only**. League must run in **Borderless or
Windowed mode**, not exclusive fullscreen. The current workspace can also be
built and checked on Linux. Builds will initially be unsigned.

## Privacy

The planned application has a local-only privacy contract: webcam frames never
leave RAM and are never written to disk. Nothing is sent off the machine. The only
network connection during app use is to the game's own local API at
`127.0.0.1:2999`.

Only settings, calibration, and per-game summaries (including event timestamps)
will be stored locally. Raw webcam frames will not be stored. The current scaffold
does not collect or persist any data. Developer build and dependency-check tools
may access package registries and advisory databases.

## Build and check

Install [Rust via rustup](https://rustup.rs/), then run from the repository root.
The toolchain and required components are pinned in
[`rust-toolchain.toml`](rust-toolchain.toml).

```sh
cargo build --workspace
cargo run --bin lucas

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cargo install cargo-deny --locked
cargo deny check
```

The first build may download the pinned toolchain. `cargo deny` checks dependency
licenses, sources, bans, and RustSec advisories; its initial check requires network
access to fetch the advisory database.

### Workspace

| Crate | Responsibility |
| --- | --- |
| `lucas-core` | Pure glance detector, gap clock, pulse policy, levels, metrics, session types |
| `lucas-geometry` | Pure minimap rectangle calculation and gaze zone inflation |
| `lucas-gaze` | Camera, Landmarker, and GazeSource interfaces; capture and calibration |
| `lucas-game` | Local game API, game state, settings, foreground/fullscreen detection |
| `lucas-overlay` | Overlay interface, future Win32 layered overlay, recording implementation |
| `lucas-store` | TOML settings, calibration profiles, SQLite sessions |
| `lucas-app` | `lucas` binary, future tray application and UI |
| `xtask` | Development tooling scaffold |

These are intended responsibility boundaries; implementation status is described
[above](#status). No model files are included yet. The gaze pipeline will use
MediaPipe models through ONNX Runtime behind a `GazeSource` trait; see
[`models/README.md`](models/README.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. See [NOTICE](NOTICE) for the model attribution placeholder.

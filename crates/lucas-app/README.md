# Post-game summary window

This chunk adds `lucas_app::postgame`, independent of the tray, camera debug
window, and future game loop. It never opens automatically during play.

See the root README's [viewer commands](../../README.md#preview-the-post-game-card)
for synthetic and saved-game previews.

An empty or missing database produces an explanatory error without launching a
window. The native UI targets Windows; Linux builds/headless tests and Xvfb are
useful for development. **Open data folder** launches Explorer on Windows. Demo
mode disables data controls and identifies all its data as synthetic.

## Integration at game end

After finalizing and saving a summary with `lucas-store`:

```rust,ignore
let card = lucas_app::postgame::PostGameCard::from_store(store, &summary.id)?;
lucas_app::postgame::show_postgame(card)?;
```

`show_postgame` starts an eframe event loop and belongs on the main/UI thread.
An already-running UI should embed `PostGameCard::show(ui)` instead. Game-end
routing, suppressing the card during play, and pause-aware event clock mapping
remain the later loop's responsibility. Follow the storage reference's
[deletion prerequisites](../lucas-store/README.md#privacy-controls) first:
ownership transfers into the card so **Delete all data** can close SQLite before
deleting files. Deletion requires a second,
explicit confirmation and clears the in-memory card on success. If deleting
fails, keep the card open, close other LUCAS windows, and choose **Delete all data**
again. The card can retry even if the database or summary has already been removed.

## Reading the card

- C uses core's **eligible time**, not all elapsed game time. Undefined C is shown
  as unavailable; tracking quality and the amount of eligible time remain visible.
- Blind / total includes unknown deaths in the total but never in the blind count.
  Each expandable death row gives its elapsed evidence gap, classification-window
  coverage, and timeline of checks/pulses in game seconds relative to death.
  The elapsed gap is shown separately, not as a band on the pause-aware timeline.
  Unknown gaps are explicitly uncertain; no earlier evidence is not fabricated
  as a zero gap.
- Probe lines require at least two measured probes. Lines join only consecutive
  fixed-standard probes with the same goal; missing or changed goals break the
  line. Low tracking gets a warning. These are raw observations, not a promotion
  assessment or a streak.
- Wheels text is derived from `lucas-core::TrainingLevel`. The next target is
  phase/level-specific guidance for the planned programme. Core currently exposes
  only **manual** levels, so the card explicitly does not claim to evaluate or
  perform automatic promotions.

The UI uses eframe/egui 0.36 and egui_plot 0.37 (the latter targets egui 0.36).
The default font notices are retained under `licenses/`; distributions should
include that directory. The dependency policy exception is scoped to egui's
bundled font package. No advisory exception was added.

```sh
cargo test -p lucas-app
cargo clippy -p lucas-app --all-targets -- -D warnings
```

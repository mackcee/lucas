# Development simulator

```sh
cargo xtask sim
cargo xtask sim --cap both
cargo xtask sim --games 6 --minutes 30 --means 6,10,15 --levels 1,7,8 --seed 42
cargo xtask sim --levels 0,9 --response 0
cargo xtask sim --help
```

Defaults: 20 games × 30 **eligible** minutes, habitual means 4, 6, 8, 10,
15 and 20 seconds, level 1, density cap on, 80% pulse response, seed 42.
`--response` is a probability in `[0, 1]`, not a percentage. `--cap both`
prints uncapped/capped rows side by side. Lists are comma-separated; unknown
options and invalid/nonfinite inputs fail with exit code 2.

## What runs

Each game creates a fresh `lucas_core::Session`. Synthetic high-quality gaze
samples at 50 ms intervals pass through the **production detector, eligible-time
gap clock, pulse policy and metrics**. There is no replacement policy or scoring
formula in xtask. Core supplies its default dwell/hysteresis, ±10% delay jitter,
one opportunity per repeating interval, response attribution, and the Fade-only
density staircase. Ramp keeps its selected cadence with either cap setting.
The cap starts fresh each game, as does the session; no automatic level
progression is simulated.

The model intentionally isolates pulse load from camera and game integration:

- Natural inter-glance intervals are gamma(shape = 2, scale = mean / 2), sampled
  as the sum of two exponential variates. Glances last 300 ms. Sampled intervals
  have a 350 ms floor (one away sample between glances) and are observed on the
  next 50 ms tick. Thus extremely small configured means cannot produce that
  same observed mean. The initial wait has only a 50 ms floor.
- Intervals are measured between glance **onsets**, not gamma plus dwell.
  Core's reported gaps exclude sustained confirmed minimap viewing; its median
  will therefore differ slightly from the gamma onset-interval median.
- A delivered pulse is answered with the configured probability. Response onset
  latency is uniform over 0.4–1.2 s, rounded to the nearest 50 ms. Detector
  confirmation adds its normal dwell delay. A natural glance that comes sooner
  wins; a pulse during an already-started glance does not queue a second one.
  Each actual glance restarts the natural renewal process.
- All time is active, alive, foreground, tracked and after the 90-second gate.
  A “30-minute game” here means 30 eligible minutes, not 30 wall-clock minutes
  including loading or deaths. No frames, game API, webcam or storage are used.
- Habit, pulse-response, latency and policy RNG streams are separate and seeded.
  Seeds are keyed by habitual mean and game index, not table order. Selecting
  different levels/caps or reordering means does not select different players.
  This is reproducible synthetic behaviour, **not evidence about real players**.

## Levels

`--levels` selects indices directly from core's `TRAINING_LEVELS`:

| Index | Phase | Standard S | Nominal delay | Pulse probability |
| --- | --- | --- | --- | --- |
| 0 | Baseline | 10 s | off | 0 |
| 1–4 | Ramp | 10, 8, 6, 5 s | same as S | 1 |
| 5–6 | Fade | 5 s | 6, 8 s | 1 |
| 7 | Fade | 5 s | 10 s | .75 |
| 8 | Fade | 5 s | 15 s | .5 |
| 9 | Graduated | 5 s | off | 0 |

The table prints S, nominal delay and probability so comparisons are explicit.
The old design prototype's 15 s/.75 and 20 s/.5 settings are **not** levels in
the approved ramp-then-fade programme. No custom core levels are introduced to
reproduce those obsolete settings.

## Metrics

- **Pulses/game:** total emitted pulses / number of games.
- **Gaps prompted:** fraction of assisted positive gaps that received a pulse,
  including unanswered pulses. Positive initial and terminal censored gaps are
  included. Multiple reminders in one gap count as **one prompted gap**, while
  each reminder counts toward pulses/game. The share therefore stays within
  [0,1] even when reminders go unanswered. It is **not** `1 − U`, which counts
  checks in core's post-pulse attribution window instead.
- **Solo blind:** time-weighted unassisted blind fraction at the row's S,
  `(eligible_ms − compliant_ms) / eligible_ms`. A separate paired run uses the
  same habit seeds but discards *all* pulse outputs rather than delivering them
  to the player. Core still evaluates the selected policy so scoring uses the
  correct standard; only blind time and gap medians are read from that run.
  Its pulse/attribution metrics are deliberately not reported.
- **Solo median:** median of the per-game core gap medians, in seconds, from
  that same unassisted run (not a pooled median or the assisted median).

This separation prevents assistance from being mistaken for improved habits.
Changing response probability or the cap cannot improve the reported solo
metrics. Missing ratios/medians are printed as `n/a`.

## Regression guard

```sh
cargo test -p xtask
cargo clippy -p xtask --all-targets -- -D warnings
```

These tests also run in the existing Linux and Windows workspace CI jobs.
The main regression uses six 30-minute games per habit. It compares Ramp pulses
to the repeating-policy anchors in [`src/regression.rs`](src/regression.rs)
within ±25%, and analytical solo blind time within four percentage points.
Ramp cap-on/off loads must be identical.
A separate Fade level 7 regression uses means 15/20 s and requires capped load
to remain 30–75% of uncapped load, preserving staircase settling time. Tests also
cover repeated unanswered reminders as a single prompted gap, the approved fade
levels, zero-assistance levels, gamma mean/variance, response timing, seed/order
reproducibility, exact eligible duration, and CLI validation.

Generate current load estimates directly from the simulator rather than a copied
output table:

```sh
cargo xtask sim --cap both
cargo xtask sim --games 6 --cap both --levels 7
```

The first command compares Ramp cap settings; the second exercises Fade.
These are synthetic load estimates, not real-game efficacy evidence. The
regression anchors replace the old one-shot prototype references; bands are
intentionally not golden exact counts, so harmless timing changes need not update
a snapshot.

## G2 calibration and glance evaluation

```sh
cargo xtask eval glance-daylight.jsonl glance-night.jsonl
cargo xtask eval --calibration calibration-session.jsonl unseen-session.jsonl
cargo xtask eval --width-mm 598 --height-mm 336 --distance-mm 650 glance.jsonl
```

Reads the recorder's [JSON-lines format](../crates/lucas-app/README.md#json-lines-schema-v2-v1-remains-readable);
writes no files or weights.
Without a separate calibration trace, each class's target occurrences are split
chronologically into 50% fit, 25% threshold-validation, and the rest held-out test
(rounding fit/validation down). With `--calibration`, 75%/25% of that file's
occurrences fit/validate the model and **all** other files' trials are evaluated.
Use a genuinely independent recording, not a copy of the fitting file. Different
monitor resolutions, minimap settings or gaze sources between calibration and
evaluation are rejected. Multi-file results are
reported independently, never pooled across camera positions or people.

The first 300 ms of each displayed hold/focus return are excluded for saccade and
filter settling. No countdown/unfocused frames enter fitting or scoring. Unknown
hold frames remain in the denominator: they cannot count as checks. Splitting is
by whole trial, never by nearby correlated frames. Missing expected trials remain
missed glances/missing coverage, rather than disappearing from the denominator.

The same calibrated decision layer as the app processes each evaluation trial,
reset at trial boundaries so a previous dot cannot donate dwell. A minimap trial
is detected if it starts at least one confirmed check. Every check-start in an
elsewhere trial is a false detection. Elsewhere exposure sums observed intervals
<=250 ms, capped to each hold minus settling; missing camera time cannot dilute
false detections/minute. Known/usable frame share includes off-screen rejection.

G2 requires >=90% detected minimap trials and <=1 false event/minute elsewhere.
A completed trace, >=80% temporal coverage and >=80% known frames are additionally
required to avoid declaring success from missing evidence. Exit codes: **0** all
traces GO, **1** at least one NO-GO, **2** malformed/insufficient inputs. The output
includes split sizes, empirical held-out ROC, fitted threshold and mean error in
degrees. Defaults assume 531×299 mm and 650 mm distance, recorded as assumed; pass
physical dimensions for meaningful angular diagnostics.

A short calibration ROC is not a confidence bound, and scripted four-second
fixations do not establish performance on every in-game glance duration. The
synthetic tests exercise both GO and NO-GO paths; they are **not** the real-webcam
G2 gate. Run these commands on the captain's recordings when available; no new
code or video conversion is needed.

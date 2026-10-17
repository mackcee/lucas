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
one opportunity per gap, response attribution, and density staircase. The cap
starts fresh each game, as does the session; no automatic level progression is
simulated.

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
  included. Because core emits at most one pulse per gap, this is total pulses /
  total gaps. It is **not** `1 − U`, which counts checks in core's post-pulse
  attribution window instead.
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
The main regression uses six 30-minute games per habit, preserving the density
staircase's settling time. It compares pulses to the design plan §13 P4 within
±25%, and solo blind time within four percentage points. For means ≥10 s it
also requires capped load to be 30–75% of uncapped load. Tests cover the approved
fade levels, zero-assistance levels, gamma mean/variance, response timing,
seed/order reproducibility, exact eligible duration, and CLI validation.

For orientation, `cargo xtask sim --cap both` (seed 42, default 20 games) gives:

| Habit mean | Pulses/game off / on | Solo blind at S=10 |
| --- | --- | --- |
| 4 s | 15.4 / 15.4 | 1.9% |
| 6 s | 46.2 / 46.1 | 8.8% |
| 8 s | 69.0 / 62.8 | 17.1% |
| 10 s | 85.3 / 56.1 | 25.6% |
| 15 s | 102.5 / 46.7 | 43.5% |
| 20 s | 111.0 / 45.5 | 54.0% |

The historical anchors are about 85 uncapped pulses for the 10-second habit,
and about 105 → 47 with the cap for the 15-second habit. Small differences are
expected from sampling, glance timing, and using the real detector and policy
instead of the discarded prototype. Bands are intentionally not golden exact
counts; a policy change that floods players should fail the tests without
requiring every harmless timing change to update a snapshot.

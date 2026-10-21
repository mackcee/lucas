# lucas-core

Pure, clock-free training logic. No platform code, camera frames, I/O, system
clock, or implicit randomness. `Session<R: RandomSource>` is the app/simulator
entry point; the detector, gap clock, pulse policy, and metrics accumulator are
also usable independently. See the crate-level Rust example.

## Input contract

- `Timestamp(u64)` is **milliseconds on one caller-owned monotonic timeline**.
  Align Live Client death timestamps to that timeline before calling `death`.
- Submit `GazeSample { at, p_minimap, quality }` with a `GateInputs` snapshot.
  Probabilities and quality are in `[0, 1]`; no face is quality zero. Nonfinite,
  out-of-range, or low-quality samples are **unknown**, never an away observation.
- Samples must have strictly increasing timestamps. Rejected duplicate/backward
  samples do not mutate the session. Lower-level interval APIs allow equal
  timestamps to express simultaneous state changes without elapsed time.
- Gate changes take effect at the supplied timestamp. Provide an explicit unknown
  sample on a gate-only tick if there is no new camera sample. Silence expires
  tracking after the configurable maximum sample gap (default 250 ms).
- `active_game` encodes caller policy such as supported game mode and map. Pulses
  require active game, alive, game time **greater than 90 seconds**, foreground,
  tracked face, and not paused. A window-only adapter supplies its fallback game
  values explicitly; core does not call any game API.
- `SessionUpdate.events` contains only new events from that observation. Render a
  `Pulse` as a one-shot animation. Hide it when eligibility is false or gaze is
  `Minimap`. Settings for style, colour, and sound belong to the consuming app.
  Gate events record input-state changes and eligibility threshold crossings,
  not ordinary game-clock progression.

## Detection and timing

Defaults require probability at least 0.8 for **both** 100 ms and three distinct
consecutive samples. A confirmed glance persists until probability drops below
0.6. Unknown interrupts candidates and closes a confirmed glance at its last
observed frame; missing frames cannot satisfy dwell. One sustained view emits one
check and holds the gap at zero.

The gap clock measures **eligible time**, freezing on tracking/foreground loss or
pause, and resetting on death, pre-game, or game end. A segment starts at gap zero
without inventing a check. Death classification uses elapsed time on the input
timeline, not the frozen eligible-time gap.

## Manual programme and pulse policy

`TRAINING_LEVELS` exposes baseline, ramp, fade, and graduated levels as data:

| Phase | Standard | Pulse delays | Assistance |
| --- | --- | --- | --- |
| Baseline | 10 s | off | none |
| Ramp | 10 → 8 → 6 → 5 s | matches standard | strong, probability 1 |
| Fade | 5 s | 6 → 8 → 10 → 15 s | strong → medium → soft → minimal; final probabilities 0.75, 0.5 |
| Graduated | 5 s | off | none |

`Session::set_level` is explicit; there is no promotion or probe scheduling.
Changes apply at the latest observation boundary, reset the pulse opportunity and
staircase, and are recorded as events. Constructors and setters accept only the
levels in `TRAINING_LEVELS`; custom programmes are not supported.

Reminders repeat while the eligible no-check gap continues. Each interval samples
±10% jitter **once** and makes **one** probability draw when due, even when that
opportunity is skipped. At Ramp 5 s with midpoint jitter, pulses occur at 5, 10,
15 s, etc. High tick rates do not retry a skipped interval. Late ticks start the
next interval from the observed eligible gap, never emit catch-up bursts. Checks
restart the interval; sustained minimap viewing holds the gap at zero. Gate
freezes preserve interval progress; death/loading/end reset it.

Ramp always uses the selected level delay. Its previous +750 ms staircase after
each pulse would steadily defeat the selected cadence, so the density cap now
only adapts **Fade** levels. In Fade, the nominal delay remains
`max(level_delay, staircase_delay)` when the cap is on (default). The staircase
starts at the level delay, rises 750 ms after each emitted pulse, and falls 250 ms
after each unprompted check, saturating at zero. With the cap off, Fade uses its
level delay. Probability, jitter, level definitions and off levels are unchanged.
`SeededRng` provides reproducible SplitMix64, or callers can inject their own
`RandomSource`.

## Metrics and persistence

`SessionRecord` is a serde-serializable version-1 storage-neutral payload with
session ID, timestamps, configuration, initial level, event log, and summary.
It never contains images or per-frame gaze probabilities. Event arrival order is
preserved, so late deaths can appear after newer events. `Session::death`
deduplicates the local player's deaths by timestamp. Submit delayed death events
before consuming the session with `finish`.

Definitions:

- **C**: eligible milliseconds at gap ≤ the interval's standard / eligible time.
  Changing standard does not rewrite previous intervals' scores.
- Checks/minute uses eligible time. Sustained minimap viewing remains compliant.
- Gap median, p80, p90, and maximum use positive eligible-time gaps, including
  initial/terminal censored gaps. Percentiles use type-7 linear interpolation.
- Blind spells are distinct gaps **strictly greater than** 15 s (configurable).
- Mean dwell uses completed confirmed glances; `finish` closes the current glance
  at the last observed frame. Live snapshots omit the still-open dwell.
- **U**: checks outside the inclusive 3 s post-pulse window / all checks.
- Every emitted reminder counts as a pulse, including repeats within one gap;
  skipped opportunities do not count. Stored summaries retain each pulse event.
- Pulse-to-glance latency pairs the most recent outstanding pulse with the next
  check, even if it is outside the prompted window. Unanswered pulses have no
  latency, not zero latency.
- Anticipation share: unprompted checks in the inclusive last 1 s before the
  next sampled pulse deadline / all unprompted checks. For repeats this is the
  interval start plus its delay on the eligible gap clock, not the first interval's
  delay. Off levels have no deadline.
- Blind death: most recent valid minimap evidence at least 8 s ago (configurable),
  or no observed check with at least a full threshold window of session history.
  A more recent check is seen; insufficient history without a check is unknown.
  If usable gaze evidence is missing for **more than half** of that preceding
  window, the result is unknown. Detector-suppressed intervals (including pause,
  foreground loss, and inactive gameplay) count as missing evidence, even with a
  tracked face. Coverage is duration-weighted, not frame-count-weighted;
  unobserved history before session start is unknown. Delayed deaths are classified
  against historical evidence, not current gaze.
- Undefined ratios and percentiles are `None`, not NaN or fabricated zeroes.

Lower-level `GapClock` and `MetricsAccumulator` consume explicit state intervals
and do not expire missing camera frames themselves; `Session` performs expiry.
The accumulator expects validated detector events and caller-deduplicated deaths;
prefer `Session` when composing the whole pipeline.

## Tests

```sh
cargo test -p lucas-core
cargo clippy -p lucas-core --all-targets -- -D warnings
```

Component tests cover boundary conditions, unknown tracking, dwell/hysteresis,
freezing/resetting, cap/probability behaviour, hand-computed summaries, delayed
deaths, serialization, and end-to-end traces. Proptest checks detector invariants,
a reference gap state machine, weighted staircase arithmetic, seeded jitter,
closed-form metrics, tick-density independence, time-weighted tracking coverage,
and gated session behaviour.

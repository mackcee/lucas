//! Synthetic gaze only: policy, detection, gap timing and scoring stay in core.
use lucas_core::{
    CoreError, EventKind, GameMetrics, GateInputs, GazeSample, RandomSource, SeededRng, Session,
    SessionConfig, TRAINING_LEVELS, Timestamp,
};

const TICK_MS: u64 = 50;
const GLANCE_MS: u64 = 300;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub games: usize,
    pub duration_ms: u64,
    pub means: Vec<f64>,
    pub levels: Vec<usize>,
    pub caps: Vec<bool>,
    pub response: f64,
    pub seed: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            games: 20,
            duration_ms: 30 * 60 * 1_000,
            means: vec![4.0, 6.0, 8.0, 10.0, 15.0, 20.0],
            levels: vec![1],
            caps: vec![true],
            response: 0.8,
            seed: 42,
        }
    }
}

impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            if !matches!(
                flag.as_str(),
                "--games"
                    | "--minutes"
                    | "--means"
                    | "--levels"
                    | "--cap"
                    | "--response"
                    | "--seed"
            ) {
                return Err(format!("unknown option: {flag}"));
            }
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            let invalid = || format!("invalid value for {flag}: {value}");
            match flag.as_str() {
                "--games" => config.games = value.parse().map_err(|_| invalid())?,
                "--minutes" => {
                    let minutes: f64 = value.parse().map_err(|_| invalid())?;
                    if !minutes.is_finite() || !(1.0 / 1_200.0..=1_440.0).contains(&minutes) {
                        return Err("--minutes must describe 50 ms to 24 hours".into());
                    }
                    config.duration_ms = (minutes * 60_000.0).round() as u64;
                }
                "--means" => {
                    config.means = value
                        .split(',')
                        .map(|v| v.parse().map_err(|_| invalid()))
                        .collect::<Result<_, _>>()?;
                }
                "--levels" => {
                    config.levels = value
                        .split(',')
                        .map(|v| v.parse().map_err(|_| invalid()))
                        .collect::<Result<_, _>>()?;
                }
                "--cap" => {
                    config.caps = match value.as_str() {
                        "on" => vec![true],
                        "off" => vec![false],
                        "both" => vec![false, true],
                        _ => return Err("--cap must be on, off or both".into()),
                    };
                }
                "--response" => config.response = value.parse().map_err(|_| invalid())?,
                "--seed" => config.seed = value.parse().map_err(|_| invalid())?,
                _ => unreachable!(),
            }
        }
        if config.games == 0 {
            return Err("--games must be positive".into());
        }
        if config
            .means
            .iter()
            .any(|m| !m.is_finite() || *m <= 0.0 || *m > 86_400.0)
        {
            return Err("--means must contain finite seconds in (0, 86400]".into());
        }
        if config.levels.iter().any(|&l| l >= TRAINING_LEVELS.len()) {
            return Err(format!("--levels must be in 0..{}", TRAINING_LEVELS.len()));
        }
        if !config.response.is_finite() || !(0.0..=1.0).contains(&config.response) {
            return Err("--response must be a probability in [0, 1]".into());
        }
        Ok(config)
    }
}

// Open interval (0, 1): neither ln(0) nor a zero exponential variate.
fn unit(rng: &mut SeededRng) -> f64 {
    ((rng.next_u64() >> 12) as f64 + 0.5) / ((1u64 << 52) as f64)
}

fn gamma_gap_ms(rng: &mut SeededRng, mean_seconds: f64) -> u64 {
    // Gamma(shape=2, scale=mean/2) is the sum of two independent exponentials.
    (-(unit(rng).ln() + unit(rng).ln()) * mean_seconds * 500.0).round() as u64
}

struct Player {
    habit: SeededRng,
    response: SeededRng,
    latency: SeededRng,
    mean_seconds: f64,
    next_look: u64,
    looking_until: u64,
}

impl Player {
    fn new(mean_seconds: f64, seed: u64) -> Self {
        let mut streams = SeededRng::new(seed);
        let mut habit = SeededRng::new(streams.next_u64());
        Self {
            next_look: gamma_gap_ms(&mut habit, mean_seconds).max(TICK_MS),
            habit,
            response: SeededRng::new(streams.next_u64()),
            latency: SeededRng::new(streams.next_u64()),
            mean_seconds,
            looking_until: 0,
        }
    }

    fn looking(&mut self, now: u64) -> bool {
        if now >= self.next_look {
            self.looking_until = now + GLANCE_MS;
            // Gamma gaps are onset-to-onset, not gamma plus a dwell. Leave
            // one away sample between glances so the real detector can exit.
            self.next_look = now.saturating_add(
                gamma_gap_ms(&mut self.habit, self.mean_seconds).max(GLANCE_MS + TICK_MS),
            );
        }
        now < self.looking_until
    }

    fn pulse(&mut self, now: u64, probability: f64) {
        // A natural glance may already be in progress but not yet confirmed
        // by the detector. Do not queue a redundant response to that pulse.
        if now < self.looking_until {
            return;
        }
        if unit(&mut self.response) < probability {
            let latency_ms = 400 + (unit(&mut self.latency) * 16.0).round() as u64 * TICK_MS;
            // A prompt never postpones a natural glance. The next renewal is
            // scheduled only when the glance actually starts.
            self.next_look = self.next_look.min(now + latency_ms);
        }
    }
}

struct Trial {
    metrics: GameMetrics,
    gaps: u64,
}

fn game(
    config: &Config,
    mean: f64,
    level: usize,
    cap: bool,
    seed: u64,
    deliver_pulses: bool,
) -> Result<Trial, CoreError> {
    let mut player = Player::new(mean, seed);
    // Independent policy stream: changing policy draws cannot change the
    // player's sampled habit, response decisions or response latencies.
    let mut session = Session::new(
        "synthetic",
        SessionConfig {
            density_cap: cap,
            ..SessionConfig::default()
        },
        TRAINING_LEVELS[level],
        SeededRng::new(seed ^ 0xa076_1d64_78bd_642f),
    )?;
    let mut now = 0;
    let mut gaps = 0;
    loop {
        let looking = player.looking(now);
        let update = session.observe(
            GazeSample {
                at: Timestamp(now),
                p_minimap: if looking { 1.0 } else { 0.0 },
                quality: 1.0,
            },
            GateInputs {
                active_game: true,
                alive: true,
                game_time_ms: 90_001 + now,
                foreground: true,
                face_tracked: true,
                paused: false,
            },
        )?;
        for event in update.events {
            match event.kind {
                EventKind::Pulse(_) if deliver_pulses => player.pulse(now, config.response),
                EventKind::Check(check) if check.gap_ms > 0 => gaps += 1,
                _ => {}
            }
        }
        if now == config.duration_ms {
            // Include the censored terminal gap, just as core gap statistics do.
            gaps += u64::from(update.gap_ms > 0);
            break;
        }
        now = (now + TICK_MS).min(config.duration_ms);
    }
    Ok(Trial {
        metrics: session.finish(Timestamp(now))?.metrics,
        gaps,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub mean: f64,
    pub level: usize,
    pub cap: bool,
    pub pulses_per_game: f64,
    pub prompted_gap_share: Option<f64>,
    pub unassisted_blind_share: f64,
    pub unassisted_median_gap_seconds: Option<f64>,
}

pub fn run(config: &Config) -> Result<Vec<Row>, CoreError> {
    let mut rows = Vec::new();
    for &mean in &config.means {
        for &level in &config.levels {
            // Paired counterfactual: same habit seeds, all pulse outputs discarded.
            // Scoring still uses this level's standard (not always baseline's 10s).
            // Only blind time and median gap are consumed from these sessions.
            let mut blind_ms = 0;
            let mut eligible_ms = 0;
            let mut medians = Vec::new();
            for game_index in 0..config.games {
                let seed = game_seed(config.seed, mean, game_index);
                let solo = game(config, mean, level, false, seed, false)?.metrics;
                blind_ms += solo.eligible_ms - solo.compliant_ms;
                eligible_ms += solo.eligible_ms;
                if let Some(median) = solo.gaps.median_ms {
                    medians.push(median / 1_000.0);
                }
            }
            for &cap in &config.caps {
                let mut pulses = 0;
                let mut gaps = 0;
                for game_index in 0..config.games {
                    let seed = game_seed(config.seed, mean, game_index);
                    let trial = game(config, mean, level, cap, seed, true)?;
                    pulses += trial.metrics.pulses;
                    gaps += trial.gaps;
                }
                rows.push(Row {
                    mean,
                    level,
                    cap,
                    pulses_per_game: pulses as f64 / config.games as f64,
                    prompted_gap_share: (gaps > 0).then(|| pulses as f64 / gaps as f64),
                    unassisted_blind_share: blind_ms as f64 / eligible_ms as f64,
                    unassisted_median_gap_seconds: median(&mut medians),
                });
            }
        }
    }
    Ok(rows)
}

fn game_seed(seed: u64, mean: f64, game: usize) -> u64 {
    // Key by habit and game, not CLI iteration order or selected policy.
    SeededRng::new(seed ^ mean.to_bits() ^ (game as u64).wrapping_mul(0xe703_7ed1_a0b4_28db))
        .next_u64()
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some((values[(values.len() - 1) / 2] + values[values.len() / 2]) / 2.0)
}

pub fn print_table(config: &Config, rows: &[Row]) {
    println!(
        "{} games x {:.2} eligible minutes | seed {} | response {:.0}% | gamma k=2 | 50 ms ticks",
        config.games,
        config.duration_ms as f64 / 60_000.0,
        config.seed,
        config.response * 100.0
    );
    println!(
        " mean(s) level  S(s) delay(s)    p cap  pulses/game gaps prompted solo blind solo median(s)"
    );
    for row in rows {
        let level = TRAINING_LEVELS[row.level];
        let delay = level
            .pulse_delay_ms
            .map_or_else(|| "off".into(), |ms| format!("{:.0}", ms as f64 / 1_000.0));
        let prompted = row
            .prompted_gap_share
            .map_or_else(|| "n/a".into(), |v| format!("{:.1}%", v * 100.0));
        let median = row
            .unassisted_median_gap_seconds
            .map_or_else(|| "n/a".into(), |v| format!("{v:.2}"));
        println!(
            "{:>8.2} {:>5} {:>5.0} {:>8} {:>4.2} {:>3} {:>12.1} {:>13} {:>9.1}% {:>14}",
            row.mean,
            row.level,
            level.standard_ms as f64 / 1_000.0,
            delay,
            level.pulse_probability,
            if row.cap { "on" } else { "off" },
            row.pulses_per_game,
            prompted,
            row.unassisted_blind_share * 100.0,
            median
        );
    }
    println!(
        "Solo = pulses not delivered; blind time scored at S; median of per-game solo gap medians."
    );
    println!(
        "Gaps prompted = gaps receiving a pulse / all assisted gaps (including censored ends)."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_defaults_match_the_reference_experiment() {
        assert_eq!(Config::parse([]).unwrap(), Config::default());
        let config = Config::default();
        assert_eq!(config.games, 20);
        assert_eq!(config.duration_ms, 1_800_000);
        assert_eq!(config.means, [4.0, 6.0, 8.0, 10.0, 15.0, 20.0]);
        assert_eq!(config.response, 0.8);
        assert_eq!(config.levels, [1]);
        assert_eq!(config.caps, [true]);
    }

    #[test]
    fn gamma_has_requested_mean_and_variance() {
        let mut rng = SeededRng::new(123);
        let samples: Vec<f64> = (0..100_000)
            .map(|_| gamma_gap_ms(&mut rng, 10.0) as f64 / 1_000.0)
            .collect();
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let variance =
            samples.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        assert!((mean - 10.0).abs() < 0.1, "mean {mean}");
        assert!((variance - 50.0).abs() < 1.0, "variance {variance}");
    }

    #[test]
    fn pulse_response_is_bounded_and_never_delays_a_natural_glance() {
        let mut player = Player::new(10.0, 123);
        for _ in 0..1_000 {
            player.next_look = 10_000;
            player.pulse(0, 1.0);
            assert!((400..=1_200).contains(&player.next_look));
        }
        player.next_look = 100;
        player.pulse(0, 1.0);
        assert_eq!(player.next_look, 100);
        player.next_look = 10_000;
        player.pulse(0, 0.0);
        assert_eq!(player.next_look, 10_000);
        let mut answers = 0;
        for _ in 0..10_000 {
            player.next_look = 10_000;
            player.pulse(0, 0.8);
            answers += usize::from(player.next_look < 10_000);
        }
        assert!((7_800..=8_200).contains(&answers), "answers: {answers}");
        player.looking_until = GLANCE_MS;
        player.next_look = 10_000;
        player.pulse(0, 1.0);
        assert_eq!(player.next_look, 10_000);
    }

    #[test]
    fn deterministic_and_independent_of_row_order() {
        let mut config = Config {
            games: 2,
            duration_ms: 60_000,
            means: vec![6.0, 15.0],
            caps: vec![false, true],
            ..Config::default()
        };
        let first = run(&config).unwrap();
        assert_eq!(first, run(&config).unwrap());
        config.means.reverse();
        let reversed = run(&config).unwrap();
        assert_eq!(first[..2], reversed[2..]);
        assert_eq!(first[2..], reversed[..2]);
        config.seed += 1;
        assert_ne!(first, run(&config).unwrap());
    }

    #[test]
    fn off_levels_and_zero_response_preserve_unassisted_habits() {
        let config = Config {
            games: 1,
            duration_ms: 60_000,
            means: vec![15.0],
            levels: vec![0, 9],
            ..Config::default()
        };
        for row in run(&config).unwrap() {
            assert_eq!(row.pulses_per_game, 0.0);
            assert_eq!(row.prompted_gap_share, Some(0.0));
        }
        let config = Config {
            response: 0.0,
            ..config
        };
        let assisted = game(&config, 15.0, 1, true, 123, true).unwrap();
        let solo = game(&config, 15.0, 1, false, 123, false).unwrap();
        assert_eq!(assisted.metrics.compliant_ms, solo.metrics.compliant_ms);
        assert_eq!(assisted.metrics.gaps, solo.metrics.gaps);
        assert_eq!(assisted.metrics.eligible_ms, config.duration_ms);
    }
}

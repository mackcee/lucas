use crate::{CoreError, Timestamp, types::elapsed};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Baseline,
    Ramp,
    Fade,
    Graduated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Salience {
    Strong,
    Medium,
    Soft,
    Minimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrainingLevel {
    pub phase: Phase,
    pub standard_ms: u64,
    pub pulse_delay_ms: Option<u64>,
    pub salience: Salience,
    pub pulse_probability: f64,
}

impl TrainingLevel {
    pub fn validate(self) -> Result<(), CoreError> {
        if !TRAINING_LEVELS.contains(&self) {
            return Err(CoreError::InvalidConfig("unsupported training level"));
        }
        Ok(())
    }
}

/// Manual programme only.
/// Promotion, solo-probe scheduling, and dependency decisions belong to M7.
pub const TRAINING_LEVELS: [TrainingLevel; 10] = [
    level(Phase::Baseline, 10_000, None, Salience::Strong, 0.0),
    level(Phase::Ramp, 10_000, Some(10_000), Salience::Strong, 1.0),
    level(Phase::Ramp, 8_000, Some(8_000), Salience::Strong, 1.0),
    level(Phase::Ramp, 6_000, Some(6_000), Salience::Strong, 1.0),
    level(Phase::Ramp, 5_000, Some(5_000), Salience::Strong, 1.0),
    level(Phase::Fade, 5_000, Some(6_000), Salience::Strong, 1.0),
    level(Phase::Fade, 5_000, Some(8_000), Salience::Medium, 1.0),
    level(Phase::Fade, 5_000, Some(10_000), Salience::Soft, 0.75),
    level(Phase::Fade, 5_000, Some(15_000), Salience::Minimal, 0.5),
    level(Phase::Graduated, 5_000, None, Salience::Minimal, 0.0),
];

const fn level(
    phase: Phase,
    standard_ms: u64,
    pulse_delay_ms: Option<u64>,
    salience: Salience,
    pulse_probability: f64,
) -> TrainingLevel {
    TrainingLevel {
        phase,
        standard_ms,
        pulse_delay_ms,
        salience,
        pulse_probability,
    }
}

/// Randomness is caller-owned; no OS entropy or clock is consulted by the core.
pub trait RandomSource {
    fn next_u64(&mut self) -> u64;
}

/// Reproducible SplitMix64 stream. Not suitable for cryptographic use.
#[derive(Clone, Debug)]
pub struct SeededRng(u64);
impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
}
impl RandomSource for SeededRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
}
fn unit(rng: &mut impl RandomSource) -> f64 {
    (rng.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pulse {
    pub at: Timestamp,
    pub salience: Salience,
    pub effective_delay_ms: u64,
}

#[derive(Clone, Debug)]
pub struct PulsePolicy {
    level: TrainingLevel,
    cap_enabled: bool,
    staircase_ms: u64,
    due_ms: Option<u64>,
    interval_start_ms: u64,
    last_at: Option<Timestamp>,
}

impl PulsePolicy {
    pub fn new(level: TrainingLevel, cap_enabled: bool) -> Result<Self, CoreError> {
        level.validate()?;
        Ok(Self {
            level,
            cap_enabled,
            staircase_ms: level.pulse_delay_ms.unwrap_or(0),
            due_ms: None,
            interval_start_ms: 0,
            last_at: None,
        })
    }
    pub fn level(&self) -> TrainingLevel {
        self.level
    }
    pub fn staircase_ms(&self) -> u64 {
        self.staircase_ms
    }
    pub fn effective_delay_ms(&self) -> Option<u64> {
        self.due_ms
    }

    /// Next opportunity on the eligible gap clock, for anticipation metrics.
    /// Unlike the interval duration, this includes earlier missed intervals.
    pub fn next_due_gap_ms(&self) -> Option<u64> {
        self.due_ms
            .map(|delay| self.interval_start_ms.saturating_add(delay))
    }

    fn adaptive_cap(&self) -> bool {
        // Ramp is explicit cadence training: a 5-second selection must not
        // silently grow slower after every reminder. Retain fading's cap.
        self.cap_enabled && self.level.phase == Phase::Fade
    }

    /// Explicit level changes restart the cap and current opportunity. They do
    /// not fabricate a glance or change the gap clock. No automatic promotion.
    pub fn set_level(&mut self, level: TrainingLevel) -> Result<(), CoreError> {
        level.validate()?;
        self.level = level;
        self.staircase_ms = level.pulse_delay_ms.unwrap_or(0);
        self.reset_gap();
        Ok(())
    }
    pub fn reset_gap(&mut self) {
        self.due_ms = None;
        self.interval_start_ms = 0;
    }

    pub fn on_check(&mut self, unprompted: bool) {
        if self.adaptive_cap() && unprompted {
            self.staircase_ms = self.staircase_ms.saturating_sub(250);
        }
        self.reset_gap();
    }

    /// Sample jitter once per reminder interval, not once per tick. Fade uses
    /// max(level delay, staircase delay) when capped; Ramp keeps its level delay.
    /// Each interval has one probability draw, including skipped reminders.
    pub fn prepare_gap(&mut self, rng: &mut impl RandomSource) -> Option<u64> {
        let base = self.level.pulse_delay_ms?;
        if self.due_ms.is_none() {
            let nominal = if self.adaptive_cap() {
                base.max(self.staircase_ms)
            } else {
                base
            };
            self.due_ms = Some((nominal as f64 * (0.9 + 0.2 * unit(rng))).round() as u64);
        }
        self.due_ms
    }

    pub fn tick(
        &mut self,
        at: Timestamp,
        gap_ms: u64,
        eligible: bool,
        rng: &mut impl RandomSource,
    ) -> Result<Option<Pulse>, CoreError> {
        elapsed(self.last_at, at)?;
        self.last_at = Some(at);
        if !eligible {
            return Ok(None);
        }
        let Some(delay) = self.prepare_gap(rng) else {
            return Ok(None);
        };
        if gap_ms.saturating_sub(self.interval_start_ms) < delay {
            return Ok(None);
        }
        // Start the next interval even when probability skips this one. Anchor
        // to the observed gap, so a late caller never emits catch-up bursts.
        self.interval_start_ms = gap_ms;
        self.due_ms = None;
        if unit(rng) >= self.level.pulse_probability {
            return Ok(None);
        }
        if self.adaptive_cap() {
            self.staircase_ms = self.staircase_ms.saturating_add(750);
        }
        Ok(Some(Pulse {
            at,
            salience: self.level.salience,
            effective_delay_ms: delay,
        }))
    }
}

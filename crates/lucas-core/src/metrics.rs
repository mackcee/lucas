use crate::{
    CoreError, DeathClassification, DeathEvent, GapClock, GateInputs, Glance, Pulse, Timestamp,
    types::elapsed,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub prompted_window_ms: u64,
    pub anticipation_window_ms: u64,
    pub blind_spell_ms: u64,
    pub blind_death_ms: u64,
}
impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            prompted_window_ms: 3_000,
            anticipation_window_ms: 1_000,
            blind_spell_ms: 15_000,
            blind_death_ms: 8_000,
        }
    }
}
impl MetricsConfig {
    pub fn validate(self) -> Result<(), CoreError> {
        if self.prompted_window_ms == 0
            || self.anticipation_window_ms == 0
            || self.blind_spell_ms == 0
            || self.blind_death_ms == 0
        {
            return Err(CoreError::InvalidConfig("metric windows must be positive"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GapStatistics {
    pub median_ms: Option<f64>,
    pub p80_ms: Option<f64>,
    pub p90_ms: Option<f64>,
    pub max_ms: Option<u64>,
}

/// Undefined ratios/percentiles are None, never NaN. Gap statistics include
/// positive initial and terminal censored gaps, but exclude paused/dead time
/// and sustained viewing. Percentiles use linear interpolation (type 7).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameMetrics {
    pub eligible_ms: u64,
    pub compliant_ms: u64,
    pub compliance: Option<f64>,
    pub checks: u64,
    pub checks_per_minute: Option<f64>,
    pub gaps: GapStatistics,
    pub blind_spells: u64,
    pub mean_dwell_ms: Option<f64>,
    pub pulses: u64,
    pub unprompted_checks: u64,
    pub unprompted_ratio: Option<f64>,
    pub median_pulse_to_glance_ms: Option<f64>,
    /// Anticipatory unprompted checks / all unprompted checks.
    pub anticipation_share: Option<f64>,
    pub blind_deaths: u64,
    pub seen_deaths: u64,
    pub unknown_deaths: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckEvent {
    pub at: Timestamp,
    pub gap_ms: u64,
    pub prompted: bool,
    pub anticipatory: bool,
    pub pulse_latency_ms: Option<u64>,
}

/// One timestamped state transition, normally produced by Session. Inputs use
/// left-continuous intervals: this observation takes effect at `at`, not before.
#[derive(Clone, Copy, Debug)]
pub struct MetricsObservation {
    pub at: Timestamp,
    pub gate: GateInputs,
    pub tracking: bool,
    pub looking: bool,
    pub check_started: bool,
    pub effective_delay_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct EvidenceInterval {
    start: Timestamp,
    end: Timestamp,
    tracked: bool,
    looking: bool,
}

/// In-memory event-time accumulator. It neither stores camera frames nor does
/// I/O. Retains coalesced tracking intervals so delayed death API events can be
/// classified at their original timestamp, rather than at poll arrival time.
#[derive(Clone, Debug)]
pub struct MetricsAccumulator {
    config: MetricsConfig,
    standard_ms: u64,
    clock: GapClock,
    started_at: Option<Timestamp>,
    last_at: Option<Timestamp>,
    tracking: bool,
    looking: bool,
    evidence: Vec<EvidenceInterval>,
    checks: Vec<Timestamp>,
    gaps: Vec<u64>,
    latencies: Vec<u64>,
    last_pulse: Option<Timestamp>,
    pending_pulse: Option<Timestamp>,
    dwell_ms: u64,
    dwells: u64,
    anticipatory: u64,
    totals: GameMetrics,
}

impl MetricsAccumulator {
    pub fn new(standard_ms: u64, config: MetricsConfig) -> Result<Self, CoreError> {
        config.validate()?;
        if standard_ms == 0 {
            return Err(CoreError::InvalidConfig("standard must be positive"));
        }
        Ok(Self {
            config,
            standard_ms,
            clock: GapClock::default(),
            started_at: None,
            last_at: None,
            tracking: false,
            looking: false,
            evidence: Vec::new(),
            checks: Vec::new(),
            gaps: Vec::new(),
            latencies: Vec::new(),
            last_pulse: None,
            pending_pulse: None,
            dwell_ms: 0,
            dwells: 0,
            anticipatory: 0,
            totals: GameMetrics::default(),
        })
    }
    pub fn gap_ms(&self) -> u64 {
        self.clock.gap_ms()
    }
    pub fn eligible(&self) -> bool {
        self.clock.eligible()
    }
    pub fn standard_ms(&self) -> u64 {
        self.standard_ms
    }

    /// Applies to subsequent intervals only; previously scored time is unchanged.
    pub fn set_standard(&mut self, standard_ms: u64) -> Result<(), CoreError> {
        if standard_ms == 0 {
            return Err(CoreError::InvalidConfig("standard must be positive"));
        }
        self.standard_ms = standard_ms;
        Ok(())
    }

    pub fn observe(
        &mut self,
        observation: MetricsObservation,
    ) -> Result<Option<CheckEvent>, CoreError> {
        let MetricsObservation {
            at,
            mut gate,
            tracking,
            looking,
            check_started,
            effective_delay_ms,
        } = observation;
        elapsed(self.last_at, at)?;
        let tracking = tracking && gate.face_tracked;
        gate.face_tracked = tracking;
        let looking = looking && gate.eligible();
        let interval = self.clock.update(at, gate, looking)?;
        let duration = interval.eligible_ms();
        self.totals.eligible_ms = self.totals.eligible_ms.saturating_add(duration);
        let compliant = if interval.looking {
            duration
        } else {
            duration.min(self.standard_ms.saturating_sub(interval.gap_start_ms))
        };
        self.totals.compliant_ms = self.totals.compliant_ms.saturating_add(compliant);
        if interval.eligible
            && interval.gap_start_ms <= self.config.blind_spell_ms
            && interval.gap_end_ms > self.config.blind_spell_ms
        {
            self.totals.blind_spells += 1;
        }
        if let Some(previous) = self.last_at.filter(|previous| *previous < at) {
            if let Some(last) = self.evidence.last_mut().filter(|last| {
                last.end == previous
                    && last.tracked == self.tracking
                    && last.looking == self.looking
            }) {
                last.end = at;
            } else {
                self.evidence.push(EvidenceInterval {
                    start: previous,
                    end: at,
                    tracked: self.tracking,
                    looking: self.looking,
                });
            }
        }
        let is_check = check_started && looking;
        if (looking || interval.reset_at_end) && interval.gap_end_ms > 0 {
            self.gaps.push(interval.gap_end_ms);
        }
        let check = if is_check {
            let prompted = self
                .last_pulse
                .is_some_and(|pulse| at.0 - pulse.0 <= self.config.prompted_window_ms);
            let latency = self.pending_pulse.take().map(|pulse| at.0 - pulse.0);
            if let Some(latency) = latency {
                self.latencies.push(latency);
            }
            let anticipatory = !prompted
                && effective_delay_ms.is_some_and(|delay| {
                    interval.gap_end_ms <= delay
                        && delay - interval.gap_end_ms <= self.config.anticipation_window_ms
                });
            self.totals.checks += 1;
            self.totals.unprompted_checks += u64::from(!prompted);
            self.anticipatory += u64::from(anticipatory);
            self.checks.push(at);
            Some(CheckEvent {
                at,
                gap_ms: interval.gap_end_ms,
                prompted,
                anticipatory,
                pulse_latency_ms: latency,
            })
        } else {
            None
        };
        if gate.resets_gap() {
            self.last_pulse = None;
            self.pending_pulse = None;
        }
        self.started_at.get_or_insert(at);
        self.last_at = Some(at);
        self.tracking = tracking;
        self.looking = looking;
        Ok(check)
    }

    /// Record an emitted pulse at the current observation timestamp. Multiple
    /// pulses before a check pair latency with the most recent pulse.
    pub fn record_pulse(&mut self, pulse: Pulse) -> Result<(), CoreError> {
        if self.last_at != Some(pulse.at) {
            return Err(CoreError::OutsideSession);
        }
        self.totals.pulses += 1;
        self.last_pulse = Some(pulse.at);
        self.pending_pulse = Some(pulse.at);
        Ok(())
    }
    pub fn record_glance(&mut self, glance: Glance) {
        self.dwell_ms = self.dwell_ms.saturating_add(glance.dwell_ms);
        self.dwells += 1;
    }

    /// Most means strictly more than half of the preceding threshold window.
    /// Unobserved time before session start counts as lost tracking. An exact
    /// 8 s gap is blind; a glance inside the window is seen only if coverage is
    /// sufficient. Deaths may arrive late, but never outside observed time.
    pub fn classify_death(&self, at: Timestamp) -> Result<DeathEvent, CoreError> {
        if self.started_at.is_none_or(|start| at < start) || self.last_at.is_none_or(|end| at > end)
        {
            return Err(CoreError::OutsideSession);
        }
        let window = self.config.blind_death_ms;
        let start = at.0.saturating_sub(window);
        let mut tracked = 0u64;
        let mut last_look = self
            .checks
            .iter()
            .rev()
            .find(|check| **check <= at)
            .copied();
        for interval in &self.evidence {
            if interval.start >= at {
                break;
            }
            let end = interval.end.0.min(at.0);
            let overlap = end.saturating_sub(interval.start.0.max(start));
            if interval.tracked {
                tracked = tracked.saturating_add(overlap);
            }
            if interval.looking && interval.start.0 < end {
                last_look = Some(Timestamp(last_look.map_or(end, |old| old.0.max(end))));
            }
        }
        // Include a valid minimap observation exactly at the latest timestamp.
        if self.last_at == Some(at) && self.looking {
            last_look = Some(at);
        }
        let gap_ms = last_look.map(|last| at.0 - last.0);
        let tracked_fraction = tracked as f64 / window as f64;
        let classification = if tracked_fraction < 0.5 {
            DeathClassification::Unknown
        } else if gap_ms.is_some_and(|gap| gap < window) {
            DeathClassification::Seen
        } else {
            DeathClassification::Blind
        };
        Ok(DeathEvent {
            at,
            classification,
            gap_ms,
            tracked_fraction,
        })
    }

    /// Caller owns deduplication of upstream death IDs. Session does timestamp
    /// deduplication for the single player's death stream.
    pub fn record_death(&mut self, at: Timestamp) -> Result<DeathEvent, CoreError> {
        let death = self.classify_death(at)?;
        match death.classification {
            DeathClassification::Blind => self.totals.blind_deaths += 1,
            DeathClassification::Seen => self.totals.seen_deaths += 1,
            DeathClassification::Unknown => self.totals.unknown_deaths += 1,
        }
        Ok(death)
    }

    pub fn snapshot(&self) -> GameMetrics {
        let mut result = self.totals.clone();
        result.compliance = ratio(result.compliant_ms, result.eligible_ms);
        result.checks_per_minute = ratio(result.checks, result.eligible_ms).map(|v| v * 60_000.0);
        result.unprompted_ratio = ratio(result.unprompted_checks, result.checks);
        result.anticipation_share = ratio(self.anticipatory, result.unprompted_checks);
        result.mean_dwell_ms = ratio(self.dwell_ms, self.dwells);
        let mut gaps = self.gaps.clone();
        if self.clock.gap_ms() > 0 {
            gaps.push(self.clock.gap_ms());
        }
        gaps.sort_unstable();
        result.gaps = GapStatistics {
            median_ms: percentile(&gaps, 0.5),
            p80_ms: percentile(&gaps, 0.8),
            p90_ms: percentile(&gaps, 0.9),
            max_ms: gaps.last().copied(),
        };
        let mut latencies = self.latencies.clone();
        latencies.sort_unstable();
        result.median_pulse_to_glance_ms = percentile(&latencies, 0.5);
        result
    }
}
fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}
fn percentile(sorted: &[u64], quantile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = (sorted.len() - 1) as f64 * quantile;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    Some(sorted[lower] as f64 + (sorted[upper] - sorted[lower]) as f64 * index.fract())
}

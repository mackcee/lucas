use crate::{
    CheckEvent, CoreError, DeathEvent, DetectorConfig, GameMetrics, GateInputs, GazeSample,
    GazeState, Glance, GlanceDetector, MetricsAccumulator, MetricsConfig, MetricsObservation,
    Pulse, PulsePolicy, RandomSource, Timestamp, TrainingLevel, types::elapsed,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub detector: DetectorConfig,
    pub metrics: MetricsConfig,
    pub density_cap: bool,
}
impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            detector: DetectorConfig::default(),
            metrics: MetricsConfig::default(),
            density_cap: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventKind {
    GateChanged(GateInputs),
    GazeChanged(GazeState),
    Check(CheckEvent),
    Glance(Glance),
    Pulse(Pulse),
    LevelChanged(TrainingLevel),
    Death(DeathEvent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub at: Timestamp,
    pub kind: EventKind,
}

/// Storage-neutral, versioned payload. Contains no images or per-frame gaze.
/// All timestamps share the caller's monotonic epoch; the caller owns any UTC
/// metadata and mapping between game time and this timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub schema_version: u32,
    pub id: String,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    pub config: SessionConfig,
    pub initial_level: TrainingLevel,
    pub events: Vec<SessionEvent>,
    pub metrics: GameMetrics,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionUpdate {
    pub gaze: GazeState,
    pub eligible: bool,
    pub gap_ms: u64,
    pub effective_delay_ms: Option<u64>,
    /// Only newly emitted events. A Pulse requests a single animation; hide any
    /// existing overlay when eligible becomes false or gaze becomes Minimap.
    pub events: Vec<SessionEvent>,
}

/// Clock-free composition for app and simulator callers. Feed every camera
/// sample and explicit unknown samples on gate-only ticks. Missing streams are
/// considered tracked for at most maximum_sample_gap_ms after their last frame.
/// No automatic level selection or promotion is performed.
pub struct Session<R> {
    id: String,
    config: SessionConfig,
    initial_level: TrainingLevel,
    started_at: Option<Timestamp>,
    last_at: Option<Timestamp>,
    gate: GateInputs,
    gaze: GazeState,
    detector: GlanceDetector,
    policy: PulsePolicy,
    metrics: MetricsAccumulator,
    rng: R,
    events: Vec<SessionEvent>,
}

impl<R: RandomSource> Session<R> {
    pub fn new(
        id: impl Into<String>,
        config: SessionConfig,
        level: TrainingLevel,
        rng: R,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            id: id.into(),
            detector: GlanceDetector::new(config.detector)?,
            policy: PulsePolicy::new(level, config.density_cap)?,
            metrics: MetricsAccumulator::new(level.standard_ms, config.metrics)?,
            config,
            initial_level: level,
            started_at: None,
            last_at: None,
            gate: GateInputs::default(),
            gaze: GazeState::Unknown,
            rng,
            events: Vec::new(),
        })
    }

    pub fn metrics(&self) -> GameMetrics {
        self.metrics.snapshot()
    }
    pub fn level(&self) -> TrainingLevel {
        self.policy.level()
    }
    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    /// Change at the latest observation boundary. Mid-game compliance uses the
    /// standard that applied to each interval; the event log preserves changes.
    pub fn set_level(&mut self, level: TrainingLevel) -> Result<(), CoreError> {
        level.validate()?;
        self.policy.set_level(level)?;
        self.metrics.set_standard(level.standard_ms)?;
        if let Some(at) = self.last_at {
            self.emit(at, EventKind::LevelChanged(level));
        } else {
            self.initial_level = level;
        }
        Ok(())
    }

    pub fn observe(
        &mut self,
        sample: GazeSample,
        gate: GateInputs,
    ) -> Result<SessionUpdate, CoreError> {
        let dt = elapsed(self.last_at, sample.at)?;
        if self.last_at.is_some() && dt == 0 {
            return Err(CoreError::DuplicateSample);
        }
        let first_event = self.events.len();
        self.expire_stale(sample.at)?;
        self.process(sample, gate)?;
        Ok(SessionUpdate {
            gaze: self.gaze,
            eligible: self.metrics.eligible(),
            gap_ms: self.metrics.gap_ms(),
            effective_delay_ms: self.policy.effective_delay_ms(),
            events: self.events[first_event..].to_vec(),
        })
    }

    fn expire_stale(&mut self, at: Timestamp) -> Result<(), CoreError> {
        if let Some(previous) = self.last_at
            && at.0 - previous.0 > self.config.detector.maximum_sample_gap_ms
        {
            let expiry = Timestamp(previous.0 + self.config.detector.maximum_sample_gap_ms);
            let mut gate = self.gate;
            gate.face_tracked = false;
            self.process(
                GazeSample {
                    at: expiry,
                    p_minimap: 0.0,
                    quality: 0.0,
                },
                gate,
            )?;
        }
        Ok(())
    }

    fn process(&mut self, sample: GazeSample, mut gate: GateInputs) -> Result<(), CoreError> {
        let tracking = gate.face_tracked
            && sample.quality.is_finite()
            && (self.config.detector.minimum_quality..=1.0).contains(&sample.quality)
            && sample.p_minimap.is_finite()
            && (0.0..=1.0).contains(&sample.p_minimap);
        gate.face_tracked = tracking;
        // Dwell cannot bridge loading, death, pause, or foreground loss.
        let detector_sample = if gate.eligible() {
            sample
        } else {
            GazeSample {
                quality: 0.0,
                ..sample
            }
        };
        let glance = self.detector.observe(detector_sample)?;
        let looking = glance.state == GazeState::Minimap;
        if gate.eligible() {
            self.policy.prepare_gap(&mut self.rng);
        }
        let check = self.metrics.observe(MetricsObservation {
            at: sample.at,
            gate,
            tracking,
            looking,
            check_started: glance.check_started,
            effective_delay_ms: self.policy.effective_delay_ms(),
        })?;
        let previous_gate = GateInputs {
            game_time_ms: gate.game_time_ms,
            ..self.gate
        };
        if gate != previous_gate
            || gate.resets_gap() != self.gate.resets_gap()
            || self.started_at.is_none()
        {
            self.emit(sample.at, EventKind::GateChanged(gate));
        }
        if glance.state != self.gaze || self.started_at.is_none() {
            self.emit(sample.at, EventKind::GazeChanged(glance.state));
        }
        if let Some(completed) = glance.completed_glance {
            self.metrics.record_glance(completed);
            self.emit(sample.at, EventKind::Glance(completed));
        }
        if let Some(check) = check {
            self.policy.on_check(!check.prompted);
            self.emit(sample.at, EventKind::Check(check));
        }
        if gate.resets_gap() {
            self.policy.reset_gap();
        }
        if let Some(pulse) = self.policy.tick(
            sample.at,
            self.metrics.gap_ms(),
            gate.eligible() && !looking,
            &mut self.rng,
        )? {
            self.metrics.record_pulse(pulse)?;
            self.emit(sample.at, EventKind::Pulse(pulse));
        }
        self.started_at.get_or_insert(sample.at);
        self.last_at = Some(sample.at);
        self.gate = gate;
        self.gaze = glance.state;
        Ok(())
    }

    /// Late API events are accepted and timestamp-deduplicated. Event storage is
    /// in arrival order; consumers sorting by event time should use a stable sort.
    pub fn death(&mut self, at: Timestamp) -> Result<DeathEvent, CoreError> {
        if let Some(death) = self.events.iter().find_map(|event| match &event.kind {
            EventKind::Death(death) if death.at == at => Some(death.clone()),
            _ => None,
        }) {
            return Ok(death);
        }
        let death = self.metrics.record_death(at)?;
        self.emit(at, EventKind::Death(death.clone()));
        Ok(death)
    }

    pub fn finish(mut self, at: Timestamp) -> Result<SessionRecord, CoreError> {
        elapsed(self.last_at, at)?;
        self.expire_stale(at)?;
        if self.last_at != Some(at) {
            self.process(
                GazeSample {
                    at,
                    p_minimap: 0.0,
                    quality: 0.0,
                },
                GateInputs::default(),
            )?;
        }
        if let Some(glance) = self.detector.finish_session() {
            self.metrics.record_glance(glance);
            self.emit(at, EventKind::Glance(glance));
        }
        Ok(SessionRecord {
            schema_version: 1,
            id: self.id,
            started_at: self.started_at.unwrap_or(at),
            ended_at: at,
            config: self.config,
            initial_level: self.initial_level,
            events: self.events,
            metrics: self.metrics.snapshot(),
        })
    }

    fn emit(&mut self, at: Timestamp, kind: EventKind) {
        self.events.push(SessionEvent { at, kind });
    }
}

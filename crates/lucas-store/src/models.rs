use crate::{Result, StoreError, ensure_version};
use lucas_core::{
    DeathClassification, EventKind, GameMetrics, SessionRecord, Timestamp, TrainingLevel,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenGeometry {
    /// Virtual-desktop physical pixels (origins may be negative).
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

/// Opaque serialized model weights, not training samples or camera features.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub schema_version: u32,
    pub model: Vec<u8>,
    pub screen: ScreenGeometry,
    pub created_at_unix_ms: u64,
    pub validation_error_degrees: Option<f64>,
}

impl Calibration {
    pub fn validate(&self) -> Result<()> {
        ensure_version("calibration", self.schema_version)?;
        if self.model.is_empty() || self.model.len() > 65_536 {
            return Err(StoreError::Invalid(
                "calibration weights must contain 1..=65536 bytes",
            ));
        }
        if self.screen.width == 0
            || self.screen.height == 0
            || !self.screen.scale_factor.is_finite()
            || self.screen.scale_factor <= 0.0
        {
            return Err(StoreError::Invalid("invalid calibration screen geometry"));
        }
        nonnegative(self.validation_error_degrees)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackingQuality {
    pub valid_frames: u64,
    pub total_frames: u64,
    pub calibration_error_degrees: Option<f64>,
}

impl TrackingQuality {
    pub fn valid_fraction(&self) -> Option<f64> {
        (self.total_frames > 0).then(|| self.valid_frames as f64 / self.total_frames as f64)
    }

    fn validate(&self) -> Result<()> {
        if self.valid_frames > self.total_frames {
            return Err(StoreError::Invalid("valid frames exceeds total frames"));
        }
        nonnegative(self.calibration_error_degrees)
    }
}

/// The adapter supplies game metadata; unknown role/champion stays None.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameContext {
    pub ended_at_unix_ms: u64,
    pub role: Option<String>,
    pub champion: Option<String>,
    pub length_ms: u64,
    pub probe_game: bool,
    /// False for window-only sessions: zero deaths must not imply none occurred.
    pub deaths_available: bool,
    pub tracking: TrackingQuality,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredEventKind {
    Check,
    Pulse,
    Death {
        classification: DeathClassification,
        /// Time since the last valid minimap evidence; None when no check was observed.
        /// Unknown deaths retain this as uncertain evidence, never counted as blind.
        seconds_blind: Option<f64>,
        tracked_fraction: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredEvent {
    /// Milliseconds since game start, NOT the app's monotonic clock.
    pub game_time_ms: u64,
    pub kind: StoredEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameSummary {
    pub schema_version: u32,
    pub id: String,
    pub context: GameContext,
    pub metrics: GameMetrics,
    pub initial_level: TrainingLevel,
    pub wheels: TrainingLevel,
    pub standard_changed: bool,
    pub blind_death_threshold_ms: u64,
    pub events: Vec<StoredEvent>,
}

impl GameSummary {
    /// Build at game end, using the caller's pause-aware monotonic -> game clock
    /// mapping. No default linear offset is guessed. Only checks, pulses and
    /// deaths survive; gate/gaze transitions and glances are not persisted.
    /// The adapter must finalize late deaths before finishing the core session.
    pub fn from_session(
        record: &SessionRecord,
        context: GameContext,
        mut game_time: impl FnMut(Timestamp) -> Option<u64>,
    ) -> Result<Self> {
        ensure_version("session", record.schema_version)?;
        let mut wheels = record.initial_level;
        let mut standard_changed = false;
        let mut events = Vec::new();
        for event in &record.events {
            let kind = match &event.kind {
                EventKind::Check(_) => StoredEventKind::Check,
                EventKind::Pulse(_) => StoredEventKind::Pulse,
                EventKind::Death(death) => StoredEventKind::Death {
                    classification: death.classification,
                    seconds_blind: death.gap_ms.map(|ms| ms as f64 / 1000.0),
                    tracked_fraction: death.tracked_fraction,
                },
                EventKind::LevelChanged(level) => {
                    standard_changed |= level.standard_ms != wheels.standard_ms;
                    wheels = *level;
                    continue;
                }
                _ => continue,
            };
            events.push(StoredEvent {
                game_time_ms: game_time(event.at)
                    .ok_or(StoreError::Invalid("event has no game-clock mapping"))?,
                kind,
            });
        }
        events.sort_by_key(|event| event.game_time_ms);
        let summary = Self {
            schema_version: 1,
            id: record.id.clone(),
            context,
            metrics: record.metrics.clone(),
            initial_level: record.initial_level,
            wheels,
            standard_changed,
            blind_death_threshold_ms: record.config.metrics.blind_death_ms,
            events,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<()> {
        ensure_version("summary", self.schema_version)?;
        if self.id.trim().is_empty() || self.blind_death_threshold_ms == 0 {
            return Err(StoreError::Invalid("missing game ID or death threshold"));
        }
        self.initial_level
            .validate()
            .map_err(|_| StoreError::Invalid("invalid initial level"))?;
        self.wheels
            .validate()
            .map_err(|_| StoreError::Invalid("invalid wheels level"))?;
        self.context.tracking.validate()?;
        let m = &self.metrics;
        if self.context.probe_game && m.pulses != 0 {
            return Err(StoreError::Invalid(
                "solo probes cannot contain assistance pulses",
            ));
        }
        if m.compliant_ms > m.eligible_ms || m.unprompted_checks > m.checks {
            return Err(StoreError::Invalid("inconsistent metric counts"));
        }
        for value in [m.compliance, m.unprompted_ratio, m.anticipation_share] {
            fraction(value)?;
        }
        for value in [
            m.checks_per_minute,
            m.gaps.median_ms,
            m.gaps.p80_ms,
            m.gaps.p90_ms,
            m.mean_dwell_ms,
            m.median_pulse_to_glance_ms,
        ] {
            nonnegative(value)?;
        }
        let (mut checks, mut pulses, mut blind, mut seen, mut unknown) = (0, 0, 0, 0, 0);
        for event in &self.events {
            if event.game_time_ms > self.context.length_ms {
                return Err(StoreError::Invalid("event is after game end"));
            }
            match event.kind {
                StoredEventKind::Check => checks += 1,
                StoredEventKind::Pulse => pulses += 1,
                StoredEventKind::Death {
                    classification,
                    seconds_blind,
                    tracked_fraction,
                } => {
                    nonnegative(seconds_blind)?;
                    fraction(Some(tracked_fraction))?;
                    match classification {
                        DeathClassification::Blind => blind += 1,
                        DeathClassification::Seen => seen += 1,
                        DeathClassification::Unknown => unknown += 1,
                    }
                }
            }
        }
        if (checks, pulses, blind, seen, unknown)
            != (
                m.checks,
                m.pulses,
                m.blind_deaths,
                m.seen_deaths,
                m.unknown_deaths,
            )
        {
            return Err(StoreError::Invalid("event counts do not match metrics"));
        }
        if !self.context.deaths_available && blind + seen + unknown > 0 {
            return Err(StoreError::Invalid("deaths present without death data"));
        }
        Ok(())
    }
}

fn nonnegative(value: Option<f64>) -> Result<()> {
    if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(StoreError::Invalid(
            "expected finite nonnegative measurement",
        ));
    }
    Ok(())
}
fn fraction(value: Option<f64>) -> Result<()> {
    nonnegative(value)?;
    if value.is_some_and(|v| v > 1.0) {
        return Err(StoreError::Invalid("fraction is greater than one"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProbePoint {
    pub game_id: String,
    pub ended_at_unix_ms: u64,
    pub compliance: Option<f64>,
    pub standard_ms: u64,
    pub standard_changed: bool,
    pub valid_fraction: Option<f64>,
}

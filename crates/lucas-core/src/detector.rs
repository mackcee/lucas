use crate::{CoreError, GazeSample, GazeState, Glance, Timestamp, types::elapsed};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorConfig {
    pub enter_probability: f64,
    pub exit_probability: f64,
    pub minimum_quality: f64,
    pub dwell_ms: u64,
    pub minimum_samples: u32,
    /// A missing camera stream cannot complete a dwell across an arbitrary gap.
    pub maximum_sample_gap_ms: u64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            enter_probability: 0.8,
            exit_probability: 0.6,
            minimum_quality: 0.5,
            dwell_ms: 100,
            minimum_samples: 3,
            maximum_sample_gap_ms: 250,
        }
    }
}

impl DetectorConfig {
    pub fn validate(self) -> Result<(), CoreError> {
        if ![
            self.enter_probability,
            self.exit_probability,
            self.minimum_quality,
        ]
        .into_iter()
        .all(|p| p.is_finite() && (0.0..=1.0).contains(&p))
            || self.exit_probability >= self.enter_probability
            || self.minimum_quality == 0.0
            || self.dwell_ms == 0
            || self.minimum_samples < 2
            || self.maximum_sample_gap_ms == 0
        {
            return Err(CoreError::InvalidConfig(
                "invalid dwell, quality, or hysteresis thresholds",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlanceUpdate {
    pub state: GazeState,
    /// One check per confirmed dwell; sustained viewing does not create checks.
    pub check_started: bool,
    pub completed_glance: Option<Glance>,
}

#[derive(Clone, Debug)]
pub struct GlanceDetector {
    config: DetectorConfig,
    last_at: Option<Timestamp>,
    state: GazeState,
    candidate: Option<(Timestamp, u32)>,
    confirmed_at: Option<Timestamp>,
}

impl GlanceDetector {
    pub fn new(config: DetectorConfig) -> Result<Self, CoreError> {
        config.validate()?;
        Ok(Self {
            config,
            last_at: None,
            state: GazeState::Unknown,
            candidate: None,
            confirmed_at: None,
        })
    }

    pub fn config(&self) -> DetectorConfig {
        self.config
    }

    /// All invalid probability/quality values are treated as unknown. Timestamp
    /// errors leave the detector untouched. Unknown interrupts a candidate and
    /// closes a confirmed glance at the last observed sample, not after the loss.
    pub fn observe(&mut self, sample: GazeSample) -> Result<GlanceUpdate, CoreError> {
        let dt = elapsed(self.last_at, sample.at)?;
        if self.last_at.is_some() && dt == 0 {
            return Err(CoreError::DuplicateSample);
        }
        let valid = sample.quality.is_finite()
            && (self.config.minimum_quality..=1.0).contains(&sample.quality)
            && sample.p_minimap.is_finite()
            && (0.0..=1.0).contains(&sample.p_minimap);
        let discontinuity = dt > self.config.maximum_sample_gap_ms;
        let mut completed = None;
        if !valid || discontinuity {
            completed = self.finish(self.last_at.unwrap_or(sample.at));
            self.state = GazeState::Unknown;
        }
        let mut check_started = false;
        if valid {
            if self.confirmed_at.is_some() {
                if sample.p_minimap < self.config.exit_probability {
                    completed = self.finish(sample.at);
                    self.state = GazeState::Elsewhere;
                }
            } else if sample.p_minimap >= self.config.enter_probability {
                let (start, count) = self.candidate.get_or_insert((sample.at, 0));
                *count = count.saturating_add(1);
                self.state = GazeState::Elsewhere;
                if sample.at.0 - start.0 >= self.config.dwell_ms
                    && *count >= self.config.minimum_samples
                {
                    self.confirmed_at = Some(sample.at);
                    self.state = GazeState::Minimap;
                    check_started = true;
                }
            } else {
                self.candidate = None;
                self.state = GazeState::Elsewhere;
            }
        }
        self.last_at = Some(sample.at);
        Ok(GlanceUpdate {
            state: self.state,
            check_started,
            completed_glance: completed,
        })
    }

    fn finish(&mut self, at: Timestamp) -> Option<Glance> {
        let candidate = self.candidate.take();
        let confirmed = self.confirmed_at.take();
        candidate
            .zip(confirmed)
            .map(|((start, _), confirmed_at)| Glance {
                started_at: start,
                confirmed_at,
                ended_at: at,
                dwell_ms: at.0 - start.0,
            })
    }

    /// Close at the final observed frame, never extrapolating webcam dwell.
    pub fn finish_session(&mut self) -> Option<Glance> {
        self.state = GazeState::Unknown;
        self.last_at.and_then(|at| self.finish(at))
    }
}

//! Portable gaze orchestration. Only numeric samples enter this module; no I/O,
//! camera frames, Win32 handles or wall clocks are owned by the core loop.
pub mod camera;
mod clock;
#[cfg(windows)]
pub mod desktop;
#[cfg(windows)]
pub mod native_camera;

use clock::GameClock;
use lucas_core::{
    CoreError, DeathClassification, EventKind, GateInputs, GazeState, MetricsAccumulator,
    MetricsObservation, Pulse, PulsePolicy, SeededRng, SessionConfig, SessionEvent, SessionRecord,
    TRAINING_LEVELS, Timestamp,
};
use lucas_game::{Eligibility, GamePhase, GameState, Snapshot};
use lucas_gaze::{GazeSample, calibration::CalibrationModel, decision::CalibratedDetector};
use lucas_store::{GameContext, GameSummary, Settings, TrackingQuality};
use std::time::Duration;

const STALE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationState {
    Needed,
    QuickCheck,
    Ready,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    CalibrationNeeded,
    QuickCheckNeeded,
    TrackingLost,
    Paused,
    Active,
}
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    pub status: Status,
    pub hide: bool,
    pub pulse: Option<Pulse>,
    pub gap_ms: u64,
}

struct Session {
    id: String,
    probe_game: bool,
    start: Timestamp,
    last: Timestamp,
    config: SessionConfig,
    metrics: MetricsAccumulator,
    policy: PulsePolicy,
    rng: SeededRng,
    events: Vec<SessionEvent>,
    clock: GameClock,
    champion: Option<String>,
    tracking: TrackingQuality,
    deaths_available: bool,
    unknown_deaths: Vec<lucas_store::StoredEvent>,
    pending_deaths: Vec<lucas_game::Death>,
}

/// One instance per tray process. Full calibration and each game's quick check
/// are explicit actions. A failure latches Needed until a new full fit succeeds.
pub struct Runtime {
    settings: Settings,
    game: GameState,
    session: Option<Session>,
    model: Option<CalibrationModel>,
    detector: Option<CalibratedDetector>,
    calibration: CalibrationState,
    last_sample: Option<Duration>,
    gaze: GazeState,
    previously_eligible: bool,
    eligible_since: Duration,
    sequence: u64,
    summaries: Vec<GameSummary>,
    postgame_ready: bool,
    next_game_solo: bool,
    // Survives a gate-mode switch within a match, but not a genuine game end.
    match_probe: Option<bool>,
    match_level: Option<(usize, bool)>,
}
impl Runtime {
    pub fn new(settings: Settings) -> lucas_store::Result<Self> {
        settings.validate()?;
        Ok(Self {
            settings,
            game: GameState::new(),
            session: None,
            model: None,
            detector: None,
            calibration: CalibrationState::Needed,
            last_sample: None,
            gaze: GazeState::Unknown,
            previously_eligible: false,
            eligible_since: Duration::ZERO,
            sequence: 0,
            summaries: Vec::new(),
            postgame_ready: false,
            next_game_solo: false,
            match_probe: None,
            match_level: None,
        })
    }
    pub fn calibration(&self) -> CalibrationState {
        self.calibration
    }
    pub fn game(&self) -> &GameState {
        &self.game
    }
    pub fn eligibility(&self, now: Duration) -> Eligibility {
        self.game.eligibility(now)
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    /// Settings edits never mutate the policy/metrics snapshot of an active game.
    /// Persist first, then install, so a failed disk write cannot appear applied.
    pub fn update_preferences(&mut self, settings: &Settings) -> lucas_store::Result<()> {
        settings.validate()?;
        self.settings.level_index = settings.level_index;
        self.settings.pulse = settings.pulse.clone();
        self.settings.density_cap = settings.density_cap;
        self.settings.setup_completed = settings.setup_completed;
        Ok(())
    }
    pub fn arm_solo(&mut self, armed: bool) {
        self.next_game_solo = armed;
    }
    pub fn solo_armed(&self) -> bool {
        self.next_game_solo
    }
    pub fn solo_active(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.probe_game)
    }
    pub fn active_level(&self) -> Option<lucas_core::TrainingLevel> {
        self.session.as_ref().map(|s| s.policy.level())
    }
    pub fn in_session(&self) -> bool {
        self.session.is_some()
    }
    pub fn take_summaries(&mut self) -> Vec<GameSummary> {
        std::mem::take(&mut self.summaries)
    }
    pub fn postgame_ready(&self) -> bool {
        self.postgame_ready && !self.in_session()
    }
    pub fn begin_calibration(&mut self) -> Result<bool, CoreError> {
        let quick = self.calibration == CalibrationState::QuickCheck && self.in_session();
        if !quick {
            self.install_calibration(None)?;
        }
        Ok(quick)
    }

    pub fn install_calibration(
        &mut self,
        model: Option<CalibrationModel>,
    ) -> Result<(), CoreError> {
        self.model = model.filter(|m| m.validation().passes(m.target_false_positive_rate()));
        self.calibration = if self.model.is_some() {
            CalibrationState::QuickCheck
        } else {
            CalibrationState::Needed
        };
        self.reset_detector()
    }
    pub fn revalidated(&mut self, passed: bool) -> Result<(), CoreError> {
        self.calibration =
            if passed && self.model.is_some() && self.calibration != CalibrationState::Needed {
                CalibrationState::Ready
            } else {
                CalibrationState::Needed
            };
        self.reset_detector()
    }
    fn reset_detector(&mut self) -> Result<(), CoreError> {
        if let (Some(detector), Some(at), Some(session)) =
            (&mut self.detector, self.last_sample, &mut self.session)
            && let Some(glance) = detector
                .observe(GazeSample {
                    timestamp: at + Duration::from_millis(1),
                    features: None,
                    quality: 0.0,
                })?
                .completed_glance
        {
            session.metrics.record_glance(glance);
        }
        self.detector = self
            .model
            .clone()
            .map(CalibratedDetector::new)
            .transpose()?;
        self.last_sample = None;
        self.gaze = GazeState::Unknown;
        self.previously_eligible = false;
        Ok(())
    }
    pub fn camera_wanted(&self, window_exists: bool, gaze_mode: bool, calibrating: bool) -> bool {
        gaze_mode
            && !calibrating
            && self.model.is_some()
            && self.session.is_some()
            && if self.settings.live_client_gate {
                self.game.api_available()
                    && !matches!(self.game.phase(), GamePhase::NoGame | GamePhase::Ended)
            } else {
                window_exists
            }
    }
    pub fn set_live_gate(
        &mut self,
        enabled: bool,
        at: Duration,
        unix_ms: u64,
    ) -> crate::trace::Result<()> {
        if enabled != self.settings.live_client_gate {
            self.finish(at, unix_ms)?;
            self.game.reset();
            self.settings.live_client_gate = enabled;
            self.postgame_ready = false;
        }
        Ok(())
    }
    /// Complete API polls only. Loss closes the camera and gate, but a transient
    /// outage is not proof of game end: wait for GameEnd or disappearance of the
    /// game window before opening a post-game card.
    pub fn poll(
        &mut self,
        at: Duration,
        snapshot: Option<&Snapshot>,
        unix_ms: u64,
    ) -> crate::trace::Result<()> {
        if !self.settings.live_client_gate {
            return Ok(());
        }
        let Some(snapshot) = snapshot else {
            self.game.unavailable(at)?;
            if let Some(session) = &mut self.session {
                session.clock.unavailable();
            }
            return Ok(());
        };
        let previous_phase = self.game.phase();
        let update = self.game.update(at, snapshot)?;
        if update.session_started && previous_phase != GamePhase::NoGame {
            self.match_probe = None;
            self.match_level = None;
        }
        if update.session_started
            || (self.session.is_none()
                && !matches!(self.game.phase(), GamePhase::NoGame | GamePhase::Ended))
        {
            self.finish(at, unix_ms)?;
            self.start(at, unix_ms, true)?;
        }
        if let Some(s) = &mut self.session {
            s.clock.push(at, snapshot.stats.game_time);
            if let Some(player) = snapshot.active_player() {
                s.champion = Some(player.champion_name.clone());
            }
            s.pending_deaths.extend(update.deaths);
        }
        if update.game_end {
            self.finish(at, unix_ms)?;
            self.match_probe = None;
            self.match_level = None;
            self.postgame_ready = true;
        }
        Ok(())
    }
    fn start(
        &mut self,
        at: Duration,
        unix_ms: u64,
        deaths_available: bool,
    ) -> Result<(), CoreError> {
        self.sequence += 1;
        self.postgame_ready = false;
        let (index, density_cap) = *self
            .match_level
            .get_or_insert((self.settings.level_index, self.settings.density_cap));
        let level = TRAINING_LEVELS[index];
        let config = SessionConfig {
            density_cap,
            ..SessionConfig::default()
        };
        let probe_game = *self
            .match_probe
            .get_or_insert_with(|| std::mem::take(&mut self.next_game_solo));
        self.session = Some(Session {
            probe_game,
            id: format!("{unix_ms}-{}-{}", std::process::id(), self.sequence),
            start: stamp(at),
            last: stamp(at),
            metrics: MetricsAccumulator::new(level.standard_ms, config.metrics)?,
            policy: PulsePolicy::new(level, config.density_cap)?,
            config,
            rng: SeededRng::new(unix_ms ^ self.sequence),
            events: Vec::new(),
            clock: GameClock::default(),
            champion: None,
            tracking: TrackingQuality::default(),
            deaths_available,
            unknown_deaths: Vec::new(),
            pending_deaths: Vec::new(),
        });
        if self.calibration != CalibrationState::Needed {
            self.calibration = CalibrationState::QuickCheck;
        }
        self.reset_detector()
    }
    /// Run at ~30Hz. Each acquisition is fed to the detector exactly once; a
    /// held latest frame cannot satisfy dwell. Missing streams expire at 250ms.
    pub fn tick(
        &mut self,
        now: Duration,
        input: Inputs,
        sample: Option<GazeSample>,
        unix_ms: u64,
    ) -> crate::trace::Result<Tick> {
        let previous_expiry = self.last_sample.map(|t| stamp(t + STALE));
        if !self.settings.live_client_gate && input.window_exists && self.session.is_none() {
            self.start(now, unix_ms, false)?;
        }
        if !input.window_exists && (!self.settings.live_client_gate || !self.game.api_available()) {
            self.finish(now, unix_ms)?;
            self.match_probe = None;
            self.match_level = None;
            self.postgame_ready = true;
        }
        let e = self.game.eligibility(now);
        let base = if self.settings.live_client_gate {
            GateInputs {
                active_game: e.in_game && e.summoners_rift && e.supported_mode,
                alive: e.alive,
                game_time_ms: if e.past_opening { 90_001 } else { 0 },
                foreground: input.foreground,
                face_tracked: true,
                paused: e.paused
                    || !e.fresh
                    || input.paused
                    || !input.geometry_ready
                    || !input.gaze_mode,
            }
        } else {
            GateInputs {
                active_game: input.window_exists,
                alive: true,
                game_time_ms: 90_001,
                foreground: input.foreground,
                face_tracked: true,
                paused: input.paused || !input.geometry_ready || !input.gaze_mode,
            }
        };
        let allowed = base.eligible() && self.calibration == CalibrationState::Ready;
        if !allowed && self.previously_eligible {
            self.reset_detector()?;
        }
        if allowed && !self.previously_eligible {
            self.eligible_since = now;
        }
        self.previously_eligible = allowed;
        let mut check_started = false;
        let mut completed = None;
        // Reject future, stale and replayed frames before they can alter dwell.
        if let Some(sample) = sample.filter(|s| {
            s.timestamp <= now
                && now - s.timestamp <= STALE
                && self.last_sample.is_none_or(|last| s.timestamp > last)
        }) {
            self.last_sample = Some(sample.timestamp);
            let valid = self
                .model
                .as_ref()
                .is_some_and(|m| m.predict(sample).is_some());
            if let Some(session) = &mut self.session {
                session.tracking.total_frames += 1;
                session.tracking.valid_frames += u64::from(valid);
            }
            if allowed
                && sample.timestamp >= self.eligible_since
                && let Some(detector) = &mut self.detector
            {
                let update = detector.observe(sample)?;
                self.gaze = update.state;
                check_started = update.check_started;
                completed = update.completed_glance;
            }
        }
        let fresh = self
            .last_sample
            .is_some_and(|t| now.saturating_sub(t) < STALE);
        if !fresh || !allowed {
            self.gaze = GazeState::Unknown;
        }
        let tracking = self.gaze != GazeState::Unknown;
        let gate = GateInputs {
            face_tracked: tracking,
            ..base
        };
        let looking = self.gaze == GazeState::Minimap;
        let mut pulse = None;
        let mut gap_ms = 0;
        if let Some(s) = &mut self.session {
            if !s.deaths_available {
                s.clock
                    .push(now, (stamp(now).0 - s.start.0) as f64 / 1000.0);
            }
            // Expire at the last acquisition, not the last core tick. A newly
            // arriving frame cannot retroactively fill a missing-camera span.
            let expiry = previous_expiry
                .unwrap_or(s.last)
                .max(s.last)
                .min(Timestamp(s.last.0 + STALE.as_millis() as u64));
            if stamp(now) > expiry {
                s.metrics.observe(MetricsObservation {
                    at: expiry,
                    gate: GateInputs {
                        face_tracked: false,
                        ..base
                    },
                    tracking: false,
                    looking: false,
                    check_started: false,
                    effective_delay_ms: None,
                })?;
            }
            if gate.eligible() {
                s.policy.prepare_gap(&mut s.rng);
            }
            let check = s.metrics.observe(MetricsObservation {
                at: stamp(now),
                gate,
                tracking,
                looking,
                check_started,
                effective_delay_ms: s.policy.effective_delay_ms(),
            })?;
            if let Some(glance) = completed {
                s.metrics.record_glance(glance);
            }
            if let Some(check) = check {
                s.policy.on_check(!check.prompted);
                s.events.push(SessionEvent {
                    at: stamp(now),
                    kind: EventKind::Check(check),
                });
            }
            if gate.resets_gap() {
                s.policy.reset_gap();
            }
            gap_ms = s.metrics.gap_ms();
            pulse = s.policy.tick(
                stamp(now),
                gap_ms,
                gate.eligible() && !looking && !s.probe_game,
                &mut s.rng,
            )?;
            if let Some(pulse) = pulse {
                s.metrics.record_pulse(pulse)?;
                s.events.push(SessionEvent {
                    at: stamp(now),
                    kind: EventKind::Pulse(pulse),
                });
            }
            s.last = stamp(now);
            classify_deaths(s);
        }
        let status = if self.session.is_none() {
            Status::Waiting
        } else if self.calibration == CalibrationState::Needed {
            Status::CalibrationNeeded
        } else if self.calibration == CalibrationState::QuickCheck {
            Status::QuickCheckNeeded
        } else if !base.eligible() {
            Status::Paused
        } else if !tracking {
            Status::TrackingLost
        } else {
            Status::Active
        };
        Ok(Tick {
            status,
            hide: !gate.eligible() || looking || self.solo_active(),
            pulse,
            gap_ms,
        })
    }
    /// Explicit timer fallback is still assistance. Retain its pulse timestamps
    /// so switching modes cannot make an assisted session look unassisted.
    pub fn timer_pulse(&mut self, at: Duration) -> Result<(), CoreError> {
        if let Some(s) = &mut self.session {
            if s.probe_game {
                return Ok(());
            }
            let pulse = Pulse {
                at: stamp(at),
                salience: lucas_core::Salience::Medium,
                effective_delay_ms: 10_000,
            };
            s.metrics.record_pulse(pulse)?;
            s.events.push(SessionEvent {
                at: pulse.at,
                kind: EventKind::Pulse(pulse),
            });
        }
        Ok(())
    }
    pub fn finish(&mut self, at: Duration, unix_ms: u64) -> crate::trace::Result<()> {
        let Some(mut s) = self.session.take() else {
            return Ok(());
        };
        // A final unknown closes dwell at the last acquired frame, never at the
        // game-end poll. It also makes the terminal evidence interval explicit.
        if let Some(detector) = &mut self.detector {
            let end = self
                .last_sample
                .map_or(at, |last| at.max(last + Duration::from_millis(1)));
            if let Some(glance) = detector
                .observe(GazeSample {
                    timestamp: end,
                    features: None,
                    quality: 0.0,
                })?
                .completed_glance
            {
                s.metrics.record_glance(glance);
            }
        }
        let end = stamp(at).max(s.last);
        let expiry = self
            .last_sample
            .map(|t| stamp(t + STALE))
            .unwrap_or(s.last)
            .max(s.last)
            .min(Timestamp(s.last.0 + STALE.as_millis() as u64));
        if end > expiry {
            s.metrics.observe(MetricsObservation {
                at: expiry,
                gate: GateInputs::default(),
                tracking: false,
                looking: false,
                check_started: false,
                effective_delay_ms: None,
            })?;
        }
        s.metrics.observe(MetricsObservation {
            at: end,
            gate: GateInputs::default(),
            tracking: false,
            looking: false,
            check_started: false,
            effective_delay_ms: None,
        })?;
        s.last = end;
        classify_deaths(&mut s);
        let metrics = s.metrics.snapshot();
        s.tracking.calibration_error_degrees = self
            .model
            .as_ref()
            .map(|m| m.validation().mean_error_degrees);
        let record = SessionRecord {
            schema_version: 1,
            id: s.id,
            started_at: s.start,
            ended_at: end,
            config: s.config,
            initial_level: s.policy.level(),
            events: s.events,
            metrics,
        };
        let mut summary = GameSummary::from_session(
            &record,
            GameContext {
                ended_at_unix_ms: unix_ms,
                role: None,
                champion: s.champion,
                length_ms: s.clock.length_ms(),
                probe_game: s.probe_game,
                deaths_available: s.deaths_available,
                tracking: s.tracking,
            },
            |t| s.clock.game_at(t),
        )?;
        summary.metrics.unknown_deaths += s.unknown_deaths.len() as u64;
        summary.events.extend(s.unknown_deaths);
        summary.events.sort_by_key(|e| e.game_time_ms);
        summary.validate()?;
        self.summaries.push(summary);
        self.reset_detector()?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub window_exists: bool,
    pub foreground: bool,
    pub geometry_ready: bool,
    pub gaze_mode: bool,
    pub paused: bool,
}
fn stamp(at: Duration) -> Timestamp {
    Timestamp(at.as_millis().min(u64::MAX as u128) as u64)
}
fn classify_deaths(s: &mut Session) {
    for death in std::mem::take(&mut s.pending_deaths) {
        let game_ms = (death.game_time * 1000.0).round() as u64;
        let mapped = s.clock.wall_at(game_ms);
        if mapped.is_some_and(|t| t > s.last) {
            s.pending_deaths.push(death);
            continue;
        }
        let evidence = mapped.and_then(|t| s.metrics.record_death(t).ok());
        if let Some(event) = evidence {
            s.events.push(SessionEvent {
                at: event.at,
                kind: EventKind::Death(event),
            });
        } else {
            s.unknown_deaths.push(lucas_store::StoredEvent {
                game_time_ms: game_ms,
                kind: lucas_store::StoredEventKind::Death {
                    classification: DeathClassification::Unknown,
                    seconds_blind: None,
                    tracked_fraction: 0.0,
                },
            });
        }
    }
}

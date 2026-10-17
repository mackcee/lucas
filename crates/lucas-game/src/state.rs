use crate::{ClientError, Event, Snapshot};
use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};

const PAUSE_AFTER: Duration = Duration::from_secs(2);
const STALE_AFTER: Duration = Duration::from_secs(3);
const HISTORY_LENGTH: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    SummonersRift,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GamePhase {
    #[default]
    NoGame,
    Loading,
    InGame(MapKind),
    Dead(MapKind),
    Ended,
}

/// Inputs for lucas-core, independent of its still-evolving types. The app adds
/// foreground/overlay support, face tracking, and user settings to this gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Eligibility {
    pub in_game: bool,
    pub past_opening: bool,
    pub alive: bool,
    pub summoners_rift: bool,
    pub supported_mode: bool,
    pub paused: bool,
    pub fresh: bool,
}

impl Eligibility {
    pub fn eligible(self) -> bool {
        self.in_game
            && self.past_opening
            && self.alive
            && self.summoners_rift
            && self.supported_mode
            && !self.paused
            && self.fresh
    }
}

/// Exact server event time, not poll arrival time. The core classifies an >=8 s
/// blind gap using its gaze history; this crate does not infer gaze quality.
#[derive(Clone, Debug, PartialEq)]
pub struct Death {
    pub event_id: u64,
    pub game_time: f64,
    pub killer_name: Option<String>,
    pub assisters: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateUpdate {
    pub session_started: bool,
    pub respawned: bool,
    /// GameEnd was observed for the first time (authoritative, unlike API loss).
    pub game_end: bool,
    /// Newly observed events, including neutral objective and turret events.
    pub events: Vec<Event>,
    pub deaths: Vec<Death>,
}

/// Pure state machine. All wall timestamps are elapsed monotonic time from the
/// same origin. API game time alone advances the game clock, never wall time.
///
/// API loss marks the phase Ended but retains the session's event IDs/clock, so
/// reconnecting mid-game cannot duplicate deaths. A >1 s game-clock rollback or
/// a fresh event log after GameEnd starts a new session. The API has no game ID;
/// an app that knows a new window/session opened may also call `reset` explicitly.
#[derive(Debug, Default)]
pub struct GameState {
    phase: GamePhase,
    game_time: Option<f64>,
    respawn_timer: Option<f64>,
    alive: bool,
    supported_mode: bool,
    map: Option<MapKind>,
    available: bool,
    explicit_end: bool,
    last_observation: Option<Duration>,
    last_success: Option<Duration>,
    last_advance: Option<Duration>,
    seen_events: BTreeSet<u64>,
    seen_deaths: BTreeSet<u64>,
    history: VecDeque<(Duration, f64)>,
}

impl GameState {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn phase(&self) -> GamePhase {
        self.phase
    }
    pub fn game_time(&self) -> Option<f64> {
        self.game_time
    }
    pub fn respawn_timer(&self) -> Option<f64> {
        self.respawn_timer
    }
    pub fn api_available(&self) -> bool {
        self.available
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn check_timestamp(&self, now: Duration) -> Result<(), ClientError> {
        if self
            .last_observation
            .is_some_and(|previous| now <= previous)
        {
            return Err(ClientError::InvalidData(
                "observation timestamps must increase",
            ));
        }
        Ok(())
    }

    /// Apply only complete polls. Invalid snapshots leave the state untouched.
    pub fn update(
        &mut self,
        now: Duration,
        snapshot: &Snapshot,
    ) -> Result<StateUpdate, ClientError> {
        self.check_timestamp(now)?;
        if !snapshot.stats.game_time.is_finite()
            || snapshot.stats.game_time < 0.0
            || snapshot
                .players
                .iter()
                .any(|p| !p.respawn_timer.is_finite() || p.respawn_timer < 0.0)
            || snapshot
                .events
                .events
                .iter()
                .any(|e| !e.event_time.is_finite() || e.event_time < 0.0)
        {
            return Err(ClientError::InvalidData("invalid snapshot time"));
        }
        let has_end = snapshot
            .events
            .events
            .iter()
            .any(|e| e.event_name == "GameEnd");
        let new_session = self.game_time.is_none()
            || self
                .game_time
                .is_some_and(|t| snapshot.stats.game_time + 1.0 < t)
            || (self.explicit_end && !has_end);
        let was_dead = matches!(self.phase, GamePhase::Dead(_));
        if new_session {
            self.reset();
        }
        let mut update = StateUpdate {
            session_started: new_session,
            ..StateUpdate::default()
        };
        let game_time = snapshot.stats.game_time.max(self.game_time.unwrap_or(0.0));
        if self.game_time.is_none_or(|previous| game_time > previous) {
            self.last_advance = Some(now);
        }
        // No interpolation across outages: their duration may include a pause.
        if !self.available {
            self.history.clear();
        }
        self.history.push_back((now, game_time));
        while self.history.len() > HISTORY_LENGTH {
            self.history.pop_front();
        }
        self.game_time = Some(game_time);
        self.last_observation = Some(now);
        self.last_success = Some(now);
        self.available = true;
        let player = snapshot.active_player();
        self.alive = player.is_some_and(|p| !p.is_dead);
        self.respawn_timer = player.map(|p| p.respawn_timer);
        let map = if snapshot.stats.map_number == 11 {
            MapKind::SummonersRift
        } else {
            MapKind::Other
        };
        self.map = Some(map);
        // CLASSIC is the known SR mode. Unknown strings fail closed until verified.
        self.supported_mode = snapshot.stats.game_mode.eq_ignore_ascii_case("CLASSIC");
        let mut events: Vec<_> = snapshot.events.events.iter().collect();
        events.sort_by(|a, b| {
            a.event_time
                .total_cmp(&b.event_time)
                .then(a.event_id.cmp(&b.event_id))
        });
        for event in events {
            // Four endpoints are not atomic: defer events ahead of gamestats.
            if event.event_time > game_time {
                continue;
            }
            if self.seen_events.insert(event.event_id) {
                update.events.push(event.clone());
                if event.event_name == "GameEnd" && !self.explicit_end {
                    self.explicit_end = true;
                    update.game_end = true;
                }
            }
            // Revisit old events if identity was unavailable on an earlier poll.
            if event.event_name == "ChampionKill"
                && let (Some(player), Some(victim)) = (player, event.victim_name.as_deref())
                && snapshot.is_player_victim(player, victim)
                && self.seen_deaths.insert(event.event_id)
            {
                update.deaths.push(Death {
                    event_id: event.event_id,
                    game_time: event.event_time,
                    killer_name: event.killer_name.clone(),
                    assisters: event.assisters.clone(),
                });
            }
        }
        self.phase = if self.explicit_end {
            GamePhase::Ended
        } else if game_time <= 0.0 || player.is_none() {
            GamePhase::Loading
        } else if !self.alive {
            GamePhase::Dead(map)
        } else {
            GamePhase::InGame(map)
        };
        update.respawned = !new_session && was_dead && matches!(self.phase, GamePhase::InGame(_));
        Ok(update)
    }

    /// Call on a failed poll. No automatic permissive fallback: the app may opt
    /// into window-only mode while showing a connectivity/TLS diagnostic.
    pub fn unavailable(&mut self, now: Duration) -> Result<(), ClientError> {
        self.check_timestamp(now)?;
        self.last_observation = Some(now);
        self.available = false;
        self.alive = false;
        self.respawn_timer = None;
        self.phase = if self.game_time.is_some() {
            GamePhase::Ended
        } else {
            GamePhase::NoGame
        };
        self.history.clear();
        Ok(())
    }

    /// Evaluated at every core tick, so a stopped poller cannot leave the gate on.
    pub fn eligibility(&self, now: Duration) -> Eligibility {
        let fresh = self.available
            && self
                .last_success
                .is_some_and(|t| now >= t && now - t < STALE_AFTER);
        Eligibility {
            in_game: matches!(self.phase, GamePhase::InGame(_) | GamePhase::Dead(_)),
            past_opening: self.game_time.is_some_and(|t| t > 90.0),
            alive: self.alive,
            summoners_rift: self.map == Some(MapKind::SummonersRift),
            supported_mode: self.supported_mode,
            paused: self
                .last_advance
                .is_some_and(|t| now.saturating_sub(t) > PAUSE_AFTER),
            fresh,
        }
    }

    /// Map a gaze timestamp to the recent game timeline (up to 120 successful
    /// polls). Interpolate only within a <=3 s poll interval. Beyond the latest
    /// sample, clamp to its game time for <1 s; never extrapolate through pauses.
    /// Returns None across outages, old history, or stale/future timestamps.
    /// Typical uncertainty is one poll (~1 s). Buffer checks until the next poll
    /// if interpolation rather than this conservative latest-time clamp is needed.
    pub fn game_time_at(&self, timestamp: Duration) -> Option<f64> {
        let &(first_wall, _) = self.history.front()?;
        let &(last_wall, last_game) = self.history.back()?;
        if timestamp < first_wall {
            return None;
        }
        if timestamp >= last_wall {
            return (timestamp - last_wall < Duration::from_secs(1)).then_some(last_game);
        }
        for index in 1..self.history.len() {
            let (left_wall, left_game) = self.history[index - 1];
            let (right_wall, right_game) = self.history[index];
            if timestamp <= right_wall {
                if timestamp == left_wall {
                    return Some(left_game);
                }
                if timestamp == right_wall {
                    return Some(right_game);
                }
                let span = right_wall - left_wall;
                if span > STALE_AFTER {
                    return None;
                }
                let fraction = (timestamp - left_wall).as_secs_f64() / span.as_secs_f64();
                return Some(left_game + fraction * (right_game - left_game));
            }
        }
        None
    }
}

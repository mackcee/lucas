use crate::{EventData, GameStats, Player, Snapshot};
use reqwest::{Certificate, blocking::Client, redirect::Policy};
use serde::de::DeserializeOwned;
use std::{error::Error, fmt, io::Read, time::Duration};

/// Riot's published root, not the OS/WebPKI root set. See `certs/README.md`.
pub const RIOT_ROOT_PEM: &[u8] = include_bytes!("../certs/riotgames.pem");
const BASE_URL: &str = "https://127.0.0.1:2999";
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum ClientError {
    Http(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidData(&'static str),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "Live Client request: {e}"),
            Self::Io(e) => write!(f, "Live Client response: {e}"),
            Self::Json(e) => write!(f, "Live Client JSON: {e}"),
            Self::InvalidData(message) => write!(f, "Live Client data: {message}"),
        }
    }
}
impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::InvalidData(_) => None,
        }
    }
}

pub trait LiveDataSource {
    fn poll(&mut self) -> Result<Snapshot, ClientError>;
}

/// Blocking client for a dedicated poll thread. Construction and `poll` must not
/// run on an async executor thread. Only the fixed loopback endpoint is public.
pub struct LiveClient {
    client: Client,
    base_url: String,
}

impl LiveClient {
    pub fn new() -> Result<Self, ClientError> {
        Self::with_root(BASE_URL.into(), RIOT_ROOT_PEM)
    }

    fn with_root(base_url: String, pem: &[u8]) -> Result<Self, ClientError> {
        let root = Certificate::from_pem(pem).map_err(ClientError::Http)?;
        let client = Client::builder()
            .use_rustls_tls()
            .tls_built_in_root_certs(false)
            .add_root_certificate(root)
            // Riot serves its game certificate at an IP rather than its DNS name.
            // This skips ONLY name matching, not signatures, chain or expiration.
            .danger_accept_invalid_hostnames(true)
            .https_only(true)
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_millis(300))
            .timeout(Duration::from_millis(750))
            .build()
            .map_err(ClientError::Http)?;
        Ok(Self { client, base_url })
    }

    fn get<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, ClientError> {
        let response = self
            .client
            .get(format!("{}/liveclientdata/{endpoint}", self.base_url))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(ClientError::Http)?;
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(ClientError::Io)?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(ClientError::InvalidData("response exceeds 4 MiB"));
        }
        serde_json::from_slice(&bytes).map_err(ClientError::Json)
    }
}

impl LiveDataSource for LiveClient {
    fn poll(&mut self) -> Result<Snapshot, ClientError> {
        // Keep gamestats last so its clock anchor is near poll completion.
        let active_player_name: String = self.get("activeplayername")?;
        let players: Vec<Player> = self.get("playerlist")?;
        let events: EventData = self.get("eventdata")?;
        let stats: GameStats = self.get("gamestats")?;
        if !stats.game_time.is_finite()
            || stats.game_time < 0.0
            || players
                .iter()
                .any(|p| !p.respawn_timer.is_finite() || p.respawn_timer < 0.0)
            || events
                .events
                .iter()
                .any(|e| !e.event_time.is_finite() || e.event_time < 0.0)
        {
            return Err(ClientError::InvalidData("negative or non-finite game time"));
        }
        Ok(Snapshot {
            stats,
            active_player_name,
            players,
            events,
        })
    }
}

/// Caller-driven ~1 Hz cadence, with no sleeps, threads or catch-up bursts.
/// Pass elapsed monotonic time; timestamp the returned snapshot at completion
/// when calling `GameState::update`. Failures are throttled just like successes.
pub struct Poller<S> {
    source: S,
    last_poll: Option<Duration>,
}

impl<S: LiveDataSource> Poller<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            last_poll: None,
        }
    }

    pub fn poll_if_due(&mut self, now: Duration) -> Option<Result<Snapshot, ClientError>> {
        if self
            .last_poll
            .is_some_and(|last| now.saturating_sub(last) < Duration::from_secs(1))
        {
            return None;
        }
        self.last_poll = Some(now);
        Some(self.source.poll())
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;

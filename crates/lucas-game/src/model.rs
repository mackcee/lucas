use serde::Deserialize;

/// Fields consumed from `/liveclientdata/gamestats`; new API fields are ignored.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameStats {
    pub game_mode: String,
    pub game_time: f64,
    pub map_name: String,
    pub map_number: u32,
}

/// Both the modern Riot ID and the legacy summoner name are supported.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    #[serde(default)]
    pub riot_id: String,
    #[serde(default)]
    pub riot_id_game_name: String,
    #[serde(default)]
    pub riot_id_tag_line: String,
    #[serde(default)]
    pub summoner_name: String,
    pub champion_name: String,
    pub team: String,
    pub is_dead: bool,
    pub respawn_timer: f64,
}

impl Player {
    pub(crate) fn matches_id(&self, name: &str) -> bool {
        !name.is_empty()
            && (name == self.riot_id
                || name == self.summoner_name
                || (!self.riot_id_game_name.is_empty()
                    && !self.riot_id_tag_line.is_empty()
                    && name == format!("{}#{}", self.riot_id_game_name, self.riot_id_tag_line)))
    }
}

/// Event names remain strings so a new event type cannot break polling.
/// Optional fields cover ChampionKill, DragonKill, BaronKill and TurretKilled.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct Event {
    #[serde(rename = "EventID")]
    pub event_id: u64,
    pub event_name: String,
    pub event_time: f64,
    #[serde(default)]
    pub victim_name: Option<String>,
    #[serde(default)]
    pub killer_name: Option<String>,
    #[serde(default)]
    pub assisters: Vec<String>,
    #[serde(default)]
    pub dragon_type: Option<String>,
    #[serde(default)]
    pub turret_killed: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct EventData {
    #[serde(rename = "Events")]
    pub events: Vec<Event>,
}

/// A complete subset poll. Partial/invalid polls are not applied to the state.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub stats: GameStats,
    pub active_player_name: String,
    pub players: Vec<Player>,
    pub events: EventData,
}

impl Snapshot {
    /// Resolve an unambiguous player, preferring IDs over streamer-mode names.
    pub fn active_player(&self) -> Option<&Player> {
        let mut ids = self
            .players
            .iter()
            .filter(|p| p.matches_id(&self.active_player_name));
        if let Some(player) = ids.next() {
            return ids.next().is_none().then_some(player);
        }
        if self.active_player_name.is_empty() {
            return None;
        }
        let mut champions = self
            .players
            .iter()
            .filter(|p| p.champion_name == self.active_player_name);
        let player = champions.next()?;
        champions.next().is_none().then_some(player)
    }

    pub(crate) fn is_player_victim(&self, player: &Player, victim: &str) -> bool {
        player.matches_id(victim)
            || (!victim.is_empty()
                && victim == player.champion_name
                // A real opponent ID wins over a coincidentally equal champion name.
                && !self.players.iter().any(|p| p.matches_id(victim))
                && self
                    .players
                    .iter()
                    .filter(|p| p.champion_name == victim)
                    .count()
                    == 1)
    }
}

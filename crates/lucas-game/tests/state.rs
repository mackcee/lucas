use lucas_game::{EventData, GamePhase, GameState, MapKind, Snapshot};
use std::time::Duration;

fn wall(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}
fn snapshot(time: f64) -> Snapshot {
    let mut snapshot = Snapshot {
        stats: serde_json::from_str(include_str!("fixtures/gamestats.json")).unwrap(),
        active_player_name: serde_json::from_str(include_str!("fixtures/activeplayername.json"))
            .unwrap(),
        players: serde_json::from_str(include_str!("fixtures/playerlist.json")).unwrap(),
        events: serde_json::from_str(include_str!("fixtures/eventdata.json")).unwrap(),
    };
    snapshot.stats.game_time = time;
    snapshot
}
fn scripted(time: f64) -> Snapshot {
    let mut data = snapshot(time);
    data.events =
        serde_json::from_str::<EventData>(include_str!("fixtures/events-scripted.json")).unwrap();
    data
}

#[test]
fn full_game_lifecycle_and_exact_opening_boundary() {
    let mut state = GameState::new();
    assert_eq!(state.phase(), GamePhase::NoGame);
    assert!(!state.eligibility(wall(0)).eligible());
    state.unavailable(wall(0)).unwrap();
    assert_eq!(state.phase(), GamePhase::NoGame);
    assert!(
        state
            .update(wall(1), &snapshot(0.0))
            .unwrap()
            .session_started
    );
    assert_eq!(state.phase(), GamePhase::Loading);
    assert!(!state.eligibility(wall(1)).eligible());
    state.update(wall(2), &snapshot(90.0)).unwrap();
    assert_eq!(state.phase(), GamePhase::InGame(MapKind::SummonersRift));
    assert!(!state.eligibility(wall(2)).eligible());
    state.update(wall(3), &snapshot(90.01)).unwrap();
    assert!(state.eligibility(wall(3)).eligible());
    let mut dead = snapshot(100.0);
    dead.players[0].is_dead = true;
    dead.players[0].respawn_timer = 12.5;
    state.update(wall(4), &dead).unwrap();
    assert_eq!(state.phase(), GamePhase::Dead(MapKind::SummonersRift));
    assert_eq!(state.respawn_timer(), Some(12.5));
    assert!(!state.eligibility(wall(4)).eligible());
    assert!(state.update(wall(5), &snapshot(113.0)).unwrap().respawned);
    let update = state.update(wall(6), &scripted(1800.0)).unwrap();
    assert!(update.game_end);
    assert_eq!(state.phase(), GamePhase::Ended);
    assert!(!state.eligibility(wall(6)).eligible());
    assert!(!state.update(wall(7), &scripted(1801.0)).unwrap().game_end);
    assert!(
        state
            .update(wall(8), &snapshot(0.0))
            .unwrap()
            .session_started
    );
    assert_eq!(state.phase(), GamePhase::Loading);
}

#[test]
fn other_maps_and_unverified_modes_never_enable_gate() {
    for (map, mode) in [
        (12, "ARAM"),
        (30, "CHERRY"),
        (11, "PRACTICETOOL"),
        (11, "TUTORIAL"),
        (11, "NEW_UNKNOWN_MODE"),
    ] {
        let mut data = snapshot(120.0);
        data.stats.map_number = map;
        data.stats.game_mode = mode.into();
        let mut state = GameState::new();
        state.update(wall(0), &data).unwrap();
        assert!(!state.eligibility(wall(0)).eligible());
        assert_eq!(state.eligibility(wall(0)).summoners_rift, map == 11);
    }
}

#[test]
fn pause_freezes_game_clock_and_poll_staleness_fails_closed() {
    let mut state = GameState::new();
    state.update(wall(0), &snapshot(100.0)).unwrap();
    state.update(wall(1), &snapshot(101.0)).unwrap();
    assert_eq!(state.game_time_at(Duration::from_millis(500)), Some(100.5));
    for t in 2..=5 {
        state.update(wall(t), &snapshot(101.0)).unwrap();
    }
    assert!(state.eligibility(wall(5)).paused);
    assert!(!state.eligibility(wall(5)).eligible());
    assert_eq!(state.game_time_at(Duration::from_millis(4500)), Some(101.0));
    assert_eq!(state.game_time_at(Duration::from_millis(5500)), Some(101.0));
    assert_eq!(state.game_time_at(wall(6)), None);
    assert_eq!(state.game_time(), Some(101.0));
    state.update(wall(6), &snapshot(102.0)).unwrap();
    assert!(state.eligibility(wall(6)).eligible());
    assert!(!state.eligibility(wall(9)).fresh);
    assert!(!state.eligibility(wall(5)).fresh);
}

#[test]
fn death_and_objective_events_preserve_time_and_deduplicate() {
    let mut state = GameState::new();
    let first = state.update(wall(0), &scripted(116.0)).unwrap();
    assert_eq!(first.deaths.len(), 1);
    assert_eq!(first.deaths[0].game_time, 115.5);
    assert_eq!(first.deaths[0].killer_name.as_deref(), Some("Riot Gene"));
    assert_eq!(first.deaths[0].assisters, ["Teammate"]);
    assert_eq!(first.events.len(), 3); // future events deferred
    assert!(
        state
            .update(wall(1), &scripted(117.0))
            .unwrap()
            .deaths
            .is_empty()
    );
    let next = state.update(wall(2), &scripted(1501.0)).unwrap();
    assert_eq!(next.deaths.len(), 1); // champion-name victim in streamer mode
    assert_eq!(next.deaths[0].game_time, 1400.0);
    assert_eq!(
        next.events
            .iter()
            .map(|e| e.event_name.as_str())
            .collect::<Vec<_>>(),
        [
            "DragonKill",
            "TurretKilled",
            "BaronKill",
            "ChampionKill",
            "FutureUnknownEvent"
        ]
    );
    assert_eq!(next.events[0].dragon_type.as_deref(), Some("Fire"));
    assert_eq!(
        next.events[1].turret_killed.as_deref(),
        Some("Turret_T1_L_03_A")
    );
    state.unavailable(wall(3)).unwrap();
    assert_eq!(state.phase(), GamePhase::Ended);
    assert!(!state.api_available());
    assert!(!state.eligibility(wall(3)).eligible());
    assert_eq!(state.game_time_at(wall(2)), None);
    let resumed = state.update(wall(4), &scripted(1502.0)).unwrap();
    assert!(!resumed.session_started);
    assert!(resumed.events.is_empty());
    assert!(resumed.deaths.is_empty());
    assert_eq!(state.game_time_at(wall(3)), None);
    // A game-clock reset resets event/death IDs even without a GameEnd event.
    assert!(
        state
            .update(wall(5), &scripted(116.0))
            .unwrap()
            .session_started
    );
    assert_eq!(
        state
            .update(wall(6), &scripted(1400.0))
            .unwrap()
            .deaths
            .len(),
        1
    );
}

#[test]
fn unknown_identity_is_safe_and_late_identity_recovers_deaths() {
    let mut state = GameState::new();
    let mut data = scripted(116.0);
    data.active_player_name = "spectator".into();
    assert!(state.update(wall(0), &data).unwrap().deaths.is_empty());
    assert_eq!(state.phase(), GamePhase::Loading);
    assert!(!state.eligibility(wall(0)).eligible());
    data.active_player_name = "Riot Tuxedo".into();
    assert_eq!(state.update(wall(1), &data).unwrap().deaths.len(), 1);
}

#[test]
fn modern_riot_ids_and_unambiguous_streamer_fallback() {
    let mut data = scripted(1401.0);
    data.players[0].riot_id = "Map Watcher#EUW".into();
    data.active_player_name = "Map Watcher#EUW".into();
    data.events.events[2].victim_name = Some("Map Watcher#EUW".into());
    assert_eq!(
        GameState::new()
            .update(wall(0), &data)
            .unwrap()
            .deaths
            .len(),
        2
    );
    data.players[0].riot_id.clear();
    data.players[0].riot_id_game_name = "Map Watcher".into();
    data.players[0].riot_id_tag_line = "EUW".into();
    assert!(data.active_player().is_some());
    data.active_player_name = "Annie".into();
    assert!(data.active_player().is_some());
    let mut enemy = data.players[0].clone();
    enemy.summoner_name = "Someone Else".into();
    enemy.riot_id_game_name = "Someone Else".into();
    enemy.team = "CHAOS".into();
    data.players.push(enemy);
    assert!(data.active_player().is_none()); // mirror champion is ambiguous
    data.active_player_name = "Map Watcher#EUW".into();
    assert_eq!(
        GameState::new()
            .update(wall(0), &data)
            .unwrap()
            .deaths
            .len(),
        1
    );
    data.active_player_name.clear();
    assert!(data.active_player().is_none());
}

#[test]
fn opponent_deaths_do_not_become_player_deaths() {
    let mut data = scripted(1401.0);
    data.events.events[2].victim_name = Some("Other Player".into());
    data.events.events[6].victim_name = Some("OtherChampion".into());
    assert!(
        GameState::new()
            .update(wall(0), &data)
            .unwrap()
            .deaths
            .is_empty()
    );
}

#[test]
fn out_of_order_and_invalid_data_leave_state_unchanged() {
    let mut state = GameState::new();
    state.update(wall(10), &snapshot(100.0)).unwrap();
    for t in [9, 10] {
        assert!(state.update(wall(t), &snapshot(0.0)).is_err());
        assert!(state.unavailable(wall(t)).is_err());
    }
    for time in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(state.update(wall(11), &snapshot(time)).is_err());
    }
    let mut invalid = snapshot(101.0);
    invalid.players[0].respawn_timer = -1.0;
    assert!(state.update(wall(11), &invalid).is_err());
    assert_eq!(state.game_time(), Some(100.0));
    assert!(state.eligibility(wall(10)).eligible());
}

#[test]
fn mapping_history_is_bounded_and_does_not_span_poll_outages() {
    let mut state = GameState::new();
    for t in 0..121 {
        state.update(wall(t), &snapshot(100.0 + t as f64)).unwrap();
    }
    assert_eq!(state.game_time_at(wall(0)), None);
    assert_eq!(state.game_time_at(wall(1)), Some(101.0));
    state.update(wall(130), &snapshot(230.0)).unwrap();
    assert_eq!(state.game_time_at(wall(125)), None);
    assert_eq!(state.game_time_at(wall(130)), Some(230.0));
}

#[test]
fn opponent_named_after_our_champion_does_not_trigger_streamer_fallback() {
    let mut data = scripted(1401.0);
    let mut enemy = data.players[0].clone();
    enemy.champion_name = "Lux".into();
    enemy.summoner_name = "Annie".into();
    enemy.team = "CHAOS".into();
    data.players.push(enemy);
    let deaths = GameState::new().update(wall(0), &data).unwrap().deaths;
    assert_eq!(deaths.len(), 1);
    assert_eq!(deaths[0].game_time, 115.5);
}

#[test]
fn exact_clock_samples_survive_a_long_poll_gap() {
    let mut state = GameState::new();
    state.update(wall(0), &snapshot(100.0)).unwrap();
    state.update(wall(10), &snapshot(110.0)).unwrap();
    assert_eq!(state.game_time_at(wall(0)), Some(100.0));
    assert_eq!(state.game_time_at(wall(5)), None);
    assert_eq!(state.game_time_at(wall(10)), Some(110.0));
}

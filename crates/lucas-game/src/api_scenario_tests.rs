//! Consumer-level trace: actual HTTPS polls -> game state -> combined window gate.
//! This is a scripted API and fake window, not evidence of a live Windows match.
use super::*;
use crate::{
    DisplayMode, FakeWindowProbe, GameState, ScreenRect, StateUpdate, WindowProbe, WindowStatus,
};
use serde_json::{Value, json};

fn serve_snapshot(server: &MockServer, time: f64, dead: bool) {
    let mut routes = fixtures();
    let mut stats: Value = serde_json::from_str(&routes["gamestats"].1).unwrap();
    stats["gameTime"] = json!(time);
    routes.insert("gamestats", (200, stats.to_string()));
    let mut players: Value = serde_json::from_str(&routes["playerlist"].1).unwrap();
    players[0]["isDead"] = json!(dead);
    players[0]["respawnTimer"] = json!(if dead { 15.0 } else { 0.0 });
    routes.insert("playerlist", (200, players.to_string()));
    let mut events = vec![json!({"EventID":0,"EventName":"GameStart","EventTime":0.0})];
    if time >= 99.5 {
        events.push(
            json!({"EventID":1,"EventName":"ChampionKill","EventTime":99.5,
            "VictimName":"Riot Tuxedo","KillerName":"Opponent","Assisters":[]}),
        );
    }
    if time >= 150.0 {
        events.push(json!({"EventID":2,"EventName":"GameEnd","EventTime":150.0,"Result":"Win"}));
    }
    routes.insert("eventdata", (200, json!({"Events":events}).to_string()));
    *server.routes.lock().unwrap() = routes;
}

fn observe(
    poller: &mut Poller<LiveClient>,
    state: &mut GameState,
    window: &mut FakeWindowProbe,
    seconds: u64,
    label: &str,
    expected_gate: bool,
) -> StateUpdate {
    let now = Duration::from_secs(seconds);
    let result = poller.poll_if_due(now).expect("poll is due");
    let (update, error) = match result {
        Ok(snapshot) => (state.update(now, &snapshot).unwrap(), None),
        Err(error) => {
            state.unavailable(now).unwrap();
            (StateUpdate::default(), Some(error.to_string()))
        }
    };
    // This deterministic clock represents completion time; production uses the
    // worker's actual monotonic completion timestamp.
    let window = window.sample().unwrap();
    let eligibility = state.eligibility(now);
    let gate = eligibility.eligible() && window.overlay_eligible();
    assert_eq!(gate, expected_gate, "{label}");
    assert!(
        poller
            .poll_if_due(now + Duration::from_millis(999))
            .is_none()
    );
    println!(
        "EVIDENCE {}",
        json!({
            "step":label, "wall_seconds":seconds, "game_time":state.game_time(),
            "phase":format!("{:?}", state.phase()), "api_available":state.api_available(),
            "fresh":eligibility.fresh, "paused":eligibility.paused,
            "window_eligible":window.overlay_eligible(), "coaching_gate":gate,
            "session_started":update.session_started, "respawned":update.respawned,
            "game_end":update.game_end, "deaths":update.deaths.iter().map(|death|
                json!({"event_id":death.event_id,"game_time":death.game_time})
            ).collect::<Vec<_>>(), "error":error
        })
    );
    update
}

#[test]
fn https_consumer_lifecycle_trace() {
    let server = MockServer::start(fixtures(), false);
    let mut poller = Poller::new(server.client());
    let mut state = GameState::new();
    let mut window = FakeWindowProbe {
        status: WindowStatus {
            exists: true,
            foreground: true,
            minimized: false,
            client_rect: Some(ScreenRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            }),
            display_mode: DisplayMode::Borderless,
        },
    };
    println!(
        "EVIDENCE {}",
        json!({"scenario":"M3.1 HTTPS consumer lifecycle",
        "transport":"real loopback HTTPS with generated test CA",
        "data":"synthetic lifecycle based on public Riot fixture", "window":"FakeWindowProbe",
        "scope":"gate outputs and exact death timestamps; no rendered overlay or live League session"})
    );
    for (wall, time, label, gate) in [
        (0, 0.0, "loading", false),
        (1, 90.0, "opening boundary", false),
        (2, 91.0, "alive gameplay", true),
    ] {
        serve_snapshot(&server, time, false);
        observe(&mut poller, &mut state, &mut window, wall, label, gate);
    }
    serve_snapshot(&server, 100.0, true);
    let dead = observe(
        &mut poller,
        &mut state,
        &mut window,
        3,
        "player death",
        false,
    );
    assert_eq!(dead.deaths.len(), 1);
    assert_eq!(dead.deaths[0].event_id, 1);
    assert_eq!(dead.deaths[0].game_time, 99.5);
    assert_eq!(state.respawn_timer(), Some(15.0));

    server
        .routes
        .lock()
        .unwrap()
        .insert("playerlist", (503, "{}".into()));
    observe(
        &mut poller,
        &mut state,
        &mut window,
        4,
        "API outage; no automatic window fallback",
        false,
    );
    assert!(!state.api_available());
    serve_snapshot(&server, 115.0, false);
    let resumed = observe(
        &mut poller,
        &mut state,
        &mut window,
        5,
        "respawn after outage",
        true,
    );
    assert!(resumed.respawned);
    assert!(!resumed.session_started);
    assert!(resumed.deaths.is_empty());
    assert_eq!(state.game_time_at(Duration::from_secs(4)), None);

    serve_snapshot(&server, 116.0, false);
    server
        .routes
        .lock()
        .unwrap()
        .insert("activeplayername", (200, "\"spectator\"".into()));
    observe(
        &mut poller,
        &mut state,
        &mut window,
        6,
        "unresolved identity",
        false,
    );
    serve_snapshot(&server, 117.0, false);
    assert!(
        !observe(
            &mut poller,
            &mut state,
            &mut window,
            7,
            "identity restored",
            true
        )
        .respawned
    );

    window.status.foreground = false;
    serve_snapshot(&server, 118.0, false);
    observe(&mut poller, &mut state, &mut window, 8, "alt-tab", false);
    window.status.foreground = true;
    for (wall, time, label, gate) in [
        (9, 119.0, "back in game", true),
        (10, 119.0, "clock stopped one second", true),
        (11, 119.0, "clock stopped two seconds", true),
        (12, 119.0, "pause inferred", false),
        (13, 120.0, "clock resumed", true),
    ] {
        serve_snapshot(&server, time, false);
        let update = observe(&mut poller, &mut state, &mut window, wall, label, gate);
        assert!(update.deaths.is_empty());
        assert!(!update.respawned);
    }
    let stale = state.eligibility(Duration::from_secs(16));
    assert!(!stale.fresh && !stale.eligible());
    println!(
        "EVIDENCE {}",
        json!({"step":"poller stopped", "wall_seconds":16,
        "fresh":stale.fresh, "coaching_gate":stale.eligible()})
    );
    serve_snapshot(&server, 150.0, false);
    assert!(observe(&mut poller, &mut state, &mut window, 17, "game end", false).game_end);
    serve_snapshot(&server, 0.0, false);
    assert!(
        observe(
            &mut poller,
            &mut state,
            &mut window,
            18,
            "next game loading",
            false
        )
        .session_started
    );
    serve_snapshot(&server, 100.0, true);
    let next = observe(
        &mut poller,
        &mut state,
        &mut window,
        19,
        "event IDs reusable in next game",
        false,
    );
    assert_eq!(next.deaths.len(), 1);
    assert_eq!(next.deaths[0].event_id, 1);
}

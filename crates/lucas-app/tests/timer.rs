use lucas_app::timer::{
    alignment::{Adjustment, AlignmentSession},
    alignment_store::{AlignmentStore, Key},
    report::GeometryReport,
    scheduling::{GateMode, Inputs, Status, Timer},
    settings::read_hud,
};
use lucas_game::{DisplayMode, Eligibility, ScreenRect, SettingsLocator, WindowStatus};
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};
use std::{fs, time::Duration};

fn inputs() -> Inputs {
    Inputs {
        game: Eligibility {
            in_game: true,
            past_opening: true,
            alive: true,
            summoners_rift: true,
            supported_mode: true,
            paused: false,
            fresh: true,
        },
        window: WindowStatus {
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
        geometry_ready: true,
        aligning: false,
    }
}
fn sec(s: u64) -> Duration {
    Duration::from_secs(s)
}
fn client() -> Rect {
    Rect::new(0.0, 0.0, 1920.0, 1080.0).unwrap()
}

#[test]
fn alive_ten_second_cadence_no_catch_up_bursts() {
    let mut timer = Timer::default();
    assert_eq!(timer.mode(), GateMode::LiveClient);
    assert!(!timer.tick(sec(0), inputs()).pulse);
    assert!(!timer.tick(sec(9), inputs()).pulse);
    assert!(timer.tick(sec(10), inputs()).pulse);
    assert!(!timer.tick(sec(10), inputs()).pulse);
    assert!(timer.tick(sec(99), inputs()).pulse);
    assert!(!timer.tick(sec(100), inputs()).pulse);
    assert!(!timer.tick(sec(1), inputs()).pulse);
    assert!(timer.tick(sec(11), inputs()).pulse);
}

#[test]
fn every_off_switch_hides_and_resets_the_full_interval() {
    let changes: [fn(&mut Inputs); 12] = [
        |i| i.game.alive = false,        // death
        |i| i.game.in_game = false,      // loading / game end / disconnected
        |i| i.game.past_opening = false, // loading / opening 90 seconds
        |i| i.game.summoners_rift = false,
        |i| i.game.supported_mode = false,
        |i| i.game.paused = true,
        |i| i.game.fresh = false,
        |i| i.window.foreground = false,
        |i| i.window.exists = false,
        |i| i.window.minimized = true,
        |i| i.geometry_ready = false,
        |i| i.aligning = true,
    ];
    for change in changes {
        let mut timer = Timer::default();
        timer.tick(sec(0), inputs());
        let mut off = inputs();
        change(&mut off);
        let decision = timer.tick(sec(10), off);
        assert!(!decision.pulse);
        assert!(decision.hide);
        assert!(!timer.tick(sec(11), inputs()).pulse);
        assert!(!timer.tick(sec(20), inputs()).pulse);
        assert!(timer.tick(sec(21), inputs()).pulse);
    }
}

#[test]
fn window_mode_is_explicit_and_mode_toggles_reset() {
    let mut timer = Timer::default();
    let mut input = inputs();
    input.game = Eligibility::default();
    assert!(timer.tick(sec(0), input).hide);
    timer.set_mode(GateMode::WindowOnly);
    assert_eq!(timer.tick(sec(1), input).status, Status::Active);
    assert!(timer.tick(sec(11), input).pulse);
    timer.set_mode(GateMode::LiveClient);
    assert!(timer.tick(sec(12), input).hide);
    assert!(!timer.tick(sec(13), inputs()).pulse);
    assert!(timer.tick(sec(23), inputs()).pulse);
    timer.set_paused(true);
    assert_eq!(timer.tick(sec(24), inputs()).status, Status::Paused);
    timer.set_paused(false);
    assert!(!timer.tick(sec(25), inputs()).pulse);
    assert!(timer.tick(sec(35), inputs()).pulse);
    timer.set_mode(GateMode::WindowOnly);
    for display_mode in [DisplayMode::ExclusiveFullscreen, DisplayMode::Unknown] {
        input.window.display_mode = display_mode;
        assert!(timer.tick(sec(40), input).hide);
    }
    input.window.display_mode = DisplayMode::Borderless;
    input.window.foreground = false;
    assert!(timer.tick(sec(50), input).hide);
}

#[test]
fn real_game_state_loading_death_respawn_and_end_feed_the_timer() {
    use lucas_game::{EventData, GameState, Snapshot};
    let mut snapshot = Snapshot {
        stats: serde_json::from_value(serde_json::json!({
            "gameMode": "CLASSIC", "gameTime": 0.0, "mapName": "Map11", "mapNumber": 11
        }))
        .unwrap(),
        active_player_name: "Fixture#G1".into(),
        players: serde_json::from_value(serde_json::json!([{
            "riotId": "Fixture#G1", "championName": "Fixture", "team": "ORDER",
            "isDead": false, "respawnTimer": 0.0
        }]))
        .unwrap(),
        events: EventData { events: vec![] },
    };
    let mut game = GameState::new();
    let mut timer = Timer::default();
    for second in 0..40 {
        snapshot.stats.game_time = if second == 0 {
            0.0
        } else {
            90.0 + second as f64
        };
        snapshot.players[0].is_dead = (12..=15).contains(&second);
        if second == 30 {
            snapshot.events = serde_json::from_value(serde_json::json!({"Events": [{
                "EventID": 1, "EventName": "GameEnd", "EventTime": 120.0
            }]}))
            .unwrap();
        }
        game.update(sec(second), &snapshot).unwrap();
        let input = Inputs {
            game: game.eligibility(sec(second)),
            ..inputs()
        };
        let decision = timer.tick(sec(second), input);
        assert_eq!(decision.pulse, second == 11 || second == 26, "at {second}");
        if second == 0 || (12..=15).contains(&second) || second >= 30 {
            assert!(decision.hide);
        }
    }
}

#[test]
fn shared_deletion_resets_timer_alignment_and_preserves_unrelated_files() {
    let folder = tempfile::tempdir().unwrap();
    let store = lucas_store::Store::open_in(folder.path()).unwrap();
    let key = Key::new(client(), MinimapSettings::default());
    let alignment = Alignment {
        offset_x_px: -4.0,
        offset_y_px: -6.0,
        scale_multiplier: 1.1,
    };
    let mut alignments = AlignmentStore::load(folder.path()).unwrap();
    alignments.set(key, alignment).unwrap();
    alignments.save(folder.path()).unwrap();
    drop(alignments);
    assert_eq!(
        AlignmentStore::load(folder.path()).unwrap().get(key),
        alignment
    );
    fs::copy(
        folder.path().join("alignment.toml"),
        folder.path().join("alignment.toml.tmp"),
    )
    .unwrap();
    for name in ["unrelated.txt", "alignment.toml.backup"] {
        fs::write(folder.path().join(name), "keep").unwrap();
    }

    store.delete_all_data().unwrap();

    assert!(!folder.path().join("alignment.toml").exists());
    assert!(!folder.path().join("alignment.toml.tmp").exists());
    assert_eq!(
        AlignmentStore::load(folder.path()).unwrap(),
        AlignmentStore::default()
    );
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 2);
    for name in ["unrelated.txt", "alignment.toml.backup"] {
        assert_eq!(
            fs::read_to_string(folder.path().join(name)).unwrap(),
            "keep"
        );
    }
}

#[test]
fn alignment_round_trip_replace_and_exact_profile_keying() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = AlignmentStore::load(folder.path()).unwrap();
    let settings = MinimapSettings::default();
    let key = Key::new(client(), settings);
    let alignment = Alignment {
        offset_x_px: -4.0,
        offset_y_px: -6.0,
        scale_multiplier: 1.1,
    };
    store.set(key, alignment).unwrap();
    store.save(folder.path()).unwrap();
    let loaded = AlignmentStore::load(folder.path()).unwrap();
    assert_eq!(loaded.get(key), alignment);
    // Desktop origin is deliberately not part of the profile.
    assert_eq!(
        loaded.get(Key::new(
            Rect::new(-1920.0, 100.0, 0.0, 1180.0).unwrap(),
            settings
        )),
        alignment
    );
    for other in [
        Key { width: 2560, ..key },
        Key {
            height: 1440,
            ..key
        },
        Key {
            minimap_scale: 1.0001,
            ..key
        },
        Key {
            flipped: true,
            ..key
        },
    ] {
        assert_eq!(loaded.get(other), Alignment::default());
    }
    store.set(key, Alignment::default()).unwrap();
    store.save(folder.path()).unwrap();
    assert_eq!(
        AlignmentStore::load(folder.path()).unwrap().get(key),
        Alignment::default()
    );
    assert!(AlignmentStore::decode("version = 2").is_err());
    assert!(AlignmentStore::decode("not toml!").is_err());
    assert!(
        store
            .set(
                key,
                Alignment {
                    scale_multiplier: f64::NAN,
                    ..alignment
                }
            )
            .is_err()
    );
    assert!(
        store
            .set(
                Key {
                    minimap_scale: 4.0,
                    ..key
                },
                alignment
            )
            .is_err()
    );
    let text = fs::read_to_string(folder.path().join("alignment.toml")).unwrap();
    let entry = text.split_once("[[profiles]]").unwrap().1;
    assert!(AlignmentStore::decode(&format!("{text}\n[[profiles]]{entry}")).is_err());
}

#[test]
fn alignment_nudges_resizes_and_cancel_does_not_change_store() {
    let key = Key::new(client(), MinimapSettings::default());
    let store = AlignmentStore::default();
    let mut session = AlignmentSession::new(key, store.get(key));
    let initial = minimap_rect(client(), MinimapSettings::default(), session.draft).unwrap();
    session.adjust(client(), MinimapSettings::default(), Adjustment::Left);
    session.adjust(client(), MinimapSettings::default(), Adjustment::Up);
    session.adjust(client(), MinimapSettings::default(), Adjustment::Grow);
    let changed = minimap_rect(client(), MinimapSettings::default(), session.draft).unwrap();
    assert!((changed.width() - initial.width() - 1.0).abs() < 1e-9);
    assert!((changed.right() - initial.right() + 1.0).abs() < 1e-9);
    assert_eq!(store.get(key), Alignment::default()); // Esc simply drops the draft.
    let draft = session.draft;
    session.adjust(
        client(),
        MinimapSettings {
            flipped: true,
            ..MinimapSettings::default()
        },
        Adjustment::Grow,
    );
    assert_eq!(session.draft, draft);
    for _ in 0..10000 {
        session.adjust(client(), MinimapSettings::default(), Adjustment::Shrink);
    }
    assert!(session.draft.scale_multiplier >= 0.25);
}

#[test]
fn geometry_report_contains_numbers_and_round_trips_without_screenshots() {
    let settings = MinimapSettings {
        minimap_scale: 3.0,
        flipped: true,
    };
    let correction = Alignment {
        offset_x_px: 4.0,
        ..Alignment::default()
    };
    let report = GeometryReport::new(client(), settings, correction).unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["resolution"], serde_json::json!([1920, 1080]));
    assert_eq!(value["minimap_scale"], 3.0);
    assert_eq!(value["flipped"], true);
    assert_eq!(value.as_object().unwrap().len(), 9);
    assert!((report.aligned_rect.left - report.formula_rect.left - 4.0).abs() < 1e-9);
    let folder = tempfile::tempdir().unwrap();
    let first = report.save(folder.path()).unwrap();
    let second = report.save(folder.path()).unwrap();
    assert_ne!(first, second);
    let decoded: GeometryReport =
        serde_json::from_str(&fs::read_to_string(first).unwrap()).unwrap();
    assert_eq!(decoded, report);
}

#[test]
fn settings_prefer_persisted_fallback_per_field_and_do_not_mix_installs() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for root in [first.path(), second.path()] {
        fs::create_dir(root.join("Config")).unwrap();
    }
    fs::write(first.path().join("Config/PersistedSettings.json"), r#"{"files":[{"name":"game.cfg","sections":[{"name":"HUD","settings":[{"name":"MinimapScale","value":"2"}]}]}]}"#).unwrap();
    fs::write(
        first.path().join("Config/game.cfg"),
        "[HUD]\nMinimapScale=0\nFlipMiniMap=1",
    )
    .unwrap();
    let locator = SettingsLocator::new([first.path().to_owned(), second.path().to_owned()]);
    assert_eq!(
        read_hud(&locator).unwrap(),
        MinimapSettings {
            minimap_scale: 2.0,
            flipped: true
        }
    );
    fs::remove_file(first.path().join("Config/game.cfg")).unwrap();
    fs::write(
        second.path().join("Config/game.cfg"),
        "[HUD]\nFlipMiniMap=0",
    )
    .unwrap();
    assert!(read_hud(&locator).is_err());
}

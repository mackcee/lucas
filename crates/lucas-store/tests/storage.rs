use lucas_core::{
    GateInputs, GazeSample, SeededRng, Session, SessionConfig, TRAINING_LEVELS, Timestamp,
};
use lucas_store::*;

fn synthetic_record() -> lucas_core::SessionRecord {
    record_at_level(1)
}

fn record_at_level(level: usize) -> lucas_core::SessionRecord {
    let mut session = Session::new(
        "synthetic",
        SessionConfig::default(),
        TRAINING_LEVELS[level],
        SeededRng::new(8),
    )
    .unwrap();
    // Core time has a nonzero epoch. All three deaths arrive late.
    for ms in (0..=30_000).step_by(50) {
        let sample = GazeSample {
            at: Timestamp(1_000_000 + ms),
            p_minimap: if (1_000..=1_300).contains(&ms) || (19_000..=19_300).contains(&ms) {
                1.0
            } else {
                0.0
            },
            quality: if ms >= 21_000 { 0.0 } else { 1.0 },
        };
        session
            .observe(
                sample,
                GateInputs {
                    active_game: true,
                    alive: true,
                    game_time_ms: 100_000 + ms,
                    foreground: true,
                    face_tracked: true,
                    paused: false,
                },
            )
            .unwrap();
    }
    session.death(Timestamp(1_010_000)).unwrap();
    session.death(Timestamp(1_020_000)).unwrap();
    session.death(Timestamp(1_030_000)).unwrap();
    session.finish(Timestamp(1_030_000)).unwrap()
}

fn summary() -> GameSummary {
    GameSummary::from_session(
        &synthetic_record(),
        GameContext {
            ended_at_unix_ms: 1_800_000_000_000,
            role: Some("Support".into()),
            champion: Some("Lux".into()),
            length_ms: 130_000,
            probe_game: false,
            deaths_available: true,
            tracking: TrackingQuality {
                valid_frames: 420,
                total_frames: 601,
                calibration_error_degrees: Some(2.1),
            },
        },
        |at| at.0.checked_sub(900_000),
    )
    .unwrap()
}

#[test]
fn settings_defaults_roundtrip_and_missing_nested_fields() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    assert_eq!(store.load_settings().unwrap(), Settings::default());
    let mut settings = Settings {
        camera_index: 2,
        level_index: 3,
        ..Settings::default()
    };
    settings.pulse.sound = true;
    settings.pulse.style = PulseStyle::Glow;
    store.save_settings(&settings).unwrap();
    assert_eq!(store.load_settings().unwrap(), settings);
    store.save_settings(&Settings::default()).unwrap(); // atomic replacement also on Windows
    std::fs::write(
        dir.path().join("settings.toml"),
        "camera_index = 4\nfuture_key = true\n[pulse]\nsound = true\n",
    )
    .unwrap();
    let loaded = store.load_settings().unwrap();
    assert_eq!(loaded.camera_index, 4);
    assert_eq!(loaded.pulse.colour_rgb, [255, 191, 0]);
    assert!(loaded.pulse.sound && loaded.live_client_gate);
    assert_eq!(loaded.level_index, 1);
    std::fs::write(dir.path().join("settings.toml"), "schema_version = 999\n").unwrap();
    assert!(matches!(
        store.load_settings(),
        Err(StoreError::Schema { .. })
    ));
    std::fs::write(dir.path().join("settings.toml"), "pulse = [").unwrap();
    assert!(store.load_settings().is_err());
}

#[test]
fn calibration_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    assert_eq!(store.load_calibration().unwrap(), None);
    let mut calibration = Calibration {
        schema_version: 1,
        model: vec![42; 1024],
        created_at_unix_ms: 123456,
        screen: ScreenGeometry {
            left: -1920,
            top: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.5,
        },
        validation_error_degrees: Some(2.3),
    };
    store.save_calibration(&calibration).unwrap();
    assert_eq!(store.load_calibration().unwrap(), Some(calibration.clone()));
    calibration.screen.width = 2560;
    store.save_calibration(&calibration).unwrap();
    assert_eq!(store.load_calibration().unwrap(), Some(calibration.clone()));
    calibration.validation_error_degrees = Some(f64::NAN);
    assert!(store.save_calibration(&calibration).is_err());
}

#[test]
fn empty_migration_reopens_and_rejects_future_version() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open_in(dir.path()).unwrap());
    let store = Store::open_in(dir.path()).unwrap();
    assert!(store.latest_summary().unwrap().is_none());
    assert!(store.probe_trend().unwrap().is_empty());
    drop(store);
    let connection = rusqlite::Connection::open(dir.path().join(DATABASE_NAME)).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .unwrap(),
        1
    );
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    assert!(matches!(
        Store::open_in(dir.path()),
        Err(StoreError::Schema { version: 99, .. })
    ));
}

#[test]
fn synthetic_session_keeps_only_checks_pulses_deaths_in_game_time() {
    let summary = summary();
    assert_eq!(
        (
            summary.metrics.blind_deaths,
            summary.metrics.seen_deaths,
            summary.metrics.unknown_deaths
        ),
        (1, 1, 1)
    );
    assert!(
        summary
            .events
            .iter()
            .all(|event| (100_000..=130_000).contains(&event.game_time_ms))
    );
    let deaths: Vec<_> = summary
        .events
        .iter()
        .filter_map(|event| match event.kind {
            StoredEventKind::Death {
                classification,
                seconds_blind,
                ..
            } => Some((event.game_time_ms, classification, seconds_blind)),
            _ => None,
        })
        .collect();
    assert_eq!(deaths[0].0, 110_000);
    assert_eq!(deaths[0].1, lucas_core::DeathClassification::Blind);
    assert!(deaths[0].2.unwrap() >= 8.0);
    assert_eq!(deaths[1].1, lucas_core::DeathClassification::Seen);
    assert_eq!(deaths[2].1, lucas_core::DeathClassification::Unknown);
    assert!(
        GameSummary::from_session(&synthetic_record(), summary.context.clone(), |_| None).is_err()
    );
}

#[test]
fn summary_events_queries_and_idempotent_upsert() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in(dir.path()).unwrap();
    let first = summary();
    store.save_summary(&first).unwrap();
    store.save_summary(&first).unwrap();
    assert_eq!(store.summary(&first.id).unwrap(), Some(first.clone()));
    assert_eq!(store.events(&first.id).unwrap(), first.events);
    assert!(store.summary("missing").unwrap().is_none());
    assert!(store.events("missing").unwrap().is_empty());
    let mut probe = GameSummary::from_session(&record_at_level(0), first.context.clone(), |at| {
        at.0.checked_sub(900_000)
    })
    .unwrap();
    probe.id = "probe-2".into();
    probe.context.probe_game = true;
    probe.context.ended_at_unix_ms += 100;
    store.save_summary(&probe).unwrap();
    let mut earlier = probe.clone();
    earlier.id = "probe-1".into();
    earlier.context.ended_at_unix_ms -= 50;
    store.save_summary(&earlier).unwrap();
    drop(store);
    let mut store = Store::open_in(dir.path()).unwrap();
    assert_eq!(store.latest_summary().unwrap(), Some(probe.clone()));
    let points = store.probe_trend().unwrap();
    assert_eq!(
        points
            .iter()
            .map(|p| p.game_id.as_str())
            .collect::<Vec<_>>(),
        ["probe-1", "probe-2"]
    );
    assert_eq!(points[1].compliance, probe.metrics.compliance);
    let mut invalid = probe.clone();
    invalid.events.clear();
    assert!(store.save_summary(&invalid).is_err());
    assert_eq!(store.summary(&probe.id).unwrap(), Some(probe));
}

#[test]
fn assisted_game_cannot_be_mislabeled_as_a_solo_probe() {
    let mut game = summary();
    assert!(game.metrics.pulses > 0);
    game.context.probe_game = true;
    assert!(game.validate().is_err());
}

#[test]
fn integer_overflow_rolls_back_without_losing_existing_summary() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in(dir.path()).unwrap();
    let first = summary();
    store.save_summary(&first).unwrap();
    let mut invalid = first.clone();
    invalid.context.length_ms = u64::MAX;
    invalid.events.last_mut().unwrap().game_time_ms = u64::MAX;
    assert!(store.save_summary(&invalid).is_err());
    assert_eq!(store.latest_summary().unwrap(), Some(first));
}

#[test]
fn delete_all_closes_handles_and_removes_owned_data_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in(dir.path()).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    store.save_summary(&summary()).unwrap();
    for name in [
        "calibration.json",
        ".lucas-write-interrupted",
        "games.sqlite3-wal",
        "games.sqlite3-shm",
        "games.sqlite3-journal",
    ] {
        std::fs::write(dir.path().join(name), "fixture").unwrap();
    }
    for name in ["traces", "reports"] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
        std::fs::write(dir.path().join(name).join("nested.txt"), "fixture").unwrap();
    }
    std::fs::write(dir.path().join("unrelated.txt"), "keep").unwrap();
    store.delete_all_data().unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(dir.path().join("unrelated.txt").exists());
    let store = Store::open_in(dir.path()).unwrap();
    assert_eq!(store.load_settings().unwrap(), Settings::default());
    assert!(store.latest_summary().unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn delete_does_not_follow_trace_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    std::fs::write(external.path().join("keep"), "safe").unwrap();
    std::os::unix::fs::symlink(external.path(), dir.path().join("traces")).unwrap();
    Store::open_in(dir.path())
        .unwrap()
        .delete_all_data()
        .unwrap();
    assert!(external.path().join("keep").exists());
}

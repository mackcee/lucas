use lucas_app::{
    calibration::{self, CalibrationIssue, CalibrationScript, fit_session, observation},
    preferences::{self, Setup},
    runtime::{
        CalibrationState, Inputs, Runtime, Status,
        calibration_status::{CalibrationOutcome, CalibrationStatus, Geometry, GeometrySource},
    },
    timer::alignment_store::{AlignmentStore, Key},
};
use lucas_gaze::{GazeFeatures, GazeSample};
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};
use lucas_store::{Calibration, ScreenGeometry, Settings, Store};
use std::time::Duration;

// Captured PC geometry, but synthetic gaze observations: no private fitted
// weights/webcam data. The game had ended before the 020 inspection, so this
// reproduces the confirmed mandatory-gate behavior, not an unobserved failure code.
fn geometry(fallback: bool) -> Geometry {
    let screen = ScreenGeometry {
        left: 0,
        top: 0,
        width: 3840,
        height: 2160,
        scale_factor: 1.5,
    };
    let settings = MinimapSettings {
        minimap_scale: if fallback { 1.0 } else { 2.1 },
        flipped: false,
    };
    let rect = minimap_rect(
        Rect::new(0.0, 0.0, 3840.0, 2160.0).unwrap(),
        settings,
        if fallback {
            Alignment::default()
        } else {
            correction()
        },
    )
    .unwrap();
    Geometry {
        screen,
        minimap: [
            rect.left() / 3840.0,
            rect.top() / 2160.0,
            rect.right() / 3840.0,
            rect.bottom() / 2160.0,
        ],
    }
}
fn correction() -> Alignment {
    Alignment {
        offset_x_px: 5.0,
        offset_y_px: 2.0,
        scale_multiplier: 0.9872007536196268,
    }
}
fn sample(ms: u64, [x, y]: [f64; 2]) -> GazeSample {
    GazeSample {
        timestamp: Duration::from_millis(ms),
        quality: 0.9,
        features: Some(GazeFeatures::UniGaze([
            y as f32, x as f32, 0.2, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 650.0,
        ])),
    }
}
fn profile() -> Calibration {
    let mut alignment = AlignmentStore::default();
    alignment
        .set(
            Key {
                width: 3840,
                height: 2160,
                minimap_scale: 2.1,
                flipped: false,
            },
            correction(),
        )
        .unwrap();
    let script = CalibrationScript::new(
        [3840, 2160],
        MinimapSettings {
            minimap_scale: 2.1,
            flipped: false,
        },
        &alignment,
        [598.0, 336.0],
        650.0,
    )
    .unwrap();
    let rows: Vec<_> = script
        .targets()
        .iter()
        .flat_map(|t| (0..30).map(move |i| observation(t.target, sample(i, t.target.position))))
        .collect();
    calibration::profile(
        &fit_session(&script, &rows).unwrap(),
        geometry(false).screen,
        1,
    )
    .unwrap()
}
fn settings() -> Settings {
    Settings {
        live_client_gate: false,
        ..Settings::default()
    }
}
fn inputs() -> Inputs {
    Inputs {
        window_exists: true,
        foreground: true,
        geometry_ready: true,
        gaze_mode: true,
        paused: false,
    }
}
fn update(
    gate: &mut CalibrationStatus,
    rt: &mut Runtime,
    store: &Store,
    g: Geometry,
    source: GeometrySource,
) -> bool {
    let update = gate.update_geometry(g, source);
    if update.reload {
        gate.reload(store, rt).unwrap();
    }
    update.reload
}
fn tick(rt: &mut Runtime, ms: u64) -> lucas_app::runtime::Tick {
    rt.tick(
        Duration::from_millis(ms),
        inputs(),
        Some(sample(ms, [0.5, 0.5])),
        ms,
    )
    .unwrap()
}

#[test]
fn calibrate_before_game_then_coach_automatically_in_every_matching_game() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let mut gate = CalibrationStatus::default();
    let mut rt = Runtime::new(settings()).unwrap();
    update(
        &mut gate,
        &mut rt,
        &store,
        geometry(false),
        GeometrySource::Provisional,
    );
    assert_eq!(rt.calibration(), CalibrationState::Needed);
    // Real setup persistence contract, before any game exists.
    let mut setup = Setup::new(settings());
    for _ in 0..3 {
        assert!(setup.advance());
    }
    setup.calibrated(profile());
    assert!(setup.advance());
    assert!(setup.advance());
    setup.finish(&store).unwrap();
    rt.update_preferences(&store.load_settings().unwrap())
        .unwrap();
    assert!(!preferences::needs_setup(rt.settings()));
    let saved_before = std::fs::read(dir.path().join("calibration.json")).unwrap();
    gate.reload(&store, &mut rt).unwrap();
    assert_eq!(rt.calibration(), CalibrationState::Ready);
    let mut evidence = Vec::new();
    // First game, next game, then a restarted tray: no test/revalidation call.
    for game in 0..3 {
        if game == 2 {
            rt = Runtime::new(store.load_settings().unwrap()).unwrap();
            gate = CalibrationStatus::default();
            update(
                &mut gate,
                &mut rt,
                &store,
                geometry(false),
                GeometrySource::Provisional,
            );
        }
        update(
            &mut gate,
            &mut rt,
            &store,
            geometry(false),
            GeometrySource::LiveGame,
        );
        let start = 1 + game * 40_000;
        let mut pulses = 0;
        for offset in (0..30_000).step_by(50) {
            assert!(!update(
                &mut gate,
                &mut rt,
                &store,
                geometry(false),
                GeometrySource::LiveGame
            ));
            let output = tick(&mut rt, start + offset);
            assert_eq!(rt.calibration(), CalibrationState::Ready);
            assert_eq!(output.status, Status::Active);
            pulses += usize::from(output.pulse.is_some());
        }
        assert!(
            pulses > 0,
            "coaching must actually run without a per-game test"
        );
        evidence.push(serde_json::json!({"game":game,"status":"Active","pulses":pulses}));
        let end = start + 30_000;
        rt.tick(Duration::from_millis(end), Inputs::default(), None, end)
            .unwrap();
        assert!(!rt.in_session());
    }
    assert_eq!(
        std::fs::read(dir.path().join("calibration.json")).unwrap(),
        saved_before
    );
    if let Some(directory) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("calibrate-once-runtime.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "synthetic_not_webcam_or_G2_acceptance":true,
                "setup_saved_before_game":true,
                "per_game_test_calls":0,
                "calibration_file_unchanged":true,
                "games":evidence,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn provisional_rejection_recovers_automatically_at_matching_live_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let mut gate = CalibrationStatus::default();
    let mut rt = Runtime::new(settings()).unwrap();
    update(
        &mut gate,
        &mut rt,
        &store,
        geometry(true),
        GeometrySource::Provisional,
    );
    assert_eq!(rt.calibration(), CalibrationState::Needed);
    assert_eq!(
        gate.needed_message(),
        "Calibration needed — screen or minimap changed"
    );
    assert!(update(
        &mut gate,
        &mut rt,
        &store,
        geometry(false),
        GeometrySource::LiveGame
    ));
    assert_eq!(rt.calibration(), CalibrationState::Ready);
    assert!(gate.issue().is_none());
    assert_eq!(tick(&mut rt, 1).status, Status::Active);
}

#[test]
fn failed_cancelled_stale_or_unavailable_optional_tests_never_revoke_a_valid_fit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let saved_before = std::fs::read(dir.path().join("calibration.json")).unwrap();
    for (outcome, same, hint) in [
        (Some(CalibrationOutcome::Failed), true, "Test did not pass"),
        (Some(CalibrationOutcome::Cancelled), true, "Test cancelled"),
        (Some(CalibrationOutcome::Passed), true, "Test passed"),
        (Some(CalibrationOutcome::Passed), false, "Test interrupted"),
        (Some(CalibrationOutcome::Changed), true, "Test interrupted"),
        (None, true, "Test unavailable"),
    ] {
        let mut gate = CalibrationStatus::default();
        let mut rt = Runtime::new(settings()).unwrap();
        update(
            &mut gate,
            &mut rt,
            &store,
            geometry(false),
            GeometrySource::LiveGame,
        );
        assert_eq!(tick(&mut rt, 1).status, Status::Active);
        rt.reset_gaze().unwrap();
        assert!(!rt.camera_wanted(true, true, true));
        let paused = rt
            .tick(
                Duration::from_millis(500),
                Inputs {
                    paused: true,
                    ..inputs()
                },
                None,
                500,
            )
            .unwrap();
        assert_eq!(paused.status, Status::Paused);
        assert!(paused.hide && paused.pulse.is_none());
        let notice = gate.finish_test(&mut rt, outcome, same).unwrap();
        assert!(notice.starts_with(hint), "{notice}");
        assert_eq!(rt.calibration(), CalibrationState::Ready);
        assert!(gate.issue().is_none());
        let resumed = tick(&mut rt, 1000);
        assert_eq!(resumed.status, Status::Active);
        assert_eq!(
            resumed.gap_ms, 0,
            "test time must not advance the reminder clock"
        );
        assert!(resumed.pulse.is_none());
        assert!(rt.camera_wanted(true, true, false));
        for ms in (1050..2000).step_by(50) {
            assert_eq!(tick(&mut rt, ms).status, Status::Active);
        }
    }
    assert_eq!(
        std::fs::read(dir.path().join("calibration.json")).unwrap(),
        saved_before
    );
}

#[test]
fn only_a_real_geometry_mismatch_disables_the_fit_and_test_success_cannot_override_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let mut gate = CalibrationStatus::default();
    let mut rt = Runtime::new(settings()).unwrap();
    update(
        &mut gate,
        &mut rt,
        &store,
        geometry(false),
        GeometrySource::LiveGame,
    );
    tick(&mut rt, 1);
    // Applies during an optional test as well as during ordinary play.
    update(
        &mut gate,
        &mut rt,
        &store,
        geometry(true),
        GeometrySource::LiveGame,
    );
    assert_eq!(rt.calibration(), CalibrationState::Needed);
    assert!(!rt.camera_wanted(true, true, false));
    assert!(matches!(
        gate.issue(),
        Some(CalibrationIssue::GeometryChanged(_))
    ));
    gate.finish_test(&mut rt, Some(CalibrationOutcome::Passed), false)
        .unwrap();
    for ms in 2..100 {
        assert!(!update(
            &mut gate,
            &mut rt,
            &store,
            geometry(true),
            GeometrySource::LiveGame
        ));
        let output = tick(&mut rt, ms);
        assert_eq!(output.status, Status::CalibrationNeeded);
        assert!(output.hide && output.pulse.is_none());
    }
    // Restoring genuinely matching geometry needs no new fit or ritual.
    assert!(update(
        &mut gate,
        &mut rt,
        &store,
        geometry(false),
        GeometrySource::LiveGame
    ));
    assert_eq!(tick(&mut rt, 100).status, Status::Active);
}

#[test]
fn closing_recalibration_or_setup_uses_the_still_valid_saved_profile() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let mut gate = CalibrationStatus::default();
    let mut rt = Runtime::new(settings()).unwrap();
    update(
        &mut gate,
        &mut rt,
        &store,
        geometry(false),
        GeometrySource::LiveGame,
    );
    tick(&mut rt, 1);
    rt.reset_gaze().unwrap();
    rt.tick(
        Duration::from_millis(100),
        Inputs {
            paused: true,
            ..inputs()
        },
        None,
        100,
    )
    .unwrap();
    // The desktop reloads the actual saved file on full-calibration/setup exit,
    // including cancellation, instead of revoking it based on a UI result.
    gate.reload(&store, &mut rt).unwrap();
    assert_eq!(tick(&mut rt, 200).status, Status::Active);
    std::fs::remove_file(dir.path().join("calibration.json")).unwrap();
    gate.reload(&store, &mut rt).unwrap();
    assert_eq!(gate.issue(), Some(&CalibrationIssue::MissingProfile));
    assert_eq!(tick(&mut rt, 300).status, Status::CalibrationNeeded);
}

#[test]
fn explained_loader_distinguishes_missing_unreadable_model_and_geometry_and_keeps_exact_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let g = geometry(false);
    assert_eq!(
        calibration::load_explained(&store, None).unwrap_err(),
        CalibrationIssue::MissingProfile
    );
    std::fs::write(dir.path().join("calibration.json"), b"invalid JSON").unwrap();
    assert!(matches!(
        calibration::load_explained(&store, None),
        Err(CalibrationIssue::ReadError(_))
    ));
    let mut old = profile();
    old.model[7] = b'1';
    store.save_calibration(&old).unwrap();
    let error = calibration::load_explained(&store, None).unwrap_err();
    assert!(matches!(error, CalibrationIssue::ModelError(_)));
    assert!(
        error
            .details()
            .iter()
            .any(|row| row.contains("unsupported or corrupt"))
    );
    store.save_calibration(&profile()).unwrap();
    let mut screen = g.screen.clone();
    screen.scale_factor = 1.0;
    let error = calibration::load_explained(&store, Some((&screen, g.minimap))).unwrap_err();
    assert_eq!(error.details()[1], "Display scale: saved 1.5, current 1");
    let mut map = g.minimap;
    map[0] += 2e-9;
    let error = calibration::load_explained(&store, Some((&g.screen, map))).unwrap_err();
    assert_eq!(
        error.details()[1],
        format!(
            "Minimap left (normalized): saved {}, current {}",
            g.minimap[0], map[0]
        )
    );
    map[0] = g.minimap[0] + 0.5e-9;
    assert!(calibration::load_explained(&store, Some((&g.screen, map))).is_ok());
    map[0] = f64::NAN;
    assert!(matches!(
        calibration::load_explained(&store, Some((&g.screen, map))),
        Err(CalibrationIssue::GeometryChanged(_))
    ));
}

#[test]
fn validation_failure_is_explained_without_promoting_or_rewriting_the_profile() {
    let mut saved = profile();
    // Owned LUCASC02 wire contract: persisted validation's angular error.
    saved.model[442..450].copy_from_slice(&6.0_f64.to_le_bytes());
    let error = calibration::assess(Some(saved), None).unwrap_err();
    assert!(matches!(error, CalibrationIssue::ValidationFailed(_)));
    assert!(error.details()[1].contains("error 6°"));
}

#[test]
fn subprocess_protocol_distinguishes_outcomes_and_unexpected_exits() {
    for outcome in [
        CalibrationOutcome::Passed,
        CalibrationOutcome::Cancelled,
        CalibrationOutcome::Failed,
        CalibrationOutcome::Changed,
    ] {
        assert_eq!(
            CalibrationOutcome::from_code(Some(outcome as i32)),
            Some(outcome)
        );
    }
    assert_eq!(CalibrationOutcome::from_code(None), None);
    assert_eq!(CalibrationOutcome::from_code(Some(1)), None);
}

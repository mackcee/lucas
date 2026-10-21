use lucas_app::{
    calibration::{self, CalibrationIssue, CalibrationScript, fit_session, observation},
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

// Captured PC geometry, but synthetic gaze observations: no user's fitted
// weights or webcam data are fixtures. The startup fallback is a regression
// scenario, not a claim to have recovered the old tray's in-memory state.
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
            Alignment {
                offset_x_px: 5.0,
                offset_y_px: 2.0,
                scale_multiplier: 0.9872007536196268,
            }
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
            Alignment {
                offset_x_px: 5.0,
                offset_y_px: 2.0,
                scale_multiplier: 0.9872007536196268,
            },
        )
        .unwrap();
    let script = CalibrationScript::new(
        [3840, 2160],
        MinimapSettings {
            minimap_scale: 2.1,
            flipped: false,
        },
        &alignment,
        [597.0, 336.0],
        650.0,
    )
    .unwrap();
    let rows: Vec<_> = script
        .targets()
        .iter()
        .flat_map(|t| {
            (0..30).map(move |i| {
                let [x, y] = t.target.position;
                observation(
                    t.target,
                    GazeSample {
                        timestamp: Duration::from_millis(i),
                        quality: 0.9,
                        features: Some(GazeFeatures::UniGaze([
                            y as f32, x as f32, 0.2, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 650.0,
                        ])),
                    },
                )
            })
        })
        .collect();
    calibration::profile(
        &fit_session(&script, &rows).unwrap(),
        geometry(false).screen,
        1,
    )
    .unwrap()
}
fn runtime() -> Runtime {
    Runtime::new(Settings {
        live_client_gate: false,
        ..Settings::default()
    })
    .unwrap()
}
fn install(gate: &mut CalibrationStatus, runtime: &mut Runtime, store: &Store) {
    let g = gate.geometry().map(|g| (&g.screen, g.minimap));
    let result = calibration::load_explained(store, g);
    runtime.install_calibration(gate.loaded(result)).unwrap();
}
fn rejected_provisional(store: &Store) -> (CalibrationStatus, Runtime) {
    let mut gate = CalibrationStatus::default();
    let mut rt = runtime();
    install(&mut gate, &mut rt, store);
    assert_eq!(rt.calibration(), CalibrationState::QuickCheck);
    assert!(
        gate.update_geometry(
            geometry(true),
            GeometrySource::Provisional,
            rt.calibration(),
            false
        )
        .reload
    );
    install(&mut gate, &mut rt, store);
    assert_eq!(rt.calibration(), CalibrationState::Needed);
    assert!(matches!(
        gate.issue(),
        Some(CalibrationIssue::GeometryChanged(_))
    ));
    (gate, rt)
}
#[test]
fn provisional_fallback_rejection_recovers_once_to_quick_check_at_matching_live_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let saved_before = std::fs::read(dir.path().join("calibration.json")).unwrap();
    let (mut gate, mut rt) = rejected_provisional(&store);
    let rejected_message = gate.tray_message(false);
    let rejected_details = gate.issue().unwrap().details();
    let update = gate.update_geometry(
        geometry(false),
        GeometrySource::LiveGame,
        rt.calibration(),
        false,
    );
    assert!(
        update.reload,
        "a startup rejection must not latch across the first authoritative geometry"
    );
    install(&mut gate, &mut rt, &store);
    assert_eq!(rt.calibration(), CalibrationState::QuickCheck);
    assert!(gate.issue().is_none());
    let tick = rt
        .tick(
            Duration::from_secs(1),
            Inputs {
                window_exists: true,
                foreground: true,
                geometry_ready: true,
                gaze_mode: true,
                paused: false,
            },
            None,
            1,
        )
        .unwrap();
    assert_eq!(tick.status, Status::QuickCheckNeeded);
    assert!(tick.hide && tick.pulse.is_none());
    let repeated = gate.update_geometry(
        geometry(false),
        GeometrySource::LiveGame,
        rt.calibration(),
        false,
    );
    assert!(!repeated.reload);
    assert_eq!(
        std::fs::read(dir.path().join("calibration.json")).unwrap(),
        saved_before,
        "geometry recovery must not rewrite the user's saved calibration"
    );
    if let Some(evidence) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let evidence = std::path::PathBuf::from(evidence);
        std::fs::create_dir_all(&evidence).unwrap();
        let report = serde_json::json!({
            "synthetic_features_not_webcam_or_gpu_acceptance": true,
            "provisional_rejection": {
                "tray_message": rejected_message,
                "details": rejected_details,
                "minimap": geometry(true).minimap,
            },
            "first_live_geometry": {
                "minimap": geometry(false).minimap,
                "reload_requested": update.reload,
                "calibration_state": format!("{:?}", rt.calibration()),
                "runtime_status": format!("{:?}", tick.status),
                "overlay_hidden": tick.hide,
                "pulse_requested": tick.pulse.is_some(),
                "rejection_cleared": gate.issue().is_none(),
            },
            "repeat_live_geometry_requests_reload": repeated.reload,
            "persisted_calibration_unchanged": true,
        });
        std::fs::write(
            evidence.join("synthetic-calibration-readiness.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn captured_matching_startup_geometry_is_accepted_without_a_recovery_or_threshold_change() {
    let saved = profile();
    let g = geometry(false);
    assert_eq!(
        g.minimap,
        [
            0.8166295833333334,
            0.6726525925925927,
            0.9974020833333335,
            0.994025925925926
        ]
    );
    let model = calibration::assess(Some(saved), Some((&g.screen, g.minimap))).unwrap();
    assert!(
        model
            .validation()
            .passes(model.target_false_positive_rate())
    );
    let mut gate = CalibrationStatus::default();
    let mut rt = runtime();
    rt.install_calibration(gate.loaded(Ok(model))).unwrap();
    assert_eq!(rt.calibration(), CalibrationState::QuickCheck);
    assert!(gate.issue().is_none());
}
#[test]
fn explicit_full_calibration_cancellation_failure_or_setup_closure_cannot_be_resurrected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let before = std::fs::read(dir.path().join("calibration.json")).unwrap();
    for reason in [
        CalibrationIssue::FullCalibrationCancelled,
        CalibrationIssue::FullCalibrationFailed,
        CalibrationIssue::FullCalibrationChanged,
        CalibrationIssue::SetupCancelled,
        CalibrationIssue::FullCalibrationRunning,
    ] {
        let (mut gate, mut rt) = rejected_provisional(&store);
        assert!(!rt.begin_calibration().unwrap());
        gate.block(reason.clone());
        assert!(
            !gate
                .update_geometry(
                    geometry(false),
                    GeometrySource::LiveGame,
                    rt.calibration(),
                    false
                )
                .reload
        );
        assert_eq!(gate.issue(), Some(&reason));
        assert_eq!(rt.calibration(), CalibrationState::Needed);
        assert!(!rt.camera_wanted(true, true, false));
    }
    assert_eq!(
        std::fs::read(dir.path().join("calibration.json")).unwrap(),
        before
    );
}
#[test]
fn failed_cancelled_or_stale_quick_checks_keep_their_reason_across_geometry_changes() {
    for (outcome, same, expected) in [
        (
            Some(CalibrationOutcome::Failed),
            true,
            CalibrationIssue::QuickCheckFailed,
        ),
        (
            Some(CalibrationOutcome::Cancelled),
            true,
            CalibrationIssue::QuickCheckCancelled,
        ),
        (
            Some(CalibrationOutcome::Passed),
            false,
            CalibrationIssue::QuickCheckChanged,
        ),
        (
            Some(CalibrationOutcome::Changed),
            true,
            CalibrationIssue::QuickCheckChanged,
        ),
    ] {
        let model = calibration::assess(Some(profile()), None).unwrap();
        let mut gate = CalibrationStatus::default();
        let mut rt = runtime();
        rt.install_calibration(gate.loaded(Ok(model))).unwrap();
        rt.revalidated(false).unwrap();
        gate.quick_finished(outcome, same);
        assert!(
            !gate
                .update_geometry(
                    geometry(false),
                    GeometrySource::LiveGame,
                    rt.calibration(),
                    false
                )
                .reload
        );
        assert_eq!(gate.issue(), Some(&expected));
        assert_eq!(rt.calibration(), CalibrationState::Needed);
    }
}
#[test]
fn a_live_geometry_rejection_is_not_an_automatic_retry_loop() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let mut gate = CalibrationStatus::default();
    let mut rt = runtime();
    install(&mut gate, &mut rt, &store);
    assert!(
        gate.update_geometry(
            geometry(true),
            GeometrySource::LiveGame,
            rt.calibration(),
            false
        )
        .reload
    );
    install(&mut gate, &mut rt, &store);
    assert_eq!(rt.calibration(), CalibrationState::Needed);
    for i in 0..10 {
        assert!(
            !gate
                .update_geometry(
                    geometry(i % 2 == 0),
                    GeometrySource::LiveGame,
                    rt.calibration(),
                    false
                )
                .reload
        );
    }
}
#[test]
fn active_calibration_never_reloads_a_profile_mid_fit() {
    let mut gate = CalibrationStatus::default();
    gate.block(CalibrationIssue::FullCalibrationRunning);
    assert!(
        !gate
            .update_geometry(
                geometry(false),
                GeometrySource::LiveGame,
                CalibrationState::Needed,
                true
            )
            .reload
    );
    gate.block(CalibrationIssue::FullCalibrationCancelled);
    assert!(
        !gate
            .update_geometry(
                geometry(true),
                GeometrySource::LiveGame,
                CalibrationState::Needed,
                false
            )
            .reload
    );
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
fn live_reproduction_active_full_calibration_does_not_masquerade_as_a_missing_profile() {
    // The read-only 019 snapshot found a --runtime-calibrate child while the
    // user saw "Calibration needed". begin_calibration intentionally revokes
    // readiness, but the tray must explain the active operation instead.
    let mut gate = CalibrationStatus::default();
    let mut rt = runtime();
    rt.install_calibration(gate.loaded(calibration::assess(Some(profile()), None)))
        .unwrap();
    assert!(!rt.begin_calibration().unwrap()); // no session: full calibration
    gate.block(CalibrationIssue::FullCalibrationRunning);
    assert_eq!(rt.calibration(), CalibrationState::Needed);
    assert_eq!(
        gate.tray_message(true),
        "Calibration / quick check running — coaching paused"
    );
    gate.block(CalibrationIssue::FullCalibrationCancelled);
    assert_eq!(
        gate.tray_message(false),
        "Calibration needed — full calibration was cancelled (see Calibration details)"
    );
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
    let mut gate = CalibrationStatus::default();
    gate.quick_finished(None, true);
    assert!(matches!(
        gate.issue(),
        Some(CalibrationIssue::WindowError(_))
    ));
    gate.quick_finished(Some(CalibrationOutcome::Passed), true);
    assert!(gate.issue().is_none());
}

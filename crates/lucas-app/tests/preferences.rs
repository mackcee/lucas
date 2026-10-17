use lucas_app::preferences::{Setup, SetupStep, level_label, needs_setup};
use lucas_core::{Phase, TRAINING_LEVELS};
use lucas_store::{PulseStyle, Settings, Store};

fn profile() -> lucas_store::Calibration {
    use lucas_app::calibration::{CalibrationScript, fit_session, observation};
    let script = CalibrationScript::new(
        [1920, 1080],
        Default::default(),
        &Default::default(),
        [531.0, 299.0],
        650.0,
    )
    .unwrap();
    let samples: Vec<_> = script
        .targets()
        .iter()
        .flat_map(|target| {
            (0..30).map(move |i| {
                let [x, y] = target.target.position.map(|v| v as f32);
                observation(
                    target.target,
                    lucas_gaze::GazeSample {
                        timestamp: std::time::Duration::from_millis(i),
                        quality: 0.9,
                        features: Some(lucas_gaze::GazeFeatures::from_array([
                            x, y, 0.2, x, y, 0.2, 0.0, 0.0, 0.0, 3.0,
                        ])),
                    },
                )
            })
        })
        .collect();
    let model = fit_session(&script, &samples).unwrap();
    lucas_app::calibration::profile(
        &model,
        lucas_store::ScreenGeometry {
            left: 0,
            top: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
        },
        123,
    )
    .unwrap()
}

#[test]
fn labels_are_derived_from_every_core_level() {
    for level in TRAINING_LEVELS {
        let label = level_label(level);
        if let Some(delay) = level.pulse_delay_ms {
            assert!(label.contains(&format!("{} s", delay as f64 / 1000.0)));
        }
        if level.phase == Phase::Fade {
            assert!(label.starts_with("Fade:"));
        }
    }
    assert!(level_label(TRAINING_LEVELS[Settings::default().level_index]).ends_with("(start)"));
}

#[test]
fn every_style_colour_sound_and_level_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    assert!(!store.load_settings().unwrap().pulse.sound);
    for style in [
        PulseStyle::Ripple,
        PulseStyle::BreathingBorder,
        PulseStyle::CornerBrackets,
        PulseStyle::CometSweep,
    ] {
        for sound in [false, true] {
            for level_index in 0..TRAINING_LEVELS.len() {
                let mut settings = Settings {
                    level_index,
                    ..Settings::default()
                };
                settings.pulse.style = style;
                settings.pulse.colour_rgb = [24, 192, 255];
                settings.pulse.sound = sound;
                store.save_settings(&settings).unwrap();
                assert_eq!(store.load_settings().unwrap(), settings);
            }
        }
    }
}

#[test]
fn first_run_complete_skip_cancel_and_resume_preserves_draft_not_settings() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    assert!(needs_setup(dir.path()).unwrap());
    let mut setup = Setup::load(&store).unwrap();
    assert_eq!(setup.step, SetupStep::Intro);
    assert_eq!(setup.draft.level_index, Settings::default().level_index);
    assert!(setup.finish(&store).is_err());
    setup.advance();
    setup.select_camera(7);
    setup.advance();
    setup.checkpoint(dir.path()).unwrap(); // close/cancel has this same boundary
    assert!(needs_setup(dir.path()).unwrap());
    let mut setup = Setup::load(&store).unwrap();
    assert_eq!(setup.step, SetupStep::Borderless);
    assert_eq!(setup.draft.camera_index, 7);
    setup.skip_borderless();
    assert!(setup.borderless_skipped);
    assert_eq!(setup.step, SetupStep::Calibration);
    assert!(!setup.advance());
    setup.calibrated(profile());
    assert!(setup.advance());
    assert_eq!(setup.step, SetupStep::Alignment);
    setup.advance();
    setup.draft.level_index = TRAINING_LEVELS.len() - 1;
    setup.finish(&store).unwrap();
    assert!(!needs_setup(dir.path()).unwrap());
    assert!(!dir.path().join("setup-draft.toml").exists());
    assert_eq!(store.load_settings().unwrap(), setup.draft);
    assert!(store.load_calibration().unwrap().is_some());
}

#[test]
fn cancelled_setup_keeps_live_camera_and_weights_together_and_resumes_calibration() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let old = Settings::default();
    let old_profile = profile();
    store.save_settings(&old).unwrap();
    store.save_calibration(&old_profile).unwrap();
    let mut setup = Setup::new(old.clone());
    setup.select_camera(7);
    for _ in 0..3 {
        setup.advance();
    }
    setup.calibrated(profile());
    setup.advance();
    setup.advance();
    setup.checkpoint(dir.path()).unwrap();
    assert_eq!(store.load_settings().unwrap(), old);
    assert_eq!(store.load_calibration().unwrap().unwrap(), old_profile);
    let resumed = Setup::load(&store).unwrap();
    assert_eq!(resumed.step, SetupStep::Level);
    assert!(resumed.calibration_passed());
    assert!(needs_setup(dir.path()).unwrap());
    resumed.finish(&store).unwrap();
    assert_eq!(store.load_settings().unwrap().camera_index, 7);
    assert!(!needs_setup(dir.path()).unwrap());
}

#[test]
fn interrupted_finish_fails_closed_and_keeps_a_retryable_draft() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_calibration(&profile()).unwrap();
    let mut setup = Setup::new(Settings::default());
    setup.select_camera(9);
    for _ in 0..3 {
        setup.advance();
    }
    setup.calibrated(profile());
    setup.advance();
    setup.advance();
    // Atomic replacement cannot overwrite a directory: force a settings write failure.
    std::fs::create_dir(dir.path().join("settings.toml")).unwrap();
    assert!(setup.finish(&store).is_err());
    assert!(store.load_calibration().unwrap().is_none());
    assert!(needs_setup(dir.path()).unwrap());
    std::fs::remove_dir(dir.path().join("settings.toml")).unwrap();
    let resumed = Setup::load(&store).unwrap();
    assert_eq!(resumed.step, SetupStep::Level);
    resumed.finish(&store).unwrap();
    assert_eq!(store.load_settings().unwrap().camera_index, 9);
    assert!(store.load_calibration().unwrap().is_some());
}

#[test]
fn back_recalibration_and_camera_changes_require_a_new_pass() {
    let mut setup = Setup::new(Settings::default());
    for _ in 0..3 {
        setup.advance();
    }
    assert!(!setup.borderless_skipped);
    setup.calibrated(profile());
    setup.begin_calibration();
    assert!(!setup.advance()); // failed/cancelled recalibration cannot reuse old pass
    setup.calibrated(profile());
    setup.advance();
    setup.back();
    assert_eq!(setup.step, SetupStep::Calibration);
    setup.select_camera(3);
    assert!(!setup.calibration_passed());
    assert!(!setup.advance());
}

#[test]
fn missing_calibration_rewinds_resumed_setup_and_malformed_draft_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let mut setup = Setup::new(Settings::default());
    for _ in 0..3 {
        setup.advance();
    }
    setup.calibrated(profile());
    setup.advance();
    setup.advance();
    setup.begin_calibration();
    setup.checkpoint(dir.path()).unwrap();
    let resumed = Setup::load(&store).unwrap();
    assert_eq!(resumed.step, SetupStep::Calibration);
    assert!(!resumed.calibration_passed());
    std::fs::write(dir.path().join("setup-draft.toml"), "broken[").unwrap();
    assert!(Setup::load(&store).is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("setup-draft.toml")).unwrap(),
        "broken["
    );
}

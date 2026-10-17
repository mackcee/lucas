use lucas_app::preferences::{Setup, SetupStep, level_label, needs_setup, selectable_levels};
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
fn first_run_finishes_only_after_calibration_and_sets_completion() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    assert!(needs_setup(&store.load_settings().unwrap()));
    let mut setup = Setup::load(&store).unwrap();
    assert_eq!(setup.step, SetupStep::Intro);
    assert_eq!(setup.draft.level_index, Settings::default().level_index);
    assert!(setup.finish(&store).is_err());
    setup.advance();
    setup.select_camera(7);
    setup.advance();
    assert!(needs_setup(&store.load_settings().unwrap()));
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
    let saved = store.load_settings().unwrap();
    assert!(!needs_setup(&saved));
    assert!(saved.setup_completed);
    let mut expected = setup.draft;
    expected.setup_completed = true;
    assert_eq!(saved, expected);
    assert!(store.load_calibration().unwrap().is_some());
    // Export the actual persisted contract, not a hand-authored settings sample.
    if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let directory = std::path::Path::new(&path).join("completed-setup-store");
        assert!(!directory.exists(), "refusing to overwrite evidence");
        std::fs::create_dir_all(&directory).unwrap();
        for name in ["settings.toml", "calibration.json"] {
            std::fs::copy(dir.path().join(name), directory.join(name)).unwrap();
        }
        let reopened = Store::open_in(&directory).unwrap();
        assert_eq!(reopened.load_settings().unwrap(), saved);
        assert!(!needs_setup(&reopened.load_settings().unwrap()));
        assert!(reopened.load_calibration().unwrap().is_some());
    }
}

#[test]
fn cancelled_setup_preserves_live_data_and_reopens_with_current_settings() {
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
    drop(setup);
    assert_eq!(store.load_settings().unwrap(), old);
    assert_eq!(store.load_calibration().unwrap().unwrap(), old_profile);
    let current = Settings {
        live_client_gate: false,
        level_index: 2,
        ..old
    };
    store.save_settings(&current).unwrap();
    let mut reopened = Setup::load(&store).unwrap();
    assert_eq!(reopened.step, SetupStep::Intro);
    assert_eq!(reopened.draft, current);
    assert!(!reopened.calibration_passed());
    assert!(needs_setup(&store.load_settings().unwrap()));
    for _ in 0..3 {
        reopened.advance();
    }
    reopened.calibrated(profile());
    reopened.advance();
    reopened.advance();
    reopened.finish(&store).unwrap();
    assert!(!store.load_settings().unwrap().live_client_gate);
    assert!(!needs_setup(&store.load_settings().unwrap()));
}

#[test]
fn failed_finish_keeps_the_in_memory_draft_retryable() {
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
    std::fs::create_dir(dir.path().join("settings.toml")).unwrap();
    assert!(setup.finish(&store).is_err());
    assert!(store.load_calibration().unwrap().is_none());
    std::fs::remove_dir(dir.path().join("settings.toml")).unwrap();
    assert!(needs_setup(&store.load_settings().unwrap()));
    assert_eq!(setup.step, SetupStep::Level);
    setup.finish(&store).unwrap();
    assert!(!needs_setup(&store.load_settings().unwrap()));
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
    assert!(!setup.advance());
    setup.calibrated(profile());
    setup.advance();
    setup.back();
    assert_eq!(setup.step, SetupStep::Calibration);
    setup.select_camera(3);
    assert!(!setup.calibration_passed());
    assert!(!setup.advance());
}

#[test]
fn ordinary_settings_writes_do_not_complete_setup() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    std::fs::write(dir.path().join("settings.toml"), "level_index = 2\n").unwrap();
    let mut settings = store.load_settings().unwrap();
    assert!(needs_setup(&settings));
    settings.live_client_gate = false;
    settings.pulse.sound = true;
    store.save_settings(&settings).unwrap();
    assert!(needs_setup(&store.load_settings().unwrap()));
    let mut runtime = lucas_app::runtime::Runtime::new(settings).unwrap();
    let mut setup = Setup::load(&store).unwrap();
    for _ in 0..3 {
        setup.advance();
    }
    setup.calibrated(profile());
    setup.advance();
    setup.advance();
    setup.finish(&store).unwrap();
    runtime
        .update_preferences(&store.load_settings().unwrap())
        .unwrap();
    assert!(!needs_setup(runtime.settings()));
    let mut updated = runtime.settings().clone();
    updated.level_index = 3;
    store.save_settings(&updated).unwrap();
    assert!(!needs_setup(&store.load_settings().unwrap()));
}

#[test]
fn selectors_offer_ramp_fade_and_off_with_original_core_indices() {
    let levels: Vec<_> = selectable_levels().collect();
    assert_eq!(levels.len(), TRAINING_LEVELS.len() - 1);
    assert_eq!(levels[0].0, Settings::default().level_index);
    assert_eq!(levels[0].1.phase, Phase::Ramp);
    assert_eq!(levels.last().unwrap().1.phase, Phase::Graduated);
    for (index, level) in levels {
        assert_ne!(level.phase, Phase::Baseline);
        assert_eq!(TRAINING_LEVELS[index], level);
    }
}

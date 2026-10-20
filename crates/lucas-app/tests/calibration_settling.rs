//! Synthetic saccade/filter replay, never evidence of the user's actual gaze.
use lucas_app::{calibration::*, script::Phase, timer::alignment_store::AlignmentStore};
use lucas_gaze::{GazeFeatures, GazeSample, filter::OneEuro};
use lucas_geometry::MinimapSettings;
use std::time::Duration;

fn replay(visible_during_settle: bool, reaction_ms: u64) -> (CalibrationScript, Vec<Observation>) {
    replay_for(
        visible_during_settle,
        reaction_ms,
        MinimapSettings::default(),
    )
}
fn replay_for(
    visible_during_settle: bool,
    reaction_ms: u64,
    settings: MinimapSettings,
) -> (CalibrationScript, Vec<Observation>) {
    let script = CalibrationScript::new(
        [3840, 2160],
        settings,
        &AlignmentStore::default(),
        [597.0, 336.0],
        650.0,
    )
    .unwrap();
    let mut filters = [OneEuro::new(1.0, 0.1, 1.0).unwrap(); 2];
    let mut rows = Vec::new();
    for frame in 0..1380_u64 {
        let ms = frame * 1000 / 30;
        let now = Duration::from_millis(ms);
        let (step, target) = script.at(now).unwrap();
        // Eyes keep the previous fixation until the new dot appears + reaction.
        let latency = if visible_during_settle {
            reaction_ms
        } else {
            500 + reaction_ms
        };
        let seen = script
            .at(Duration::from_millis(ms.saturating_sub(latency)))
            .unwrap()
            .1
            .target
            .position;
        let raw = [(seen[1] - 0.5) * 0.5, (seen[0] - 0.5) * 0.7];
        let f = [
            filters[0].filter(now, raw[0] as f32).unwrap(),
            filters[1].filter(now, raw[1] as f32).unwrap(),
            0.2,
            0.2,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            650.0,
        ];
        if step.phase == Phase::Hold && ms % 2000 >= 700 {
            rows.push(observation(
                target.target,
                GazeSample {
                    timestamp: now,
                    quality: 0.9,
                    features: Some(GazeFeatures::UniGaze(f)),
                },
            ));
        }
    }
    (script, rows)
}
#[test]
fn hidden_dot_contaminates_negatives_and_visible_settling_fixes_the_replay() {
    for delay in [150, 200, 250, 300] {
        let (script, rows) = replay(false, delay);
        let old = fit_session(&script, &rows).unwrap();
        assert!(old.validation().mean_error_degrees < 1.5);
        assert!(old.validation().true_positive_rate < 0.5);
        assert_eq!(old.validation().false_positive_rate, 0.0);
        assert_eq!(old.validation().usable_fraction, 1.0);
        assert!(old.threshold() > 0.6);
        let positive: Vec<_> = rows
            .iter()
            .filter(|o| {
                script.targets()[o.target_id as usize].stage == Stage::Validate
                    && o.labelled.minimap
            })
            .collect();
        let half = positive
            .iter()
            .filter(|o| {
                old.predict(o.labelled.sample)
                    .is_some_and(|p| p.probability >= 0.5)
            })
            .count() as f64
            / positive.len() as f64;
        let (script, rows) = replay(true, delay);
        let fixed = fit_session(&script, &rows).unwrap();
        assert!(
            fixed
                .validation()
                .passes(fixed.target_false_positive_rate())
        );
        assert_eq!(fixed.threshold(), 0.5);
        println!(
            "reaction={delay}ms hidden: error={:.3} tpr={:.3} tpr_at_half={half:.3} threshold={:.3}; visible: error={:.3} tpr={:.3}",
            old.validation().mean_error_degrees,
            old.validation().true_positive_rate,
            old.threshold(),
            fixed.validation().mean_error_degrees,
            fixed.validation().true_positive_rate
        );
    }
}
#[test]
fn physical_dimensions_change_degrees_not_the_rectangular_decision() {
    let (script, rows) = replay(false, 200);
    let correct = fit_session(&script, &rows).unwrap();
    let default_size = CalibrationScript::new(
        [3840, 2160],
        MinimapSettings::default(),
        &AlignmentStore::default(),
        [531.0, 299.0],
        650.0,
    )
    .unwrap();
    let smaller = fit_session(&default_size, &rows).unwrap();
    assert!(correct.validation().mean_error_degrees > smaller.validation().mean_error_degrees);
    assert_eq!(correct.threshold(), smaller.threshold());
    assert_eq!(correct.decision_rectangle(), smaller.decision_rectangle());
    assert_eq!(
        correct.validation().true_positive_rate,
        smaller.validation().true_positive_rate
    );
    for o in &rows {
        assert_eq!(
            correct.predict(o.labelled.sample),
            smaller.predict(o.labelled.sample)
        );
    }
}
#[test]
fn calibration_dot_is_visible_before_samples_become_eligible() {
    let (script, _) = replay(true, 200);
    for ms in [0, 200, 499, 500, 700, 1999, 2000] {
        let (step, _) = script.at(Duration::from_millis(ms)).unwrap();
        assert_eq!(visible_target(step, true), Some(step.target));
        assert_eq!(visible_target(step, false), None);
    }
    assert_eq!(script.duration(), Duration::from_secs(46));
}
#[test]
fn low_mean_error_also_can_mask_local_bias_without_threshold_inflation() {
    // A separate falsifier: settling is not the only way to get this symptom.
    let (script, mut rows) = replay_for(
        true,
        200,
        MinimapSettings {
            minimap_scale: 0.0,
            flipped: false,
        },
    );
    for row in &mut rows {
        if script.targets()[row.target_id as usize].stage == Stage::Validate && row.labelled.minimap
        {
            let Some(GazeFeatures::UniGaze(ref mut f)) = row.labelled.sample.features else {
                unreachable!()
            };
            f[1] -= 0.056 * 0.7;
        }
    }
    let model = fit_session(&script, &rows).unwrap();
    println!(
        "local map bias (synthetic): {:?}, threshold {}",
        model.validation(),
        model.threshold()
    );
    assert!(model.validation().mean_error_degrees < 2.0);
    assert_eq!(model.validation().usable_fraction, 1.0);
    assert_eq!(model.threshold(), 0.5);
    assert!(model.validation().true_positive_rate < 0.1);
    assert!(
        !model
            .validation()
            .passes(model.target_false_positive_rate())
    );
}
/// Exercise the persisted UniGaze-to-coaching seam, not just the fit in memory.
/// Optional artifacts contain synthetic numbers only, never webcam acceptance.
#[test]
fn unigaze_settling_fit_persists_and_drives_coaching() {
    use lucas_core::GazeState;
    use lucas_gaze::{decision::CalibratedDetector, features::FeatureKind};

    let (script, rows) = replay(true, 200);
    let model = fit_session(&script, &rows).unwrap();
    assert!(
        model
            .validation()
            .passes(model.target_false_positive_rate())
    );
    let directory = tempfile::tempdir().unwrap();
    let screen = lucas_store::ScreenGeometry {
        left: 0,
        top: 0,
        width: 3840,
        height: 2160,
        scale_factor: 1.0,
    };
    {
        let store = lucas_store::Store::open_in(directory.path()).unwrap();
        save(&store, &model, screen.clone(), 1).unwrap();
    }
    let store = lucas_store::Store::open_in(directory.path()).unwrap();
    let restored = load(&store, &screen, script.screen().minimap)
        .unwrap()
        .unwrap();
    assert_eq!(restored, model);
    assert_eq!(restored.feature_kind(), FeatureKind::UniGaze);

    // A later failed calibration must not replace the usable saved model.
    let (hidden_script, hidden_rows) = replay(false, 200);
    let failed = fit_session(&hidden_script, &hidden_rows).unwrap();
    assert!(save(&store, &failed, screen.clone(), 2).is_err());
    assert_eq!(
        load(&store, &screen, script.screen().minimap).unwrap(),
        Some(model.clone())
    );

    let [left, top, right, bottom] = model.screen().minimap;
    let point = [(left + right) / 2.0, (top + bottom) / 2.0];
    let mut detector = CalibratedDetector::new(restored).unwrap();
    let mut events = Vec::new();
    for (ms, known, starts) in [
        (0, true, false),
        (50, true, false),
        (99, true, false),
        (100, true, true),
        (150, false, false),
        (200, true, false),
        (250, true, false),
        (300, true, true),
    ] {
        let sample = GazeSample {
            timestamp: Duration::from_millis(ms),
            quality: if known { 0.9 } else { 0.0 },
            features: known.then_some(GazeFeatures::UniGaze([
                ((point[1] - 0.5) * 0.5) as f32,
                ((point[0] - 0.5) * 0.7) as f32,
                0.2,
                0.2,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                650.0,
            ])),
        };
        let update = detector.observe(sample).unwrap();
        assert_eq!(update.check_started, starts, "timestamp {ms}");
        if !known {
            assert_eq!(update.state, GazeState::Unknown);
        }
        events.push(serde_json::json!({
            "timestamp_ms": ms, "known": known,
            "state": format!("{:?}", update.state), "check_started": update.check_started,
        }));
    }

    if let Some(evidence) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let evidence = std::path::PathBuf::from(evidence);
        std::fs::create_dir_all(&evidence).unwrap();
        std::fs::copy(
            directory.path().join("calibration.json"),
            evidence.join("synthetic-unigaze-calibration.json"),
        )
        .unwrap();
        let report = serde_json::json!({
            "synthetic_features_not_webcam_or_gpu_acceptance": true,
            "calibration_duration_seconds": script.duration().as_secs(),
            "persisted_feature_kind": model.feature_kind(),
            "visible_settling": lucas_app::calibration_diagnostics::report(&script, &rows, &[], Some(&model), None, None).unwrap()["model"],
            "hidden_settling": lucas_app::calibration_diagnostics::report(&hidden_script, &hidden_rows, &[], Some(&failed), None, None).unwrap()["model"],
            "failed_calibration_preserved_saved_weights": true,
            "coaching_events": events,
        });
        std::fs::write(
            evidence.join("synthetic-unigaze-coaching.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn numeric_export_is_opt_in_and_reconstructs_the_failed_model() {
    use lucas_app::calibration_diagnostics as diag;
    let (script, rows) = replay(false, 200);
    let model = fit_session(&script, &rows).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(
        diag::export(None, &script, &rows, &[], Some(&model), None, None)
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let file = diag::export(
        Some(directory.path()),
        &script,
        &rows,
        &[],
        Some(&model),
        None,
        None,
    )
    .unwrap()
    .unwrap();
    let value: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(file).unwrap()).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["numbers_only"], true);
    assert_eq!(value["observations"].as_array().unwrap().len(), rows.len());
    let bytes: Vec<u8> = serde_json::from_value(value["model"]["weights"].clone()).unwrap();
    let restored = lucas_gaze::calibration::CalibrationModel::from_bytes(&bytes).unwrap();
    assert_eq!(restored, model);
    assert!(
        !restored
            .validation()
            .passes(restored.target_false_positive_rate())
    );
    let error = diag::report(
        &script,
        &rows,
        &[],
        None,
        Some("synthetic fit failure"),
        None,
    )
    .unwrap();
    assert!(error["model"].is_null());
    assert_eq!(error["fit_error"], "synthetic fit failure");
}

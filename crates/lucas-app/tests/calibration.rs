use lucas_app::{
    calibration::*,
    script::{Phase, Script, TargetKind},
    settings::MinimapSelection,
    trace::*,
};
use lucas_gaze::{
    GazeFeatures, GazeSample,
    calibration::{FitOptions, LabelledSample, fit},
};
use lucas_geometry::MinimapSettings;
use std::time::Duration;
fn sample(p: [f64; 2], n: u64) -> GazeSample {
    GazeSample {
        timestamp: Duration::from_millis(n),
        quality: 0.9,
        features: Some(GazeFeatures::from_array([
            p[0] as f32,
            p[1] as f32,
            0.2,
            p[0] as f32,
            p[1] as f32,
            0.2,
            0.0,
            0.0,
            0.0,
            3.0,
        ])),
    }
}
fn fitted() -> lucas_gaze::calibration::CalibrationModel {
    let script = CalibrationScript::new(
        [1920, 1080],
        MinimapSettings::default(),
        [531.0, 299.0],
        650.0,
    )
    .unwrap();
    let observations: Vec<_> = script
        .targets()
        .iter()
        .flat_map(|t| (0..30).map(move |i| observation(t.target, sample(t.target.position, i))))
        .collect();
    fit_session(&script, &observations).unwrap()
}
#[test]
fn session_fits_persists_and_revalidates_without_samples_on_disk() {
    let model = fitted();
    assert!(
        model
            .validation()
            .passes(model.target_false_positive_rate()),
        "{:?}",
        model.validation()
    );
    let dir = tempfile::tempdir().unwrap();
    let store = lucas_store::Store::open_in(dir.path()).unwrap();
    let screen = lucas_store::ScreenGeometry {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
        scale_factor: 1.5,
    };
    save(&store, &model, screen.clone(), 123).unwrap();
    assert_eq!(
        load(&store, &screen, model.screen().minimap).unwrap(),
        Some(model.clone())
    );
    let mut changed = screen.clone();
    changed.scale_factor = 1.0;
    assert!(
        load(&store, &changed, model.screen().minimap)
            .unwrap()
            .is_none()
    );
    let mut map = model.screen().minimap;
    map[0] += 0.01;
    assert!(load(&store, &screen, map).unwrap().is_none());
    let quick = CalibrationScript::quick(&model);
    assert_eq!(quick.duration(), Duration::from_secs(4));
    let observations: Vec<_> = quick
        .targets()
        .iter()
        .flat_map(|t| (0..30).map(move |i| observation(t.target, sample(t.target.position, i))))
        .collect();
    assert!(
        quick_revalidate(&model, &observations)
            .unwrap()
            .passes(model.target_false_positive_rate())
    );
    let mut drift = observations.clone();
    for o in &mut drift {
        o.labelled.sample = sample([0.5, 0.5], 0);
    }
    assert!(
        !quick_revalidate(&model, &drift)
            .unwrap()
            .passes(model.target_false_positive_rate())
    );
    assert!(quick_revalidate(&model, &[]).is_err());
    assert_eq!(
        store.load_calibration().unwrap().unwrap().model,
        model.to_bytes()
    );
    let text = std::fs::read_to_string(dir.path().join("calibration.json")).unwrap();
    assert!(!text.contains("features"));
}
fn trace(bad: bool) -> Trace {
    let header = Header {
        schema_version: 1,
        app_version: "synthetic".into(),
        started_unix_ms: 0,
        camera: CameraInfo {
            name: "synthetic".into(),
            resolution: [1280, 720],
            fps: 30,
        },
        monitor_resolution: [1920, 1080],
        dpi_scale: 1.0,
        lighting_note: "synthetic fixture, not webcam evidence".into(),
        script_seconds: 120,
        minimap: MinimapSelection::resolve(&lucas_game::SettingsLocator::default()).unwrap(),
    };
    let script = Script::new(120, header.monitor_resolution, header.minimap.settings()).unwrap();
    let mut writer = TraceWriter::new(Vec::new(), header).unwrap();
    for n in 0..3600 {
        let timestamp = Duration::from_millis(n * 1000 / 30);
        let step = script.at(timestamp).unwrap();
        let p = if bad && step.target.id >= 14 {
            [0.5, 0.5]
        } else {
            step.target.position
        };
        let features = sample(p, 0).features.map(GazeFeatures::as_array);
        writer
            .frame(FrameRecord {
                timestamp_ns: timestamp.as_nanos() as u64,
                source_timestamp_ns: timestamp.as_nanos() as u64,
                features,
                raw_features: features,
                face_found: true,
                quality: 0.9,
                cpu_ms: 1.0,
                phase: step.phase,
                target: (step.phase == Phase::Hold).then_some(step.target),
            })
            .unwrap();
    }
    read_trace(writer.finish(EndReason::Completed).unwrap().as_slice()).unwrap()
}
#[test]
fn heldout_synthetic_trace_passes_and_missed_glances_fail_g2() {
    let good = trace(false);
    let run = lucas_app::eval::evaluate(&good, None, [531.0, 299.0], 650.0).unwrap();
    assert_eq!(run.training_trials, 10);
    assert_eq!(run.validation_trials, 4);
    assert!(run.evaluation.passes(), "{:?}", run.evaluation);
    let bad = trace(true);
    let run = lucas_app::eval::evaluate(&bad, None, [531.0, 299.0], 650.0).unwrap();
    assert!(!run.evaluation.passes());
    assert_eq!(run.evaluation.detected_trials, 0);
    let separate = lucas_app::eval::evaluate(&good, Some(&good), [531.0, 299.0], 650.0).unwrap();
    assert_eq!(separate.evaluation.minimap_trials, 10);
}
#[test]
fn false_detections_and_unknowns_cannot_pass_as_negative_evidence() {
    let calibration = trace(false);
    let mut test = trace(false);
    let map = Script::new(120, [1920, 1080], MinimapSettings::default())
        .unwrap()
        .at(Duration::from_secs(8))
        .unwrap()
        .target
        .position;
    for f in &mut test.frames {
        if f.target.is_some_and(|t| t.kind == TargetKind::Elsewhere) {
            f.features = sample(map, 0).features.map(GazeFeatures::as_array);
        }
    }
    let result =
        lucas_app::eval::evaluate(&test, Some(&calibration), [531.0, 299.0], 650.0).unwrap();
    assert!(result.evaluation.false_per_minute() > 1.0);
    assert!(!result.evaluation.passes());
    for f in &mut test.frames {
        f.features = None;
        f.quality = 0.0;
    }
    assert!(
        !lucas_app::eval::evaluate(&test, Some(&calibration), [531.0, 299.0], 650.0)
            .unwrap()
            .evaluation
            .passes()
    );
}
#[test]
fn missing_heldout_trials_are_misses_not_removed_from_the_denominator() {
    let mut incomplete = trace(false);
    incomplete
        .frames
        .retain(|f| f.target.is_none_or(|t| t.id < 14));
    let run = lucas_app::eval::evaluate(&incomplete, None, [531.0, 299.0], 650.0).unwrap();
    assert_eq!(run.evaluation.minimap_trials, 3);
    assert_eq!(run.evaluation.detected_trials, 0);
    assert_eq!(run.evaluation.coverage, 0.0);
    assert!(!run.evaluation.passes());
}
#[test]
fn flipped_and_scaled_minimaps_fit_and_revalidate() {
    for flipped in [false, true] {
        for minimap_scale in [0.0, 3.0] {
            let script = CalibrationScript::new(
                [1920, 1080],
                MinimapSettings {
                    minimap_scale,
                    flipped,
                },
                [531.0, 299.0],
                650.0,
            )
            .unwrap();
            let rows: Vec<_> = script
                .targets()
                .iter()
                .flat_map(|t| {
                    (0..30).map(move |i| observation(t.target, sample(t.target.position, i)))
                })
                .collect();
            let model = fit_session(&script, &rows).unwrap();
            assert!(
                model
                    .validation()
                    .passes(model.target_false_positive_rate()),
                "{flipped} {minimap_scale}: {:?}",
                model.validation()
            );
            let mut invalid = rows;
            invalid[0].labelled.target = [0.4, 0.4];
            assert!(fit_session(&script, &invalid).is_err());
        }
    }
}
#[test]
fn failed_calibration_does_not_replace_previous_weights() {
    let good = fitted();
    let dir = tempfile::tempdir().unwrap();
    let store = lucas_store::Store::open_in(dir.path()).unwrap();
    let screen = lucas_store::ScreenGeometry {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
        scale_factor: 1.0,
    };
    save(&store, &good, screen.clone(), 1).unwrap();
    let rows: Vec<_> = (0..30)
        .flat_map(|i| {
            [true, false].map(move |minimap| LabelledSample {
                sample: sample([0.5, 0.5], i),
                target: if minimap { [0.9, 0.9] } else { [0.5, 0.5] },
                minimap,
            })
        })
        .collect();
    let failed = fit(&rows, &rows, good.screen(), FitOptions::default()).unwrap();
    assert!(save(&store, &failed, screen, 2).is_err());
    assert_eq!(
        store.load_calibration().unwrap().unwrap().model,
        good.to_bytes()
    );
}

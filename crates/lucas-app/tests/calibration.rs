use lucas_app::{
    calibration::*,
    script::{Phase, Script, TargetKind},
    settings::MinimapSelection,
    timer::alignment_store::{AlignmentStore, Key},
    trace::*,
};
use lucas_gaze::{
    GazeFeatures, GazeSample,
    calibration::{FitOptions, LabelledSample, fit},
};
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};
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
        &AlignmentStore::default(),
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
fn calibration_and_quick_check_cover_every_target_at_30_and_60fps() {
    let script = CalibrationScript::new(
        [1920, 1080],
        MinimapSettings::default(),
        &AlignmentStore::default(),
        [531.0, 299.0],
        650.0,
    )
    .unwrap();
    for fps in [30, 60] {
        let collect = |script: &CalibrationScript| {
            (0..script.duration().as_secs() * fps)
                .filter_map(|n| {
                    let at = Duration::from_nanos(n * 1_000_000_000 / fps);
                    let (step, _) = script.at(at)?;
                    let target = step.target;
                    let within = at - TARGET_TIME * target.id;
                    if step.phase != Phase::Hold || within < SETTLE + Duration::from_millis(200) {
                        return None;
                    }
                    Some(observation(
                        target,
                        sample(target.position, at.as_millis() as u64),
                    ))
                })
                .collect::<Vec<_>>()
        };
        let observations = collect(&script);
        assert!(observations.len() <= MAX_OBSERVATIONS);
        assert!(script.targets().iter().all(|t| {
            observations
                .iter()
                .filter(|o| o.target_id == t.target.id)
                .count()
                >= 12
        }));
        let model = fit_session(&script, &observations).unwrap();
        assert!(
            model
                .validation()
                .passes(model.target_false_positive_rate())
        );
        let quick = CalibrationScript::quick(&model);
        assert!(
            quick_revalidate(&model, &collect(&quick))
                .unwrap()
                .passes(model.target_false_positive_rate())
        );
    }
}

#[test]
fn quick_check_snapshot_rejects_replaced_weights_and_changed_cameras() {
    let model_a = fitted();
    let expected_a = quick_snapshot_id(&model_a, 0);
    let script = CalibrationScript::new(
        [1920, 1080],
        MinimapSettings::default(),
        &AlignmentStore::default(),
        [531.0, 299.0],
        650.0,
    )
    .unwrap();
    let observations: Vec<_> = script
        .targets()
        .iter()
        .flat_map(|t| {
            (0..30).map(move |i| {
                let p = [t.target.position[0] + 0.02, t.target.position[1]];
                observation(t.target, sample(p, i))
            })
        })
        .collect();
    let model_b = fit_session(&script, &observations).unwrap();
    assert_ne!(model_a, model_b);
    let dir = tempfile::tempdir().unwrap();
    let store = lucas_store::Store::open_in(dir.path()).unwrap();
    let screen = lucas_store::ScreenGeometry {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
        scale_factor: 1.0,
    };
    save(&store, &model_a, screen.clone(), 1).unwrap();
    let loaded_a = load(&store, &screen, model_a.screen().minimap)
        .unwrap()
        .unwrap();
    verify_quick_snapshot(&loaded_a, 0, &expected_a).unwrap();
    assert!(verify_quick_snapshot(&loaded_a, 1, &expected_a).is_err());

    save(&store, &model_b, screen.clone(), 2).unwrap();
    let loaded_b = load(&store, &screen, model_b.screen().minimap)
        .unwrap()
        .unwrap();
    assert!(verify_quick_snapshot(&loaded_b, 0, &expected_a).is_err());
    let expected_b = quick_snapshot_id(&model_b, 1);
    verify_quick_snapshot(&loaded_b, 1, &expected_b).unwrap();
    assert!(verify_quick_snapshot(&loaded_b, 0, &expected_b).is_err());
    assert!(verify_quick_snapshot(&loaded_a, 1, &expected_b).is_err());
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
                &AlignmentStore::default(),
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
fn saved_alignment_controls_targets_labels_and_calibration_reuse() {
    let evidence = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR").map(std::path::PathBuf::from);
    let mut reports = Vec::new();
    for flipped in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let settings = MinimapSettings {
            flipped,
            ..MinimapSettings::default()
        };
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0).unwrap();
        let key = Key::new(bounds, settings);
        let alignment = Alignment {
            offset_x_px: if flipped { 12.0 } else { -12.0 },
            offset_y_px: -10.0,
            scale_multiplier: 1.25,
        };
        let mut alignments = AlignmentStore::default();
        alignments.set(key, alignment).unwrap();
        alignments.save(dir.path()).unwrap();
        let script = CalibrationScript::new(
            [1920, 1080],
            settings,
            &AlignmentStore::load(dir.path()).unwrap(),
            [531.0, 299.0],
            650.0,
        )
        .unwrap();
        let map = minimap_rect(bounds, settings, alignment).unwrap();
        assert_eq!(
            script.screen().minimap,
            [
                map.left() / 1920.0,
                map.top() / 1080.0,
                map.right() / 1920.0,
                map.bottom() / 1080.0,
            ]
        );
        let seed = minimap_rect(bounds, settings, Alignment::default()).unwrap();
        assert!(map.contains(
            (seed.left() + seed.right()) / 2.0,
            seed.top() - 0.04 * 1080.0,
        ));
        let above = script.targets()[12].target;
        assert_eq!(above.kind, TargetKind::Elsewhere);
        assert!((above.position[1] - (map.top() / 1080.0 - 0.04)).abs() < 1e-9);
        for target in script.targets() {
            let [x, y] = target.target.position;
            assert_eq!(
                target.target.kind == TargetKind::Minimap,
                map.contains(x * 1920.0, y * 1080.0)
            );
        }
        let rows: Vec<_> = script
            .targets()
            .iter()
            .flat_map(|t| (0..30).map(move |i| observation(t.target, sample(t.target.position, i))))
            .collect();
        let model = fit_session(&script, &rows).unwrap();
        let store = lucas_store::Store::open_in(dir.path()).unwrap();
        let screen = lucas_store::ScreenGeometry {
            left: 0,
            top: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
        };
        save(&store, &model, screen.clone(), 1).unwrap();
        let loaded = load(&store, &screen, script.screen().minimap).unwrap();
        let mut report = serde_json::json!({
            "synthetic_features_not_webcam_acceptance": true,
            "flipped": flipped,
            "aligned_minimap_normalized": script.screen().minimap,
            "lane_above_target_normalized": above.position,
            "unchanged_geometry_reuses_model": loaded.is_some(),
            "changed_alignments": [],
        });
        assert_eq!(loaded, Some(model));
        if let Some(directory) = &evidence {
            std::fs::create_dir_all(directory).unwrap();
            for name in ["calibration.json", "alignment.toml"] {
                std::fs::copy(
                    dir.path().join(name),
                    directory.join(format!("aligned-flipped-{flipped}-{name}")),
                )
                .unwrap();
            }
        }
        for changed in [
            Alignment {
                scale_multiplier: 1.0,
                ..alignment
            },
            Alignment {
                offset_x_px: 0.0,
                ..alignment
            },
            Alignment {
                offset_y_px: 0.0,
                ..alignment
            },
        ] {
            alignments.set(key, changed).unwrap();
            alignments.save(dir.path()).unwrap();
            let script = CalibrationScript::new(
                [1920, 1080],
                settings,
                &AlignmentStore::load(dir.path()).unwrap(),
                [531.0, 299.0],
                650.0,
            )
            .unwrap();
            let reused = load(&store, &screen, script.screen().minimap)
                .unwrap()
                .is_some();
            report["changed_alignments"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "offset_x_px": changed.offset_x_px,
                    "offset_y_px": changed.offset_y_px,
                    "scale_multiplier": changed.scale_multiplier,
                    "minimap_normalized": script.screen().minimap,
                    "reuses_saved_model": reused,
                }));
            assert!(!reused);
        }
        reports.push(report);
    }
    if let Some(directory) = evidence {
        std::fs::write(
            directory.join("alignment-calibration-reuse.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
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

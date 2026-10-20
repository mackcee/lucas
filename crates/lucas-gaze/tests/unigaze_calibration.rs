//! Synthetic boundary tests, not real-camera accuracy claims.
use lucas_core::GazeState;
use lucas_gaze::{
    GazeFeatures, GazeSample, calibration::*, decision::CalibratedDetector, features::FeatureKind,
};
use std::time::Duration;
fn sample(point: [f64; 2], ms: u64) -> GazeSample {
    GazeSample {
        timestamp: Duration::from_millis(ms),
        quality: 0.9,
        features: Some(GazeFeatures::UniGaze([
            ((point[1] - 0.5) * 0.5) as f32,
            ((point[0] - 0.5) * 0.7) as f32,
            0.2,
            0.2,
            0.1,
            -0.1,
            0.0,
            20.0,
            30.0,
            650.0,
        ])),
    }
}
fn screen(flipped: bool) -> Screen {
    Screen {
        resolution: [3840, 2160],
        size_mm: [597.0, 336.0],
        distance_mm: 650.0,
        distance_assumed: true,
        minimap: if flipped {
            [0.15, 0.55, 0.4, 0.9]
        } else {
            [0.6, 0.55, 0.85, 0.9]
        },
    }
}
fn label(point: [f64; 2], screen: Screen) -> LabelledSample {
    let [l, t, r, b] = screen.minimap;
    LabelledSample {
        sample: sample(point, 0),
        target: point,
        minimap: point[0] >= l && point[0] <= r && point[1] >= t && point[1] <= b,
    }
}
fn data(screen: Screen) -> (Vec<LabelledSample>, Vec<LabelledSample>) {
    let [l, t, r, b] = screen.minimap;
    let center = [(l + r) / 2.0, (t + b) / 2.0];
    let mut train = Vec::new();
    let mut valid = Vec::new();
    for _ in 0..20 {
        for x in [0.1, 0.3, 0.5, 0.7, 0.9] {
            for y in [0.1, 0.3, 0.5, 0.7, 0.9] {
                train.push(label([x, y], screen));
            }
        }
        train.push(label(center, screen));
        for point in [
            center,
            [l - 0.03, center[1]],
            [r + 0.03, center[1]],
            [center[0], t - 0.03],
            [center[0], b + 0.03],
        ] {
            valid.push(label(point, screen));
        }
    }
    (train, valid)
}
#[test]
fn rectangular_decision_rejects_all_four_sides_and_survives_roundtrip() {
    for flipped in [false, true] {
        let s = screen(flipped);
        let (train, valid) = data(s);
        let model = fit(&train, &valid, s, FitOptions::default()).unwrap();
        assert!(
            model
                .validation()
                .passes(model.target_false_positive_rate()),
            "{:?}",
            model.validation()
        );
        assert_eq!(model.feature_kind(), FeatureKind::UniGaze);
        let model = CalibrationModel::from_bytes(&model.to_bytes()).unwrap();
        for row in &valid {
            let p = model.predict(row.sample).unwrap();
            assert_eq!(
                p.probability >= model.threshold(),
                row.minimap,
                "{:?}",
                row.target
            );
        }
        let raw = valid[0].sample.features.unwrap().as_array();
        assert!(
            model
                .predict(GazeSample {
                    features: Some(GazeFeatures::from_array(raw)),
                    ..valid[0].sample
                })
                .is_none()
        );
        let mut changed = raw;
        changed[4] = 0.4;
        changed[7] = -100.0;
        changed[9] = 900.0;
        assert_eq!(
            model.predict(valid[0].sample),
            model.predict(GazeSample {
                features: Some(GazeFeatures::UniGaze(changed)),
                ..valid[0].sample
            })
        );
        changed[2] = 0.01;
        assert!(
            model
                .predict(GazeSample {
                    features: Some(GazeFeatures::UniGaze(changed)),
                    ..valid[0].sample
                })
                .is_none()
        );
    }
}
#[test]
fn source_mixing_and_old_half_plane_models_fail_closed() {
    let (mut train, valid) = data(screen(false));
    let model = fit(&train, &valid, screen(false), FitOptions::default()).unwrap();
    let mut old = model.to_bytes();
    old[..8].copy_from_slice(b"LUCASC01");
    assert!(CalibrationModel::from_bytes(&old).is_err());
    train[0].sample.features = Some(GazeFeatures::from_array([
        0.5, 0.5, 0.2, 0.5, 0.5, 0.2, 0.0, 0.0, 0.0, 3.0,
    ]));
    assert!(fit(&train, &valid, screen(false), FitOptions::default()).is_err());
    assert!(model.validate_samples(&train).is_err());
}
#[test]
fn acquisition_timed_dwell_still_requires_100ms_and_unknown_resets_it() {
    let (train, valid) = data(screen(false));
    let model = fit(&train, &valid, screen(false), FitOptions::default()).unwrap();
    let mut detector = CalibratedDetector::new(model).unwrap();
    let point = [0.725, 0.725];
    for ms in [0, 50, 99] {
        assert!(!detector.observe(sample(point, ms)).unwrap().check_started);
    }
    assert!(detector.observe(sample(point, 100)).unwrap().check_started);
    let unknown = GazeSample {
        timestamp: Duration::from_millis(150),
        features: None,
        quality: 0.0,
    };
    assert_eq!(detector.observe(unknown).unwrap().state, GazeState::Unknown);
    for ms in [200, 250] {
        assert!(!detector.observe(sample(point, ms)).unwrap().check_started);
    }
    assert!(detector.observe(sample(point, 300)).unwrap().check_started);
}

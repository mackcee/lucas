use lucas_core::GazeState;
use lucas_gaze::{GazeFeatures, GazeSample, calibration::*, decision::CalibratedDetector};
use std::time::Duration;
fn screen() -> Screen {
    Screen {
        resolution: [1920, 1080],
        size_mm: [531.0, 299.0],
        distance_mm: 650.0,
        distance_assumed: true,
        minimap: [0.82, 0.75, 0.995, 0.995],
    }
}
fn sample(x: f64, y: f64, at: u64) -> GazeSample {
    GazeSample {
        timestamp: Duration::from_millis(at),
        quality: 0.9,
        features: Some(GazeFeatures::from_array([
            x as f32, y as f32, 0.2, x as f32, y as f32, 0.2, 0.0, 0.0, 0.0, 3.0,
        ])),
    }
}
fn data(noisy: bool) -> Vec<LabelledSample> {
    let positions = [
        [0.88, 0.85],
        [0.94, 0.9],
        [0.5, 0.5],
        [0.5, 0.92],
        [0.94, 0.1],
        [0.75, 0.85],
        [0.9, 0.65],
        [0.1, 0.1],
        [0.1, 0.85],
    ];
    (0..30)
        .flat_map(|i| {
            positions.into_iter().enumerate().map(move |(j, target)| {
                let shift = if noisy {
                    ((i * 7 + j) % 11) as f64 * 0.018 - 0.09
                } else {
                    0.0
                };
                LabelledSample {
                    sample: sample(target[0] + shift, target[1] - shift, 0),
                    target,
                    minimap: j < 2,
                }
            })
        })
        .collect()
}
#[test]
fn fit_heldout_roundtrip_and_compact_versioned_weights() {
    let model = fit(&data(false), &data(false), screen(), FitOptions::default()).unwrap();
    assert!(
        model
            .validation()
            .passes(model.target_false_positive_rate()),
        "{:?}",
        model.validation()
    );
    assert!(model.validation().mean_error_degrees < 1.0);
    assert!(model.inflated_zone()[0] < screen().minimap[0]);
    assert!(model.screen().distance_assumed);
    let bytes = model.to_bytes();
    assert!(bytes.len() < 1024);
    assert_eq!(CalibrationModel::from_bytes(&bytes).unwrap(), model);
    for n in [0, 7, 16, bytes.len() - 1] {
        assert!(CalibrationModel::from_bytes(&bytes[..n]).is_err());
    }
    let mut bad = bytes.clone();
    bad[7] = b'9';
    assert!(CalibrationModel::from_bytes(&bad).is_err());
    let mut bad = bytes.clone();
    bad[18..26].copy_from_slice(&f64::NAN.to_le_bytes());
    assert!(CalibrationModel::from_bytes(&bad).is_err());
    let mut bad = bytes;
    bad.push(0);
    assert!(CalibrationModel::from_bytes(&bad).is_err());
}
#[test]
fn noisy_validation_sets_conservative_threshold_and_reports_misses() {
    let model = fit(&data(false), &data(true), screen(), FitOptions::default()).unwrap();
    assert!(model.validation().false_positive_rate <= 0.001);
    assert!(model.validation().true_positive_rate < 1.0);
    assert!(model.validation().mean_error_degrees > 0.5);
    let noisy_fit = fit(&data(true), &data(true), screen(), FitOptions::default()).unwrap();
    assert!(noisy_fit.validation().false_positive_rate <= 0.001);
    // Threshold selection cannot leak validation observations into the weights.
    let clean = fit(&data(false), &data(false), screen(), FitOptions::default()).unwrap();
    assert_eq!(
        &model.to_bytes()[18..18 + 44 * 8],
        &clean.to_bytes()[18..18 + 44 * 8]
    );
}
#[test]
fn angular_error_and_inflation_use_physical_monitor_geometry() {
    let s = screen();
    let mm = s.distance_mm * 1.0_f64.to_radians().tan();
    let error = s.angular_error([0.5 + mm / s.size_mm[0], 0.5], [0.5, 0.5]);
    assert!((error - 1.0).abs() < 1e-9);
    let zone = s.zone(error).unwrap();
    assert!((zone[0] - (s.minimap[0] - mm / s.size_mm[0])).abs() < 1e-9);
    assert!((zone[1] - (s.minimap[1] - mm / s.size_mm[1])).abs() < 1e-9);
    assert_eq!(zone[2], 1.0);
    assert_eq!(zone[3], 1.0);
}
#[test]
fn invalid_geometry_options_empty_and_single_class_are_rejected() {
    assert!(fit(&[], &data(false), screen(), FitOptions::default()).is_err());
    let mut single = data(false);
    single.iter_mut().for_each(|s| s.minimap = true);
    assert!(fit(&single, &data(false), screen(), FitOptions::default()).is_err());
    let mut s = screen();
    s.distance_mm = f64::NAN;
    assert!(fit(&data(false), &data(false), s, FitOptions::default()).is_err());
    assert!(fit(&data(false), &[], screen(), FitOptions::default()).is_err());
    assert!(
        fit(
            &data(false),
            &data(false),
            screen(),
            FitOptions {
                target_false_positive_rate: f64::NAN,
                ..Default::default()
            }
        )
        .is_err()
    );
}
#[test]
fn detector_dwell_unknown_blinks_offscreen_and_stream_gaps() {
    let model = fit(&data(false), &data(false), screen(), FitOptions::default()).unwrap();
    let mut detector = CalibratedDetector::new(model.clone()).unwrap();
    for at in [0, 50] {
        assert!(
            !detector
                .observe(sample(0.94, 0.9, at))
                .unwrap()
                .check_started
        );
    }
    assert!(
        detector
            .observe(sample(0.94, 0.9, 100))
            .unwrap()
            .check_started
    );
    assert!(
        !detector
            .observe(sample(0.94, 0.9, 150))
            .unwrap()
            .check_started
    );
    let unknown = GazeSample {
        features: None,
        quality: 0.0,
        ..sample(0.94, 0.9, 200)
    };
    assert_eq!(detector.observe(unknown).unwrap().state, GazeState::Unknown);
    for at in [250, 300] {
        assert!(
            !detector
                .observe(sample(0.94, 0.9, at))
                .unwrap()
                .check_started
        );
    }
    assert!(
        detector
            .observe(sample(0.94, 0.9, 350))
            .unwrap()
            .check_started
    );
    assert!(
        !detector
            .observe(sample(0.94, 0.9, 1000))
            .unwrap()
            .check_started
    );
    let mut blink = sample(0.94, 0.9, 1050);
    blink
        .features
        .as_mut()
        .unwrap()
        .landmarks_mut()
        .unwrap()
        .eyes[0]
        .aperture = 0.01;
    assert_eq!(detector.observe(blink).unwrap().state, GazeState::Unknown);
    assert!(model.predict(sample(1.2, 1.2, 0)).is_none());
    let mut low = sample(0.94, 0.9, 0);
    low.quality = 0.1;
    assert!(model.predict(low).is_none());
    let mut nan = sample(0.94, 0.9, 0);
    nan.features
        .as_mut()
        .unwrap()
        .landmarks_mut()
        .unwrap()
        .head
        .yaw = f32::NAN;
    assert!(model.predict(nan).is_none());
}
#[test]
fn hysteresis_keeps_a_confirmed_glance_below_enter_but_above_exit() {
    let model = fit(&data(false), &data(false), screen(), FitOptions::default()).unwrap();
    let middle = (750..1000)
        .flat_map(|i| (700..1000).map(move |j| sample(i as f64 / 1000.0, j as f64 / 1000.0, 150)))
        .find(|s| {
            model.predict(*s).is_some_and(|p| {
                p.probability < model.threshold() && p.probability >= model.threshold() * 0.75
            })
        })
        .expect("hysteresis band exists");
    let mut d = CalibratedDetector::new(model).unwrap();
    for at in [0, 50, 100] {
        d.observe(sample(0.94, 0.9, at)).unwrap();
    }
    let update = d.observe(middle).unwrap();
    assert_eq!(update.state, GazeState::Minimap);
    assert!(!update.check_started);
    assert_eq!(
        d.observe(sample(0.5, 0.5, 200)).unwrap().state,
        GazeState::Elsewhere
    );
}

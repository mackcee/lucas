//! Apache-2.0 MediaPipe portrait fixture, not a captured user's webcam frame.
#![cfg(any(feature = "ort", feature = "tract"))]
use lucas_gaze::{
    Camera, Frame, GazeSource, ImageCamera, Landmarker, MediaPipeLandmarker, WebcamGazeSource,
    features, models::VerifiedModels,
};
use std::{path::PathBuf, time::Duration};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
#[cfg(feature = "ort")]
fn backend() -> lucas_gaze::runtime::OrtBackend {
    lucas_gaze::runtime::OrtBackend::new(
        VerifiedModels::load(&root().join("../../models")).unwrap(),
    )
    .unwrap()
}
#[cfg(all(feature = "tract", not(feature = "ort")))]
fn backend() -> lucas_gaze::runtime::TractBackend {
    lucas_gaze::runtime::TractBackend::new(
        VerifiedModels::load(&root().join("../../models")).unwrap(),
    )
    .unwrap()
}
fn portrait() -> Frame {
    let path = root().join("tests/fixtures/portrait.jpg");
    let (w, h) = image::image_dimensions(&path).unwrap();
    let mut camera = ImageCamera::new([(Duration::ZERO, path)].into_iter(), (w, h)).unwrap();
    let mut frame = Frame::new(w, h).unwrap();
    assert!(camera.capture(&mut frame).unwrap());
    frame
}
#[test]
fn portrait_detects_478_points_tracks_iris_and_reacquires() {
    let mut frame = portrait();
    let mut tracker = MediaPipeLandmarker::new(backend());
    for i in 0..5 {
        frame.set_timestamp(Duration::from_millis(i * 33));
        let face = tracker.locate(&frame).unwrap().expect("portrait face");
        assert_eq!(face.points.len(), 478);
        assert!(face.presence > 0.9);
        for p in &face.points[468..478] {
            assert!((0.15..0.85).contains(&p.x));
            assert!((0.1..0.6).contains(&p.y));
        }
        let (features, quality) = features::extract(&face).expect("portrait eyes open");
        assert!(quality > 0.1, "quality={quality}");
        for e in features.eyes {
            assert!(
                (0.2..0.8).contains(&e.horizontal),
                "iris between corners: {e:?}"
            );
            assert!(
                (-0.2..1.2).contains(&e.vertical),
                "iris between lids: {e:?}"
            );
        }
    }
    assert_eq!(tracker.stats().detections, 1);
    assert_eq!(tracker.stats().meshes, 5);
    frame.clear();
    frame.set_timestamp(Duration::from_millis(165));
    assert!(
        tracker.locate(&frame).unwrap().is_none(),
        "blank frame loses track"
    );
    let mut frame = portrait();
    frame.set_timestamp(Duration::from_millis(198));
    assert!(tracker.locate(&frame).unwrap().is_some());
    assert_eq!(tracker.stats().detections, 2);
    // A long frame gap also invalidates the old crop.
    frame.set_timestamp(Duration::from_secs(3));
    assert!(tracker.locate(&frame).unwrap().is_some());
    assert_eq!(tracker.stats().detections, 3);
}
#[test]
fn blank_image_is_unknown_and_file_source_ends() {
    let frame = Frame::new(640, 480).unwrap();
    let mut tracker = MediaPipeLandmarker::new(backend());
    assert!(tracker.locate(&frame).unwrap().is_none());
    assert_eq!(tracker.stats().meshes, 0);
    let path = root().join("tests/fixtures/portrait.jpg");
    let dimensions = image::image_dimensions(&path).unwrap();
    let camera = ImageCamera::new([(Duration::ZERO, path)].into_iter(), dimensions).unwrap();
    let mut source = WebcamGazeSource::new(camera, tracker).unwrap();
    let sample = source.next_sample().unwrap().unwrap();
    assert!(sample.features.is_some() && sample.quality > 0.1);
    assert!(source.next_sample().unwrap().is_none());
}
#[cfg(all(feature = "tract", feature = "ort"))]
#[test]
fn tract_matches_ort_on_real_image() {
    use lucas_gaze::runtime::TractBackend;
    let frame = portrait();
    let mut ort = MediaPipeLandmarker::new(backend());
    let mut tract = MediaPipeLandmarker::new(
        TractBackend::new(VerifiedModels::load(&root().join("../../models")).unwrap()).unwrap(),
    );
    let a = ort.locate(&frame).unwrap().unwrap();
    let b = tract.locate(&frame).unwrap().unwrap();
    for (a, b) in a.points.iter().zip(&b.points) {
        assert!(
            (a.x - b.x).abs() < 0.002 && (a.y - b.y).abs() < 0.002 && (a.z - b.z).abs() < 0.002
        );
    }
}

//! CPU-time regression guard; run with `cargo bench -p lucas-gaze --bench pipeline`.
use cpu_time::ProcessTime;
use lucas_gaze::{
    Camera, Frame, ImageCamera, Landmarker, MediaPipeLandmarker, features,
    models::VerifiedModels,
    runtime::{InferenceBackend, OrtBackend},
};
use std::{hint::black_box, path::Path, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut backend = OrtBackend::new(VerifiedModels::load(&root.join("../../models"))?)?;
    let input: Vec<_> = (0..256 * 256 * 3)
        .map(|i| (i % 251) as f32 / 250.0)
        .collect();
    for _ in 0..10 {
        black_box(backend.mesh(&input)?);
    }
    let start = ProcessTime::now();
    for _ in 0..100 {
        black_box(backend.mesh(&input)?);
    }
    let mesh_ms = start.elapsed().as_secs_f64() * 10.0;
    let mut landmarker = MediaPipeLandmarker::new(backend);
    let path = root.join("tests/fixtures/portrait.jpg");
    let (w, h) = image::image_dimensions(&path)?;
    let mut camera = ImageCamera::new([(Duration::ZERO, path)].into_iter(), (w, h))?;
    let mut frame = Frame::new(w, h)?;
    camera.capture(&mut frame)?;
    for i in 0..10 {
        frame.set_timestamp(Duration::from_millis(i * 33));
        black_box(landmarker.locate(&frame)?.expect("fixture face"));
    }
    let start = ProcessTime::now();
    for i in 10..110 {
        frame.set_timestamp(Duration::from_millis(i * 33));
        let face = landmarker.locate(&frame)?.expect("fixture face");
        black_box(features::extract(&face).expect("usable eyes"));
    }
    let pipeline_ms = start.elapsed().as_secs_f64() * 10.0;
    let stats = landmarker.stats();
    println!(
        "CPU milliseconds/frame: mesh={mesh_ms:.3}, tracked_pipeline={pipeline_ms:.3}; detections={}, meshes={}",
        stats.detections, stats.meshes
    );
    assert_eq!(
        stats.detections, 1,
        "tracking should avoid repeated detection"
    );
    // Portable default is a one-core 30fps budget, not a promise that every CPU
    // reproduces the scout machine. Controlled-runner baseline: env=3.5 => 4.375ms.
    let baseline = std::env::var("LUCAS_MESH_BASELINE_MS")
        .ok()
        .map(|v| v.parse::<f64>())
        .transpose()?
        .unwrap_or(20.0);
    assert!(
        baseline.is_finite() && baseline > 0.0,
        "invalid CPU baseline"
    );
    assert!(
        mesh_ms <= baseline * 1.25,
        "mesh CPU regression exceeds 25% of baseline {baseline:.3}ms"
    );
    assert!(pipeline_ms < 33.333, "pipeline exceeds one core at 30fps");
    Ok(())
}

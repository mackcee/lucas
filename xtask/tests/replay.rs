use lucas_app::{
    script::{Phase, Target, TargetKind},
    trace::{CameraInfo, EndReason, FrameRecord, Header, TraceWriter},
};
use std::{fs::File, process::Command};

#[test]
fn replay_cli_reports_synthetic_trace_and_rejects_missing_end() {
    let path = std::env::temp_dir().join(format!("lucas-replay-test-{}.jsonl", std::process::id()));
    let header = Header {
        schema_version: 1,
        app_version: "test".into(),
        started_unix_ms: 0,
        camera: CameraInfo {
            name: "synthetic camera".into(),
            resolution: [1280, 720],
            fps: 30,
        },
        monitor_resolution: [1920, 1080],
        dpi_scale: 1.0,
        lighting_note: "synthetic".into(),
        script_seconds: 60,
        minimap: lucas_app::settings::MinimapSelection {
            source: lucas_app::settings::MinimapSource::Default,
            minimap_scale: 1.0,
            flipped: false,
        },
    };
    let mut writer = TraceWriter::new(File::create(&path).unwrap(), header).unwrap();
    for i in 0..3 {
        let valid = i < 2;
        writer
            .frame(FrameRecord {
                timestamp_ns: i * 100_000_000,
                source_timestamp_ns: i * 100_000_000,
                features: valid.then_some([i as f32; 10]),
                raw_features: valid.then_some([i as f32; 10]),
                face_found: valid,
                quality: if valid { 0.9 } else { 0.0 },
                cpu_ms: 2.0,
                phase: Phase::Hold,
                target: Some(Target {
                    id: 0,
                    position: [0.9, 0.9],
                    kind: TargetKind::Minimap,
                }),
            })
            .unwrap();
    }
    drop(writer.finish(EndReason::Cancelled).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["replay", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Session: Cancelled",
        "Frames: 3",
        "66.67%",
        "Observed FPS: 10.00",
        "n=2 mean=0.50000 std=0.50000",
        "No valid labelled frames",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    std::fs::write(&path, "{}").unwrap();
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["replay", path.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::remove_file(path).unwrap();
}

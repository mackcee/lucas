use lucas_app::{
    script::{Phase, Script, Target, TargetKind},
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

/// Exercise the public schedule -> numeric writer -> offline CLI handoff. These
/// are synthetic features, not a claim about webcam accuracy or GUI rendering.
#[test]
fn scripted_session_replays_only_valid_focused_holds() {
    let path = std::env::temp_dir().join(format!(
        "lucas-scripted-replay-{}.jsonl",
        std::process::id()
    ));
    let header = Header {
        schema_version: 1,
        app_version: env!("CARGO_PKG_VERSION").into(),
        started_unix_ms: 0,
        camera: CameraInfo {
            name: "SYNTHETIC numeric input; no webcam used".into(),
            resolution: [1280, 720],
            fps: 30,
        },
        monitor_resolution: [1920, 1080],
        dpi_scale: 1.5,
        lighting_note: "Synthetic ten-minute handoff, not a G2 accuracy evaluation".into(),
        script_seconds: 600,
        minimap: lucas_app::settings::MinimapSelection {
            source: lucas_app::settings::MinimapSource::PersistedSettings,
            minimap_scale: 3.0,
            flipped: true,
        },
    };
    let script = Script::new(600, header.monitor_resolution, header.minimap.settings()).unwrap();
    let mut writer = TraceWriter::new(File::create(&path).unwrap(), header).unwrap();
    for i in 0..18_000_u64 {
        let timestamp_ns = i * 1_000_000_000 / 30;
        let step = script
            .at(std::time::Duration::from_nanos(timestamp_ns))
            .unwrap();
        // Simulate one second of losing focus during the first elsewhere hold.
        let phase = if (120..150).contains(&i) {
            Phase::Unfocused
        } else {
            step.phase
        };
        let target = (phase == Phase::Hold).then_some(step.target);
        let value = match target.map(|target| target.kind) {
            Some(TargetKind::Minimap) => 0.8,
            Some(TargetKind::Elsewhere) => 0.2,
            None => 0.99, // Must not leak into either target's distribution.
        };
        let valid = !i.is_multiple_of(10);
        writer
            .frame(FrameRecord {
                timestamp_ns,
                source_timestamp_ns: timestamp_ns + 5_000_000_000,
                features: valid.then_some([value; 10]),
                raw_features: valid.then_some([value; 10]),
                face_found: valid,
                quality: if valid { 0.9 } else { 0.0 },
                cpu_ms: 2.0,
                phase,
                target,
            })
            .unwrap();
    }
    drop(writer.finish(EndReason::Completed).unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let trace = lucas_app::trace::read_trace(bytes.as_slice()).unwrap();
    assert_eq!(trace.header.minimap.minimap_scale, 3.0);
    assert!(trace.header.minimap.flipped);
    let summary = lucas_app::trace::summarize(&trace).unwrap();
    assert_eq!(summary.valid_frames, 16_200);
    assert_eq!(summary.distributions[&TargetKind::Minimap][0].count, 5_400);
    assert_eq!(summary.distributions[&TargetKind::Elsewhere][0].count, 5_373);
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("replay")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Session: Completed",
        "Frames: 18000",
        "Valid-frame share: 90.00%",
        "Observed FPS: 30.00",
        "n=5400 mean=0.80000 std=0.00000",
        "n=5373 mean=0.20000 std=0.00000",
        "no detection-rate evaluation",
    ] {
        assert!(text.contains(expected), "{text}");
    }

    // A real interrupted recording, with valid header and frames but no end.
    let end_start = bytes[..bytes.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .unwrap() + 1;
    std::fs::write(&path, &bytes[..end_start]).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("replay")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let error = String::from_utf8(rejected.stderr).unwrap();
    assert!(error.contains("incomplete trace: no end record"), "{error}");

    // Optional reviewer-visible output, enabled explicitly by the test runner.
    if let Some(directory) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("synthetic-scripted-session.jsonl"), &bytes).unwrap();
        std::fs::write(
            directory.join("synthetic-interrupted-session.jsonl"),
            &bytes[..end_start],
        )
        .unwrap();
        std::fs::write(
            directory.join("scripted-replay-transcript.txt"),
            format!(
                "Synthetic public-API schedule/writer -> replay CLI handoff; no webcam or UI exercised.\n\
                 600 seconds at 30 fps, scale 3 flipped, 10% unknown samples, one second unfocused.\n\
                 Valid-frame share is NOT deliberate-glance detection accuracy.\n\n\
                 $ cargo xtask replay synthetic-scripted-session.jsonl\n{text}\n\
                 $ cargo xtask replay synthetic-interrupted-session.jsonl\n{error}"
            ),
        )
        .unwrap();
    }
    std::fs::remove_file(path).unwrap();
}

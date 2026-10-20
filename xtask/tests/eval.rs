use lucas_app::{
    script::{Phase, Script},
    settings::{MinimapSelection, MinimapSource},
    trace::{CameraInfo, EndReason, FrameRecord, Header, TraceWriter},
};
use std::{fs::File, path::Path, process::Command, time::Duration};
fn fixture(path: &Path, bad: bool) {
    let header = Header {
        schema_version: 1,
        app_version: "synthetic".into(),
        started_unix_ms: 0,
        camera: CameraInfo {
            name: "Synthetic; not webcam evidence".into(),
            resolution: [1280, 720],
            fps: 30,
        },
        monitor_resolution: [1920, 1080],
        dpi_scale: 1.0,
        lighting_note: "synthetic".into(),
        script_seconds: 120,
        minimap: MinimapSelection {
            source: MinimapSource::Default,
            minimap_scale: 1.0,
            flipped: false,
        },
    };
    let script = Script::new(120, header.monitor_resolution, header.minimap.settings()).unwrap();
    let mut writer = TraceWriter::new(File::create(path).unwrap(), header).unwrap();
    for i in 0..3600 {
        let ns = i * 1_000_000_000 / 30;
        let step = script.at(Duration::from_nanos(ns)).unwrap();
        let [x, y] = if bad && step.target.id >= 14 {
            [0.5, 0.5]
        } else {
            step.target.position
        };
        let features = Some([
            x as f32, y as f32, 0.2, x as f32, y as f32, 0.2, 0.0, 0.0, 0.0, 3.0,
        ]);
        writer
            .frame(FrameRecord {
                feature_kind: Default::default(),
                timestamp_ns: ns,
                source_timestamp_ns: ns,
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
    writer.finish(EndReason::Completed).unwrap();
}
#[test]
fn eval_cli_go_no_go_multiple_files_and_independent_calibration() {
    let root = std::env::temp_dir().join(format!("lucas-eval-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let good = root.join("good.jsonl");
    let bad = root.join("bad.jsonl");
    fixture(&good, false);
    fixture(&bad, true);
    let run = |args: &[&Path]| {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("eval")
            .args(args)
            .output()
            .unwrap()
    };
    let pass = run(&[&good]);
    assert!(
        pass.status.success(),
        "{}",
        String::from_utf8_lossy(&pass.stderr)
    );
    let text = String::from_utf8(pass.stdout.clone()).unwrap();
    assert!(text.contains("G2: GO"));
    assert!(text.contains("3/3 (100.00%)"));
    assert!(text.contains("0.000/min"));
    let fail = run(&[&bad]);
    assert_eq!(fail.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&fail.stdout).contains("G2: NO-GO"));
    assert_eq!(run(&[&good, &bad]).status.code(), Some(1));
    let separate = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["eval", "--calibration"])
        .arg(&good)
        .arg(&bad)
        .output()
        .unwrap();
    assert_eq!(separate.status.code(), Some(1));
    let same = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["eval", "--calibration"])
        .arg(&good)
        .arg(&good)
        .output()
        .unwrap();
    assert_eq!(same.status.code(), Some(2));
    if let Some(directory) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let dir = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&good, dir.join("synthetic-g2-go.jsonl")).unwrap();
        std::fs::copy(&bad, dir.join("synthetic-g2-no-go.jsonl")).unwrap();
        std::fs::write(dir.join("synthetic-g2-go.txt"), &pass.stdout).unwrap();
        std::fs::write(dir.join("synthetic-g2-no-go.txt"), &fail.stdout).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

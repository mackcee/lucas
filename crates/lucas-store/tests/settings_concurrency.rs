use lucas_store::{CalibrationPreferences, DATABASE_NAME, ScreenSize, Settings, Store};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

fn screen_choice() -> CalibrationPreferences {
    CalibrationPreferences {
        screen_size: ScreenSize::Inches27,
        custom_size_mm: [597, 336],
        viewing_distance_mm: 725,
    }
}

#[test]
#[ignore = "subprocess helper for settings_updates_serialize_across_processes"]
fn settings_writer_process() {
    let store = Store::open_in(std::env::var_os("LUCAS_STORE_TEST_DIRECTORY").unwrap()).unwrap();
    let mut stale = store.load_settings().unwrap();
    println!("writer-ready");
    std::io::stdout().flush().unwrap();
    let mut command = String::new();
    std::io::stdin().read_line(&mut command).unwrap();
    assert_eq!(command.trim(), "save");
    println!("writer-saving");
    std::io::stdout().flush().unwrap();
    match std::env::var("LUCAS_STORE_TEST_WRITER").unwrap().as_str() {
        "settings" => {
            stale.level_index = 3;
            stale.live_client_gate = false;
            store.save_settings(&stale).unwrap();
        }
        "calibration" => store.save_calibration_preferences(screen_choice()).unwrap(),
        _ => panic!("unknown writer"),
    }
    println!("writer-finished");
}

fn wait_for_output(output: &mpsc::Receiver<String>, event: &str) {
    loop {
        let line = output.recv_timeout(Duration::from_secs(10)).unwrap();
        if line.ends_with(event) {
            return;
        }
    }
}

#[test]
fn settings_updates_serialize_across_processes() {
    for writer in ["settings", "calibration"] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in(dir.path()).unwrap();
        store.save_settings(&Settings::default()).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "settings_writer_process",
                "--nocapture",
            ])
            .env("LUCAS_STORE_TEST_DIRECTORY", dir.path())
            .env("LUCAS_STORE_TEST_WRITER", writer)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        wait_for_output(&output, "writer-ready");

        let mut connection = rusqlite::Connection::open(dir.path().join(DATABASE_NAME)).unwrap();
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "save").unwrap();
        wait_for_output(&output, "writer-saving");
        let blocked = matches!(
            output.recv_timeout(Duration::from_millis(300)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        let while_locked = store.load_settings().unwrap();
        let mut concurrent = Settings::default();
        if writer == "settings" {
            concurrent.calibration = screen_choice();
        } else {
            concurrent.level_index = 3;
            concurrent.live_client_gate = false;
        }
        std::fs::write(
            dir.path().join("settings.toml"),
            toml::to_string_pretty(&concurrent).unwrap(),
        )
        .unwrap();
        transaction.commit().unwrap();
        let status = child.wait().unwrap();
        reader.join().unwrap();

        assert!(status.success());
        assert!(blocked, "{writer} writer bypassed the cross-process lock");
        assert_eq!(while_locked, Settings::default());
        let expected = Settings {
            calibration: screen_choice(),
            level_index: 3,
            live_client_gate: false,
            ..Default::default()
        };
        assert_eq!(store.load_settings().unwrap(), expected);
    }
}

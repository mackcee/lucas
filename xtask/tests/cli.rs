#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn help_is_available() {
    for args in [vec![], vec!["--help"], vec!["-h"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Usage: cargo xtask"));
        assert!(stdout.contains("sim|replay|model-conversion"));
    }
}

#[test]
fn placeholder_tasks_do_not_claim_success() {
    for task in ["sim", "replay", "model-conversion"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg(task)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains(&format!("{task} is not implemented yet"))
        );
    }
}

#[test]
fn invalid_invocations_fail_with_usage() {
    for args in [vec!["unknown"], vec!["sim", "extra"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("Usage: cargo xtask")
        );
    }
}

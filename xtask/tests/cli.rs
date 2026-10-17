use std::process::{Command, Output};

fn xtask(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_and_short_seeded_run() {
    let help = xtask(&["sim", "--help"]);
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("--response")
    );
    let args = [
        "sim",
        "--games",
        "1",
        "--minutes",
        "0.25",
        "--means",
        "4,20",
        "--levels",
        "0,1,9",
        "--cap",
        "both",
        "--response",
        "1",
        "--seed",
        "0",
    ];
    let first = xtask(&args);
    assert!(first.status.success(), "{:?}", first.stderr);
    assert_eq!(first.stdout, xtask(&args).stdout);
    let text = String::from_utf8(first.stdout).unwrap();
    assert!(text.contains("seed 0"));
    assert!(text.contains("pulses/game"));
    assert!(text.contains("solo blind"));
    assert_eq!(text.lines().count(), 16); // banner, header, 12 rows, 2 definitions
}

#[test]
fn invalid_arguments_fail_instead_of_silently_using_defaults() {
    for args in [
        vec!["replay"],
        vec!["sim", "--wat"],
        vec!["sim", "--games"],
        vec!["sim", "--games", "0"],
        vec!["sim", "--games", "-1"],
        vec!["sim", "--games", "1.5"],
        vec!["sim", "--minutes", "0"],
        vec!["sim", "--minutes", "NaN"],
        vec!["sim", "--minutes", "inf"],
        vec!["sim", "--minutes", "1441"],
        vec!["sim", "--means", "0"],
        vec!["sim", "--means", "4,NaN"],
        vec!["sim", "--means", ""],
        vec!["sim", "--means", "4,"],
        vec!["sim", "--means", "-3"],
        vec!["sim", "--levels", "10"],
        vec!["sim", "--levels", ""],
        vec!["sim", "--cap", "maybe"],
        vec!["sim", "--response", "1.1"],
        vec!["sim", "--response", "NaN"],
        vec!["sim", "--response", "-0.1"],
        vec!["sim", "--seed", "-1"],
    ] {
        let output = xtask(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8(output.stderr).unwrap().contains("error:"));
    }
}

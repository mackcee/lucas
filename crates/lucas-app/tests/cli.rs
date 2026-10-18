use std::process::Command;

#[test]
fn postgame_requires_a_source() {
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--show-postgame")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr).unwrap().contains("Usage:"));
}

#[test]
fn postgame_missing_database_fails_without_opening_a_window() {
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .args([
            "--show-postgame",
            "this-directory-does-not-exist/games.sqlite3",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("No games.sqlite3")
    );
}

#[test]
fn help_and_bad_options_never_open_a_camera_or_window() {
    let help = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("No images")
    );
    for args in [
        vec!["--seconds"],
        vec!["--seconds", "61"],
        vec!["--seconds", "606"],
        vec!["--models"],
        vec!["--unknown"],
        vec!["--debug-gaze"],
        vec!["--glance-test"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        if matches!(args[0], "--debug-gaze" | "--glance-test") {
            assert_eq!(output.status.code(), Some(2));
            assert_eq!(output.stderr, b"Unknown arguments. Use --help.\n");
        }
    }
}

#[test]
fn prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("LUCAS {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn prints_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Usage: lucas")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn camera_check_is_documented_and_rejects_other_options() {
    let help = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("--check-camera")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .args(["--check-camera", "--seconds", "600"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Usage: lucas --check-camera")
    );
}

#[cfg(not(windows))]
#[test]
fn camera_check_requires_windows_without_opening_a_window() {
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--check-camera")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"LUCAS: --check-camera requires Windows 10/11\n"
    );
}

#[test]
fn prints_argument_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_lucas"))
        .arg("--invalid")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"Unknown arguments. Use --help.\n");
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::OsString;
    use std::fs::OpenOptions;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Console::{
        AllocConsole, CONSOLE_SCREEN_BUFFER_INFO, COORD, FillConsoleOutputCharacterW,
        GetConsoleScreenBufferInfo, ReadConsoleOutputCharacterW, SetConsoleCursorPosition,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DETACHED_PROCESS, GetExitCodeProcess, PROCESS_INFORMATION, STARTUPINFOW,
        WaitForSingleObject,
    };

    #[test]
    fn parent_console_output() {
        if std::env::var_os("LUCAS_CONSOLE_TEST_CHILD").is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "windows::parent_console_output", "--nocapture"])
                .env("LUCAS_CONSOLE_TEST_CHILD", "1")
                .creation_flags(DETACHED_PROCESS)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }

        unsafe {
            assert_ne!(AllocConsole(), 0);
            let console = OpenOptions::new()
                .read(true)
                .write(true)
                .open("CONOUT$")
                .unwrap();
            let handle = console.as_raw_handle();
            let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            assert_ne!(GetConsoleScreenBufferInfo(handle, &mut info), 0);
            let size = info.dwSize.X as u32 * info.dwSize.Y as u32;
            let origin = COORD { X: 0, Y: 0 };
            for (argument, expected, exit_code) in [
                (
                    "--version",
                    format!("LUCAS {}", env!("CARGO_PKG_VERSION")),
                    0,
                ),
                ("--help", "Usage: lucas".to_owned(), 0),
                ("--invalid", "Unknown arguments. Use --help.".to_owned(), 2),
            ] {
                let mut written = 0;
                assert_ne!(
                    FillConsoleOutputCharacterW(handle, b' ' as u16, size, origin, &mut written),
                    0
                );
                assert_ne!(SetConsoleCursorPosition(handle, origin), 0);
                let mut command = OsString::from("\"");
                command.push(env!("CARGO_BIN_EXE_lucas"));
                command.push("\" ");
                command.push(argument);
                let mut command: Vec<_> = command.encode_wide().chain(Some(0)).collect();
                let mut startup: STARTUPINFOW = std::mem::zeroed();
                startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
                let mut process: PROCESS_INFORMATION = std::mem::zeroed();
                assert_ne!(
                    CreateProcessW(
                        std::ptr::null(),
                        command.as_mut_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        0,
                        0,
                        std::ptr::null(),
                        std::ptr::null(),
                        &startup,
                        &mut process,
                    ),
                    0
                );
                let process_handle = OwnedHandle::from_raw_handle(process.hProcess);
                let _thread_handle = OwnedHandle::from_raw_handle(process.hThread);
                assert_eq!(
                    WaitForSingleObject(process_handle.as_raw_handle(), 10_000),
                    0
                );
                let mut actual_exit_code = 0;
                assert_ne!(
                    GetExitCodeProcess(process_handle.as_raw_handle(), &mut actual_exit_code),
                    0
                );
                assert_eq!(actual_exit_code, exit_code);
                let mut buffer = vec![0; size as usize];
                let mut read = 0;
                assert_ne!(
                    ReadConsoleOutputCharacterW(
                        handle,
                        buffer.as_mut_ptr(),
                        size,
                        origin,
                        &mut read
                    ),
                    0
                );
                let text = String::from_utf16_lossy(&buffer[..read as usize]);
                assert!(
                    text.contains(&expected),
                    "{argument} was not visible on the console"
                );
            }
            prints_version();
            prints_help();
            prints_argument_error();
        }
    }
}

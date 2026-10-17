//! Thin dispatcher; camera and recording both require a UI action.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod console;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod debug_ui;

use std::{path::PathBuf, process::ExitCode};

const HELP: &str = "LUCAS gaze diagnostics (Windows)\n\nUsage: lucas --timer\n       lucas --show-postgame <demo|games.sqlite3|data-directory>\n       lucas [--debug-gaze | --glance-test] [--seconds 600] [--models directory]\n       lucas --check-runtime\n       lucas --version\n\nNo arguments starts timer mode in the notification area.\nSet LUCAS_LEAGUE_ROOT for a custom League install folder.\n\nConnect your camera in the debug window, then opt in to a numeric-only glance test.\nDuration: 60..3600 seconds, in multiples of 6. No images are recorded.\nEsc cancels a test and saves a partial trace. No files are uploaded.";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || matches!(args.as_slice(), [flag] if flag == "--timer") {
        start_timer();
        return ExitCode::SUCCESS;
    }
    #[cfg(windows)]
    console::attach_parent();

    if args.first().is_some_and(|flag| flag == "--show-postgame") {
        let source = args.get(1).filter(|_| args.len() == 2).unwrap_or_else(|| {
            eprintln!("Usage: lucas --show-postgame <demo|games.sqlite3|data-directory>");
            std::process::exit(2);
        });
        if let Err(error) = lucas_app::postgame::run_dev(source) {
            eprintln!("Cannot show post-game summary: {error}");
            std::process::exit(1);
        }
        return ExitCode::SUCCESS;
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("LUCAS: {error}");
            ExitCode::FAILURE
        }
    }
}
fn run() -> Result<(), String> {
    let mut seconds = lucas_app::script::DEFAULT_SECONDS;
    let mut models = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("cannot locate executable directory")?
        .join("models");
    let mut check_runtime = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("LUCAS {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            "--debug-gaze" | "--glance-test" => {}
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or("--seconds needs a value")?
                    .parse()
                    .map_err(|_| "--seconds must be an integer")?;
            }
            "--models" => models = PathBuf::from(args.next().ok_or("--models needs a directory")?),
            "--check-runtime" => check_runtime = true,
            _ => {
                eprintln!("Unknown arguments. Use --help.");
                std::process::exit(2);
            }
        }
    }
    lucas_app::script::Script::new(seconds, [1920, 1080]).map_err(str::to_owned)?;
    #[cfg(windows)]
    {
        if check_runtime {
            let verified =
                lucas_gaze::models::VerifiedModels::load(&models).map_err(|e| e.to_string())?;
            lucas_gaze::runtime::OrtBackend::new(verified).map_err(|e| e.to_string())?;
            println!("Pinned models and ONNX Runtime loaded successfully.");
            return Ok(());
        }
        debug_ui::run(seconds, models)
    }
    #[cfg(not(windows))]
    {
        let _ = (seconds, models, check_runtime);
        Err("the gaze debug window requires Windows; use cargo xtask replay <trace> here".into())
    }
}

#[cfg(windows)]
fn start_timer() {
    if let Err(error) = lucas_app::timer::run() {
        // A failed tray startup cannot report through a tray that never opened.
        // Error-only dialog; normal launch and every overlay path never activate.
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let text: Vec<_> = format!("LUCAS could not start: {error}\n\nIf alignment.toml is invalid, move it out of %LOCALAPPDATA%\\lucas and retry.")
            .encode_utf16().chain(Some(0)).collect();
        let title: Vec<_> = "LUCAS".encode_utf16().chain(Some(0)).collect();
        // SAFETY: live terminated strings; no owner window.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn start_timer() {
    eprintln!("LUCAS timer mode requires Windows 10/11. Use --version for build information.");
    std::process::exit(1);
}

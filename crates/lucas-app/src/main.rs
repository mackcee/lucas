//! Tray gaze coaching by default; numeric trace recording remains opt-in.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod camera_check;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod console;
#[cfg(windows)]
mod debug_ui;
#[cfg(windows)]
mod setup_ui;

use std::{path::PathBuf, process::ExitCode};

const HELP: &str = "LUCAS gaze diagnostics (Windows)\n\nUsage: lucas --timer\n       lucas --show-postgame <demo|games.sqlite3|data-directory>\n       lucas --seconds 600 [--models directory]\n       lucas --calibrate [--models directory]\n       lucas --check-runtime\n       lucas --check-camera\n       lucas --check-gaze-model [--models directory]\n       lucas --version\n\nNo arguments starts gaze coaching in the notification area. --timer selects the explicit no-gaze fallback.\nSet LUCAS_LEAGUE_ROOT for a custom League install folder.\n\nConnect your camera in the debug window, then opt in to a numeric-only glance test.\nDuration: 60..600 seconds, in multiples of 6. No images are recorded.\n--check-camera: headless 3-second camera index 0 check; prints format and timing only.\n--check-gaze-model: bounded CUDA model diagnostic on a bundled public face; never opens a camera.\nLUCAS_GAZE_SOURCE=auto (default), unigaze, or mediapipe selects the tracker for all app modes.\nLUCAS_CALIBRATION_DIAGNOSTICS=<directory> explicitly exports numeric full-calibration evidence (no images); disabled by default.\nFor reliable interactive PowerShell GPU output, run models\\unigaze\\check.ps1; it waits and prints captured logs.\nEsc cancels a test and saves a partial trace. No files are uploaded.";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || matches!(args.as_slice(), [flag] if flag == "--timer") {
        start_tray(!args.is_empty());
        return ExitCode::SUCCESS;
    }
    #[cfg(windows)]
    if matches!(args.as_slice(), [flag] if flag == "--runtime-setup" || flag == "--runtime-settings")
    {
        return match setup_ui::run(args[0] == "--runtime-setup") {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("Cannot open LUCAS settings: {error}");
                ExitCode::FAILURE
            }
        };
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

    if args
        .first()
        .is_some_and(|flag| flag == "--runtime-postgame")
    {
        let result = args
            .get(1)
            .filter(|_| args.len() == 2)
            .and_then(|id| id.to_str())
            .ok_or_else(|| "missing game ID".to_owned())
            .and_then(lucas_app::postgame::show_runtime_postgame);
        return match result {
            Ok(true) => ExitCode::from(lucas_app::postgame::DELETE_REQUESTED_EXIT_CODE),
            Ok(false) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Cannot show post-game summary: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let result = if args.iter().any(|flag| flag == "--check-camera") {
        if args.len() != 1 {
            eprintln!("Usage: lucas --check-camera (no other options)");
            return ExitCode::from(2);
        }
        camera_check::run()
    } else {
        run()
    };
    match result {
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
    let mut check_gaze_model = false;
    let mut runtime_calibration = None;
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
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or("--seconds needs a value")?
                    .parse()
                    .map_err(|_| "--seconds must be an integer")?;
            }
            "--models" => models = PathBuf::from(args.next().ok_or("--models needs a directory")?),
            "--check-runtime" => check_runtime = true,
            "--check-gaze-model" => check_gaze_model = true,
            "--runtime-calibrate" => runtime_calibration = Some(None),
            "--runtime-quick" => {
                runtime_calibration =
                    Some(Some(args.next().ok_or("missing calibration snapshot")?));
            }
            // Opens the shared diagnostics/calibration surface, never a camera
            // or recording until the player explicitly starts it.
            "--calibrate" => {}
            _ => {
                eprintln!("Unknown arguments. Use --help.");
                std::process::exit(2);
            }
        }
    }
    if check_gaze_model {
        let arguments: Vec<_> = std::env::args().skip(1).collect();
        let valid = arguments.as_slice() == ["--check-gaze-model"]
            || (arguments.len() == 3
                && arguments[0] == "--check-gaze-model"
                && arguments[1] == "--models")
            || (arguments.len() == 3
                && arguments[0] == "--models"
                && arguments[2] == "--check-gaze-model");
        if !valid {
            return Err(
                "Usage: lucas --check-gaze-model [--models directory] (no other modes)".into(),
            );
        }
        let result = lucas_gaze::unigaze::UniGaze::start(&models)
            .and_then(|mut source| source.check())
            .map_err(|e| e.to_string())?;
        println!("{result}");
        return Ok(());
    }
    lucas_app::script::validate_duration(seconds).map_err(str::to_owned)?;
    #[cfg(windows)]
    {
        if let Some(quick_snapshot) = runtime_calibration {
            let outcome = debug_ui::run_calibration(models, quick_snapshot)?;
            // The UI and camera worker have already shut down. Preserve the
            // typed subprocess result instead of merging cancellation/failure.
            if outcome != lucas_app::runtime::calibration_status::CalibrationOutcome::Passed {
                std::process::exit(outcome as i32);
            }
            return Ok(());
        }
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
        let _ = (seconds, models, check_runtime, runtime_calibration);
        Err("the gaze debug window requires Windows; use cargo xtask replay <trace> here".into())
    }
}

#[cfg(windows)]
fn start_tray(timer: bool) {
    if let Err(error) = if timer {
        lucas_app::timer::run_timer()
    } else {
        lucas_app::timer::run()
    } {
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
fn start_tray(_: bool) {
    eprintln!("LUCAS tray coaching requires Windows 10/11. Use --version for build information.");
    std::process::exit(1);
}

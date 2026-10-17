//! Thin dispatcher; timer and gaze diagnostics belong in separate modules.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" || flag == "-V" => version(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "LUCAS\n\nUsage: lucas [--timer | --version | --help]\n\nOn Windows, no arguments starts timer mode in the notification area.\nSet LUCAS_LEAGUE_ROOT for a custom League install folder."
            );
        }
        [] => start_timer(),
        [flag] if flag == "--timer" => start_timer(),
        _ => {
            eprintln!("Unknown arguments. Use --help.");
            std::process::exit(2);
        }
    }
}

fn version() {
    println!("LUCAS {}", env!("CARGO_PKG_VERSION"));
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

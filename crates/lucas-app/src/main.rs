//! Thin dispatcher; timer and gaze diagnostics belong in separate modules.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod console;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => return start_timer(),
        [flag] if flag == "--timer" => return start_timer(),
        _ => {}
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
        return;
    }

    match args.as_slice() {
        [flag] if flag == "--version" || flag == "-V" => version(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "LUCAS\n\nUsage: lucas [--timer | --show-postgame <demo|games.sqlite3|data-directory> | --version | --help]\n\nOn Windows, no arguments starts timer mode in the notification area.\nSet LUCAS_LEAGUE_ROOT for a custom League install folder."
            );
        }
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

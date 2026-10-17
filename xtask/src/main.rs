//! Development tasks for simulation, replay, and model conversion.

#![forbid(unsafe_code)]

use std::process::ExitCode;

const USAGE: &str =
    "Usage: cargo xtask <sim|replay|model-conversion>\nAll tasks are placeholders in M0.1.";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [command] => match command.to_str() {
            Some("--help" | "-h") => {
                println!("{USAGE}");
                ExitCode::SUCCESS
            }
            Some(task @ ("sim" | "replay" | "model-conversion")) => {
                eprintln!("{task} is not implemented yet (M0.1 scaffold).");
                ExitCode::FAILURE
            }
            _ => {
                eprintln!("Unknown task.\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("Expected a single task name.\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#![forbid(unsafe_code)]

mod models;
mod replay;
mod sim;

#[cfg(test)]
mod regression;

use std::process::ExitCode;

const HELP: &str = "LUCAS development tasks

Usage: cargo xtask sim [options]
       cargo xtask replay <trace.jsonl>
       cargo xtask models <verify [directory] | prepare [--python executable] [--output directory] [--compare reference-directory]>

Simulate gamma(k=2) habitual gaps through lucas-core, with 300 ms glances.
Time is eligible play (no loading, deaths, tracking loss or pauses).

  --games N           Games per cell (positive integer; default 20)
  --minutes M         Eligible minutes/game (50 ms to 24 h; default 30)
  --means S,S,...      Mean habitual inter-glance gaps in seconds
                      (0 < S <= 86400; default 4,6,8,10,15,20)
                      Sampled gaps have a 350 ms floor to separate glances
  --levels L,L,...     Indices into core TRAINING_LEVELS (default 1)
  --cap on|off|both    Prompt-density staircase (default on)
  --response P        Probability of answering a pulse (0..1; default 0.8)
                      Response glance starts 0.4-1.2 s after the pulse
  --seed N            Reproducible unsigned 64-bit seed (default 42)
  -h, --help          Show this help

Levels: 0 baseline (S=10, off); 1..4 ramp (S=delay=10,8,6,5);
        5..8 fade (S=5, delay=6,8,10,15; p=1,1,.75,.5);
        9 graduated (S=5, off). Core supplies jitter and salience.

Example: cargo xtask sim --levels 1,7,8 --cap both --seed 42
See xtask/README.md for model assumptions and metric definitions.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command
        .as_deref()
        .is_none_or(|c| matches!(c, "-h" | "--help"))
    {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if command.as_deref() == Some("replay") {
        let args: Vec<_> = args.collect();
        if args.len() != 1 {
            eprintln!("error: usage: cargo xtask replay <trace.jsonl>");
            return ExitCode::from(2);
        }
        return match replay::run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("replay failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if command.as_deref() == Some("models") {
        return match models::run(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command.as_deref() != Some("sim") {
        eprintln!("error: unknown task: {}\n\n{HELP}", command.unwrap());
        return ExitCode::from(2);
    }
    let args: Vec<_> = args.collect();
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    match sim::Config::parse(args) {
        Ok(config) => match sim::run(&config) {
            Ok(rows) => {
                sim::print_table(&config, &rows);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("simulation failed: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}\nRun cargo xtask sim --help for usage.");
            ExitCode::from(2)
        }
    }
}

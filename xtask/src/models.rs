use std::{path::PathBuf, process::Command};

pub fn run(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models");
    match args.first().map(String::as_str) {
        Some("verify") if args.len() <= 2 => {
            let directory = args.get(1).map(PathBuf::from).unwrap_or(root);
            let models = lucas_gaze::models::VerifiedModels::load(&directory)?;
            println!("Model SHA-256s verified ({} bytes)", models.total_bytes());
        }
        Some("prepare") => {
            let mut python = String::from("python3");
            let mut forwarded = Vec::new();
            let mut args = args[1..].iter();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--python" => python = args.next().ok_or("--python requires an executable")?.clone(),
                    "--output" | "--compare" => { forwarded.push(arg.clone()); forwarded.push(args.next().ok_or("model option requires a directory")?.clone()); }
                    _ => return Err(format!("unknown model option: {arg}").into()),
                }
            }
            let status = Command::new(python).arg(root.join("prepare.py")).args(forwarded).status()?;
            if !status.success() { return Err("model preparation failed; see converter output".into()); }
        }
        _ => return Err("usage: cargo xtask models <verify [directory] | prepare [--python executable] [--output directory] [--compare reference-directory]>".into()),
    }
    Ok(())
}

use lucas_app::{
    eval::{default_screen_measurements, evaluate, screen_description},
    trace::{self, Trace},
};
use std::{fs::File, io::BufReader, path::PathBuf};
const HELP: &str = "Usage: cargo xtask eval [--calibration calibration.jsonl] [--width-mm 531 --height-mm 299 --distance-mm 650] <trace.jsonl...>\nWithout --calibration, whole target occurrences are split 50% fit / 25% threshold / 25% test per class.\nWith --calibration, that file is split 75% fit / 25% threshold; all other trace trials are evaluated.\nDefaults assume a 24-inch 16:9 monitor and 650 mm viewing distance; supply your dimensions for meaningful degrees.\nExit 0: all traces meet G2, 1: G2 failed, 2: invalid inputs. No files are written.";
fn read(path: &PathBuf) -> trace::Result<Trace> {
    trace::read_trace(BufReader::new(File::open(path)?))
}
pub fn run(args: Vec<String>) -> trace::Result<bool> {
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
        println!("{HELP}");
        return Ok(true);
    }
    let (mut size, mut distance) = default_screen_measurements();
    let mut calibration = None;
    let mut paths = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--calibration" => {
                if calibration.is_some() {
                    return Err("--calibration specified twice".into());
                }
                calibration = Some(PathBuf::from(
                    args.next().ok_or("--calibration needs a path")?,
                ));
            }
            "--width-mm" => size[0] = args.next().ok_or("--width-mm needs a value")?.parse()?,
            "--height-mm" => size[1] = args.next().ok_or("--height-mm needs a value")?.parse()?,
            "--distance-mm" => {
                distance = args.next().ok_or("--distance-mm needs a value")?.parse()?
            }
            a if a.starts_with('-') => return Err(format!("unknown option {a}\n{HELP}").into()),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        return Err(HELP.into());
    }
    let separate = calibration.as_ref().map(read).transpose()?;
    let mut pass = true;
    for path in paths {
        if let Some(c) = &calibration
            && c.canonicalize()? == path.canonicalize()?
        {
            return Err("calibration file must not also be evaluated as held-out evidence".into());
        }
        let trace = read(&path)?;
        let run = evaluate(&trace, separate.as_ref(), size, distance)?;
        let e = &run.evaluation;
        let v = run.model.validation();
        println!(
            "{}\n  {}\n  Fit: {} trials; threshold validation: {} trials\n  Validation: {:.2}° mean error; ROC TPR {:.2}% / FPR {:.3}% (target {:.3}%); threshold {:.6}\n  Held-out minimap trials: {}/{} ({:.2}%)\n  False detections: {} / {:.2} minutes elsewhere = {:.3}/min\n  Usable frames: {:.2}%; temporal coverage: {:.2}%\n  G2: {} (>=90% glances, <=1 false/min; >=80% usable/coverage; completed trace)",
            path.display(),
            screen_description(run.model.screen()),
            run.training_trials,
            run.validation_trials,
            v.mean_error_degrees,
            v.true_positive_rate * 100.0,
            v.false_positive_rate * 100.0,
            run.model.target_false_positive_rate() * 100.0,
            run.model.threshold(),
            e.detected_trials,
            e.minimap_trials,
            e.detection_rate() * 100.0,
            e.false_detections,
            e.elsewhere_seconds / 60.0,
            e.false_per_minute(),
            e.usable_fraction * 100.0,
            e.coverage * 100.0,
            if e.passes() { "GO" } else { "NO-GO" }
        );
        pass &= e.passes();
    }
    println!("Empirical result only; synthetic traces are not captain-webcam G2 acceptance.");
    Ok(pass)
}

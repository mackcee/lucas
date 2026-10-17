use std::{fs::File, io::BufReader};

pub fn run(args: Vec<String>) -> lucas_app::trace::Result<()> {
    if args.len() != 1 {
        return Err("usage: cargo xtask replay <trace.jsonl>".into());
    }
    let trace = lucas_app::trace::read_trace(BufReader::new(File::open(&args[0])?))?;
    println!(
        "Camera: {} ({}x{} @ {} fps)\nMonitor: {}x{}; DPI scale: {}\nSession: {:?}",
        trace.header.camera.name,
        trace.header.camera.resolution[0],
        trace.header.camera.resolution[1],
        trace.header.camera.fps,
        trace.header.monitor_resolution[0],
        trace.header.monitor_resolution[1],
        trace.header.dpi_scale,
        trace.end.reason
    );
    lucas_app::trace::summarize(&trace)?.print(std::io::stdout())?;
    Ok(())
}

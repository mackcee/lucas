//! Render the real shared picker from a test-exported settings file, without a camera.
//! Usage: cargo run -p lucas-app --example calibration_picker_evidence -- <settings.toml> <output.ppm>
//! On headless Linux, run under xvfb-run. This is a component capture, not webcam validation.
use eframe::egui;
use lucas_store::Settings;
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Capture {
    settings: Settings,
    output: PathBuf,
    frames: usize,
    started: Instant,
}

impl eframe::App for Capture {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(15),
            "screenshot timed out"
        );
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("Calibration picker · reopened settings");
            ui.add_space(12.0);
            lucas_app::preferences::calibration_picker(
                ui,
                &mut self.settings.calibration,
                [3840, 2160],
            );
        });
        let image = ui.ctx().input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = image {
            let mut file = std::fs::File::create_new(&self.output).unwrap();
            write!(file, "P6\n{} {}\n255\n", image.width(), image.height()).unwrap();
            for pixel in &image.pixels {
                file.write_all(&pixel.to_array()[..3]).unwrap();
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.frames += 1;
        if self.frames == 4 {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ui.ctx().request_repaint_after(Duration::from_millis(30));
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().expect("settings.toml path required"));
    let output = PathBuf::from(args.next().expect("output.ppm path required"));
    assert!(args.next().is_none(), "unexpected arguments");
    let settings: Settings = toml::from_str(&std::fs::read_to_string(input).unwrap()).unwrap();
    settings.validate().unwrap();
    eframe::run_native(
        "LUCAS calibration picker evidence",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([680.0, 230.0]),
            ..Default::default()
        },
        Box::new(move |_| {
            Ok(Box::new(Capture {
                settings,
                output,
                frames: 0,
                started: Instant::now(),
            }))
        }),
    )
}

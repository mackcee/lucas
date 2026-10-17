//! Small tray-launched window. Camera diagnostics and calibration share the
//! existing capture implementation; neither setup nor settings records traces.
use crate::{capture::Worker, debug_ui::DebugApp};
use eframe::{
    App,
    egui::{self, Color32, ViewportCommand},
};
use lucas_app::preferences::{Setup, SetupStep, level_picker};
use lucas_game::{DisplayMode, Win32WindowProbe, WindowProbe};
use lucas_store::{PulseStyle, Settings, Store};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

pub const CANCELLED: u8 = 3;
pub fn run(wizard: bool) -> Result<u8, String> {
    let store = Store::open().map_err(|e| e.to_string())?;
    let folder = store.data_folder().to_owned();
    let setup = wizard
        .then(|| Setup::load(&store))
        .transpose()
        .map_err(|e| e.to_string())?;
    let settings = store.load_settings().map_err(|e| e.to_string())?;
    drop(store);
    let models = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("no executable directory")?
        .join("models");
    let outcome = Arc::new(AtomicU8::new(CANCELLED));
    let result = outcome.clone();
    eframe::run_native(
        if wizard {
            "LUCAS · First-run setup"
        } else {
            "LUCAS · Settings"
        },
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([700.0, 660.0])
                .with_min_inner_size([560.0, 480.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            apply_style(&cc.egui_ctx);
            let window = cc
                .winit_window()
                .ok_or("native window unavailable")?
                .clone();
            let mut app = Controls {
                setup,
                settings,
                folder,
                models,
                window,
                cameras: Vec::new(),
                worker: None,
                calibration: None,
                error: None,
                confirm_delete: false,
                outcome: result,
                closing: false,
            };
            if app.setup.is_some() {
                app.refresh_cameras();
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(outcome.load(Ordering::Acquire))
}

struct Controls {
    setup: Option<Setup>,
    settings: Settings,
    folder: PathBuf,
    models: PathBuf,
    window: Arc<winit::window::Window>,
    cameras: Vec<(u32, String)>,
    worker: Option<Worker>,
    calibration: Option<DebugApp>,
    error: Option<String>,
    confirm_delete: bool,
    outcome: Arc<AtomicU8>,
    closing: bool,
}
impl Controls {
    fn refresh_cameras(&mut self) {
        match nokhwa::query(nokhwa::utils::ApiBackend::MediaFoundation) {
            Ok(list) => {
                self.cameras = list
                    .into_iter()
                    .filter_map(|c| c.index().as_index().ok().map(|i| (i, c.human_name())))
                    .collect();
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!(
                    "Cannot list cameras: {e}. Allow desktop camera access in Windows Settings, then refresh."
                ))
            }
        }
    }
    fn cancel(&mut self, ctx: &egui::Context) {
        self.worker = None;
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    fn camera(&mut self, ui: &mut egui::Ui) {
        ui.label("Sit as you will during play. Use front lighting and close other apps using the webcam.");
        let selected = self.setup.as_ref().unwrap().draft.camera_index;
        let mut index = selected;
        let running = self.worker.as_ref().is_some_and(|w| !w.finished());
        ui.add_enabled_ui(!running, |ui| {
            egui::ComboBox::from_label("Camera")
                .selected_text(
                    self.cameras
                        .iter()
                        .find(|(i, _)| *i == index)
                        .map_or("Choose a camera", |(_, n)| n.as_str()),
                )
                .show_ui(ui, |ui| {
                    for (i, name) in &self.cameras {
                        ui.selectable_value(&mut index, *i, name);
                    }
                });
            if ui.button("Refresh cameras").clicked() {
                self.refresh_cameras();
            }
        });
        if index != selected {
            self.setup.as_mut().unwrap().select_camera(index);
        }
        if self.cameras.is_empty() {
            ui.label("No camera found. Connect one and allow desktop apps in Windows Settings > Privacy > Camera.");
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !running && self.cameras.iter().any(|(i, _)| *i == index),
                    egui::Button::new("Check tracking"),
                )
                .clicked()
            {
                drop(self.worker.take());
                self.worker = Some(Worker::start(index, self.models.clone(), ui.ctx().clone()));
            }
            if ui
                .add_enabled(running, egui::Button::new("Stop camera"))
                .clicked()
            {
                self.worker = None;
            }
        });
        let diagnostic = self
            .worker
            .as_ref()
            .and_then(|w| w.shared.lock().ok()?.latest);
        if let Some(d) =
            diagnostic.filter(|d| running && d.received.elapsed() < Duration::from_secs(1))
        {
            ui.label(if d.face_found && d.sample.features.is_some() {
                "Face found · eyes usable"
            } else {
                "Tracking uncertain — face the camera and add front lighting"
            });
            ui.add(egui::ProgressBar::new(d.sample.quality).text(format!(
                "Tracking quality {:.0}% · not an accuracy score",
                d.sample.quality * 100.0
            )));
            ui.label(format!("{:.1} frames/s", d.fps));
            if d.luminance < 0.15 {
                ui.label("Image is dark. Add a light in front of you.");
            }
        } else {
            ui.label(if running {
                "Waiting for camera frames…"
            } else {
                "Camera stopped. Click Check tracking to test your choice."
            });
        }
        if let Some(error) = self
            .worker
            .as_ref()
            .and_then(|w| w.shared.lock().ok()?.error.clone())
        {
            ui.colored_label(Color32::from_rgb(255, 170, 150), error);
        }
        ui.label("Only a live quality indicator is shown. Images are never saved.");
    }
    fn wizard(&mut self, ui: &mut egui::Ui) {
        let step = self.setup.as_ref().unwrap().step;
        ui.heading(step.title());
        ui.label(format!("Step {} of 6", step.number()));
        ui.add_space(16.0);
        match step {
            SetupStep::Intro => {
                ui.label("LUCAS gently pulses around the minimap when you have not looked at it. Start with help, then reduce it as your habit grows.");
                ui.add_space(12.0);
                ui.strong("Your camera stays private.");
                ui.label("Webcam frames are never stored and nothing leaves this PC. Only settings, calibration and per-game summaries with event timestamps are saved locally.");
                ui.label("Setup covers your camera, Borderless mode and calibration. Closing discards unfinished changes; reopen from the tray to start again with your saved settings.");
            }
            SetupStep::Camera => self.camera(ui),
            SetupStep::Borderless => {
                let mode = Win32WindowProbe.sample().map(|w| w.display_mode);
                ui.strong(match mode {
                    Ok(DisplayMode::Borderless) => "League appears to be in Borderless mode.",
                    Ok(DisplayMode::ExclusiveFullscreen) => {
                        "League appears to be in exclusive fullscreen."
                    }
                    Ok(DisplayMode::Windowed) => "League is windowed. Borderless is recommended.",
                    _ => "Cannot check yet — a visible League game window is needed.",
                });
                ui.label("In League: press Esc, open Video, set Window Mode to Borderless, then apply. A separate overlay cannot appear over exclusive fullscreen.");
                ui.label("This is a best-effort window check; it does not read game pixels or change League settings.");
                if ui.button("Skip check for now").clicked() {
                    self.setup.as_mut().unwrap().skip_borderless();
                }
            }
            SetupStep::Calibration => {
                ui.label("Follow the dots in the existing calibration screen (about 46 seconds). Use your normal playing position and glasses. A validation check must pass before continuing.");
                if self.setup.as_ref().unwrap().calibration_passed() {
                    ui.strong("Calibration passed for your selected camera.");
                }
                if ui.button("Open calibration").clicked() {
                    self.worker = None;
                    self.setup.as_mut().unwrap().begin_calibration();
                    match DebugApp::for_setup(
                        self.window.clone(),
                        self.models.clone(),
                        self.setup.as_ref().unwrap().draft.camera_index,
                    ) {
                        Ok(app) => self.calibration = Some(app),
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            SetupStep::Alignment => {
                ui.label("Alignment needs a running game. In your next game, open the LUCAS tray menu and choose Align minimap…");
                ui.label("Return to League: arrow keys move the outline, Shift + arrows resize it, Enter saves and Esc cancels. The map scale and side come from League's settings.");
                ui.label("Then use the tray's quick gaze check. If alignment changed the gaze zone, recalibrate first. LUCAS will not silently recalibrate or coach with a mismatched fit.");
            }
            SetupStep::Level => {
                ui.label("Start with the longest ramp delay, then select tighter goals when you are ready. Fade levels keep the final goal while reducing help. There is no automatic promotion.");
                level_picker(ui, &mut self.setup.as_mut().unwrap().draft.level_index);
                ui.label("After setup, LUCAS lives in the notification area. Complete a quick check before each game. Use Settings for pulse style, colour and optional sound.");
            }
        }
        ui.add_space(20.0);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(step != SetupStep::Intro, egui::Button::new("Back"))
                .clicked()
            {
                self.worker = None;
                self.setup.as_mut().unwrap().back();
            }
            let can_next = match step {
                SetupStep::Camera => self
                    .cameras
                    .iter()
                    .any(|(i, _)| *i == self.setup.as_ref().unwrap().draft.camera_index),
                SetupStep::Calibration => self.setup.as_ref().unwrap().calibration_passed(),
                _ => true,
            };
            if step == SetupStep::Level {
                if ui.button("Finish — return to tray").clicked() {
                    let result = Store::open().map_err(|e| e.to_string()).and_then(|s| {
                        self.setup
                            .as_ref()
                            .unwrap()
                            .finish(&s)
                            .map_err(|e| e.to_string())
                    });
                    match result {
                        Ok(()) => {
                            self.closing = true;
                            self.outcome.store(0, Ordering::Release);
                            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                        }
                        Err(e) => self.error = Some(format!("Setup not saved: {e}. Retry Finish.")),
                    }
                }
            } else if ui
                .add_enabled(can_next, egui::Button::new("Continue"))
                .clicked()
            {
                self.worker = None;
                self.setup.as_mut().unwrap().advance();
            }
            if ui.button("Close without saving").clicked() {
                self.cancel(ui.ctx());
            }
        });
    }
    fn preferences(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label("Your choices stay on this PC.");
        ui.add_space(16.0);
        level_picker(ui, &mut self.settings.level_index);
        ui.add_space(16.0);
        ui.heading("Minimap pulse");
        egui::ComboBox::from_label("Style")
            .selected_text(style_name(self.settings.pulse.style))
            .show_ui(ui, |ui| {
                for style in [
                    PulseStyle::Ripple,
                    PulseStyle::BreathingBorder,
                    PulseStyle::CornerBrackets,
                    PulseStyle::CometSweep,
                ] {
                    ui.selectable_value(&mut self.settings.pulse.style, style, style_name(style));
                }
            });
        ui.horizontal(|ui| {
            ui.label("Colour");
            ui.color_edit_button_srgb(&mut self.settings.pulse.colour_rgb);
        });
        ui.checkbox(
            &mut self.settings.pulse.sound,
            "Play a sound with each pulse (off by default)",
        );
        ui.label("Appearance changes take effect when you save. Level changes wait until the next game. Solo games never show or play pulses.");
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Save settings").clicked() {
                match Store::open().and_then(|s| s.save_settings(&self.settings)) {
                    Ok(()) => {
                        self.closing = true;
                        self.outcome.store(0, Ordering::Release);
                        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    }
                    Err(e) => {
                        self.error = Some(format!(
                            "Settings not saved: {e}. Free disk space and retry."
                        ))
                    }
                }
            }
            if ui.button("Cancel").clicked() {
                self.cancel(ui.ctx());
            }
        });
        ui.add_space(24.0);
        ui.separator();
        ui.heading("Local data");
        if ui.button("Open data folder").clicked()
            && let Err(e) = std::process::Command::new("explorer.exe")
                .arg(&self.folder)
                .spawn()
        {
            self.error = Some(format!("Cannot open data folder: {e}"));
        }
        if ui.button("Delete all data…").clicked() {
            self.confirm_delete = true;
        }
        if self.confirm_delete {
            ui.label("Permanently delete all LUCAS settings, calibration, summaries, traces and reports? LUCAS will quit first. This cannot be undone.");
            ui.horizontal(|ui| {
                if ui.button("Delete all data and quit").clicked() {
                    self.closing = true;
                    self.outcome.store(
                        lucas_app::postgame::DELETE_REQUESTED_EXIT_CODE,
                        Ordering::Release,
                    );
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
                if ui.button("Keep my data").clicked() {
                    self.confirm_delete = false;
                }
            });
        }
    }
}
impl App for Controls {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(app) = &mut self.calibration {
            app.logic(ctx, frame);
            let passed = app.setup_passed();
            if passed || app.setup_cancelled {
                let profile = app.setup_profile.take();
                self.calibration = None;
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                if let Some(profile) = profile.filter(|_| passed) {
                    self.setup.as_mut().unwrap().calibrated(profile);
                }
            }
        }
    }
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(100));
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            self.cancel(&ctx);
        }
        if let Some(app) = &mut self.calibration {
            app.ui(root, frame);
            return;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(root.style()).inner_margin(20))
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.setup.is_some() {
                        self.wizard(ui);
                    } else {
                        self.preferences(ui);
                    }
                    if let Some(error) = &self.error {
                        ui.add_space(12.0);
                        ui.colored_label(Color32::from_rgb(255, 170, 150), error);
                    }
                });
            });
    }
}
fn apply_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(12.0, 8.0);
        style.spacing.interact_size.y = 28.0;
        for (kind, size) in [
            (egui::TextStyle::Body, 16.0),
            (egui::TextStyle::Button, 15.0),
            (egui::TextStyle::Heading, 25.0),
        ] {
            style
                .text_styles
                .insert(kind, egui::FontId::proportional(size));
        }
    });
}
fn style_name(style: PulseStyle) -> &'static str {
    match style {
        PulseStyle::Ripple => "Ripple",
        PulseStyle::BreathingBorder => "Breathing border",
        PulseStyle::CornerBrackets => "Corner brackets",
        PulseStyle::CometSweep => "Comet sweep",
    }
}

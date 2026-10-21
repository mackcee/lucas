use crate::capture::{Diagnostic, Presentation, Worker};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, RichText, Stroke, Vec2, ViewportCommand};
use lucas_app::{
    calibration::{self, CalibrationScript},
    calibration_diagnostics,
    runtime::calibration_status::CalibrationOutcome,
    script::{Phase, Script, Step, validate_duration},
    settings::MinimapSelection,
    timer::alignment_store::{self, AlignmentStore},
    trace::{CameraInfo, EndReason, Header, feature_names},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Monitor {
    resolution: [u32; 2],
    handle: winit::monitor::MonitorHandle,
}
fn trace_directory() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("lucas").join("traces"))
        .ok_or("LOCALAPPDATA is not set; cannot save a trace".into())
}

pub fn run(seconds: u64, models: PathBuf) -> Result<(), String> {
    run_impl(seconds, models, None, None).map(|_| ())
}
pub fn run_calibration(
    models: PathBuf,
    quick_snapshot: Option<String>,
) -> Result<CalibrationOutcome, String> {
    run_impl(
        600,
        models,
        Some(if quick_snapshot.is_some() {
            RunKind::Quick
        } else {
            RunKind::Calibrate
        }),
        quick_snapshot,
    )
}
fn run_impl(
    seconds: u64,
    models: PathBuf,
    runtime_kind: Option<RunKind>,
    quick_snapshot: Option<String>,
) -> Result<CalibrationOutcome, String> {
    let outcome = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(
        CalibrationOutcome::Cancelled as u8,
    ));
    let result = outcome.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 780.0])
            .with_min_inner_size([600.0, 560.0])
            .with_position([40.0, 40.0]),
        ..Default::default()
    };
    eframe::run_native(
        "LUCAS · Gaze debug",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx.set_zoom_factor(1.0);
            let window = cc
                .winit_window()
                .ok_or("native window unavailable")?
                .clone();
            let handle = window.primary_monitor().ok_or("main monitor unavailable")?;
            let monitor = Monitor {
                resolution: [handle.size().width, handle.size().height],
                handle,
            };
            let mut app = DebugApp {
                window,
                monitor,
                seconds,
                models,
                cameras: Vec::new(),
                selected: 0,
                worker: None,
                lighting: String::new(),
                consent: false,
                error: None,
                pending: None,
                test: None,
                requested: RunKind::Glance,
                measurements: [531.0, 299.0, 650.0],
                calibration_result: None,
                runtime_kind,
                quick_snapshot,
                runtime_outcome: result,
                setup_camera: None,
                setup_profile: None,
                setup_cancelled: false,
            };
            app.refresh_cameras();
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(CalibrationOutcome::from_code(Some(
        outcome.load(std::sync::atomic::Ordering::Acquire) as i32
    ))
    .unwrap_or(CalibrationOutcome::Failed))
}
struct Camera {
    index: u32,
    name: String,
}
#[derive(Clone, Copy, PartialEq)]
enum RunKind {
    Glance,
    Calibrate,
    Quick,
}
enum Schedule {
    Glance(Script),
    Calibration(CalibrationScript),
    Quick(
        CalibrationScript,
        Box<lucas_gaze::calibration::CalibrationModel>,
    ),
}
impl Schedule {
    fn duration(&self) -> Duration {
        match self {
            Self::Glance(s) => s.duration(),
            Self::Calibration(s) | Self::Quick(s, _) => s.duration(),
        }
    }
    fn at(&self, elapsed: Duration) -> Option<Step> {
        match self {
            Self::Glance(s) => s.at(elapsed),
            Self::Calibration(s) | Self::Quick(s, _) => s.at(elapsed).map(|(step, _)| step),
        }
    }
    fn trials(&self) -> u32 {
        match self {
            Self::Glance(s) => s.trials(),
            Self::Calibration(s) | Self::Quick(s, _) => s.targets().len() as u32,
        }
    }
    fn prompt(&self, elapsed: Duration) -> &'static str {
        match self {
            Self::Glance(_) => "Look at the next dot",
            Self::Calibration(s) | Self::Quick(s, _) => {
                if s.at(elapsed).is_some_and(|(_, t)| t.turned) {
                    "Shift naturally: lean, turn, move closer or farther; follow the dot"
                } else {
                    "Sit comfortably; follow the dot. We will ask you to shift partway through"
                }
            }
        }
    }
}
struct Test {
    started: Instant,
    script: Schedule,
    screen: lucas_store::ScreenGeometry,
    diagnostic_directory: Option<PathBuf>,
    paint_bounds_px: Option<[f64; 4]>,
}
struct Pending {
    started: Instant,
}
pub(crate) struct DebugApp {
    window: std::sync::Arc<winit::window::Window>,
    monitor: Monitor,
    seconds: u64,
    models: PathBuf,
    cameras: Vec<Camera>,
    selected: usize,
    worker: Option<Worker>,
    lighting: String,
    consent: bool,
    error: Option<String>,
    pending: Option<Pending>,
    test: Option<Test>,
    requested: RunKind,
    measurements: [f64; 3],
    calibration_result: Option<String>,
    runtime_kind: Option<RunKind>,
    quick_snapshot: Option<String>,
    runtime_outcome: std::sync::Arc<std::sync::atomic::AtomicU8>,
    setup_camera: Option<u32>,
    pub(crate) setup_profile: Option<lucas_store::Calibration>,
    pub(crate) setup_cancelled: bool,
}
impl DebugApp {
    /// Reuses the actual calibration surface inside setup, with no second process
    /// or camera owner. Setup alone commits settings when the wizard finishes.
    pub(crate) fn for_setup(
        window: std::sync::Arc<winit::window::Window>,
        models: PathBuf,
        camera: u32,
    ) -> Result<Self, String> {
        let handle = window.primary_monitor().ok_or("main monitor unavailable")?;
        let monitor = Monitor {
            resolution: [handle.size().width, handle.size().height],
            handle,
        };
        let mut app = Self {
            window,
            monitor,
            seconds: 600,
            models,
            cameras: Vec::new(),
            selected: 0,
            worker: None,
            lighting: String::new(),
            consent: false,
            error: None,
            pending: None,
            test: None,
            requested: RunKind::Calibrate,
            measurements: [531.0, 299.0, 650.0],
            calibration_result: None,
            runtime_kind: Some(RunKind::Calibrate),
            quick_snapshot: None,
            runtime_outcome: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(
                CalibrationOutcome::Cancelled as u8,
            )),
            setup_camera: Some(camera),
            setup_profile: None,
            setup_cancelled: false,
        };
        app.refresh_cameras();
        if !app.cameras.iter().any(|c| c.index == camera) {
            return Err(
                "Selected camera is unavailable. Go back and choose a connected camera.".into(),
            );
        }
        Ok(app)
    }
    pub(crate) fn setup_passed(&self) -> bool {
        self.runtime_outcome
            .load(std::sync::atomic::Ordering::Acquire)
            == CalibrationOutcome::Passed as u8
    }
    fn set_outcome(&self, outcome: CalibrationOutcome) {
        self.runtime_outcome
            .store(outcome as u8, std::sync::atomic::Ordering::Release);
    }
    fn refresh_cameras(&mut self) {
        match nokhwa::query(nokhwa::utils::ApiBackend::MediaFoundation) {
            Ok(cameras) => {
                self.cameras = cameras.into_iter().filter_map(|c| c.index().as_index().ok().map(|index| Camera { index, name: c.human_name() })).collect();
                let preferred = self.setup_camera.unwrap_or_else(|| lucas_store::Store::open().and_then(|s| s.load_settings()).map_or(0, |s| s.camera_index));
                self.selected = self.cameras.iter().position(|c| c.index == preferred).unwrap_or(0);
                self.error = None;
            }
            Err(_) => self.error = Some("Camera list unavailable. Check Windows camera privacy permissions, then Refresh cameras.".into()),
        }
    }
    fn finish_test(&mut self, context: &egui::Context, reason: EndReason) {
        self.set_outcome(if reason == EndReason::Completed {
            CalibrationOutcome::Failed
        } else {
            CalibrationOutcome::Cancelled
        });
        let (observations, diagnostic_frames) = if let Some(worker) = &self.worker {
            let mut state = worker.shared.lock().unwrap();
            state.finish(reason);
            let frames = state.take_calibration_diagnostics();
            (state.take_calibration(), frames)
        } else {
            (Vec::new(), Vec::new())
        };
        if let Some(test) = self.test.take() {
            let mut diagnostic_note = String::new();
            let mut export = |script: &CalibrationScript,
                              model: Option<&lucas_gaze::calibration::CalibrationModel>,
                              error: Option<&str>| {
                diagnostic_note = match calibration_diagnostics::export(
                    test.diagnostic_directory.as_deref(),
                    script,
                    &observations,
                    &diagnostic_frames,
                    model,
                    error,
                    Some(&calibration_diagnostics::Display {
                        monitor: test.screen.clone(),
                        paint_bounds_px: test.paint_bounds_px,
                    }),
                ) {
                    Ok(Some(path)) => format!(
                        "\nNumeric calibration diagnostics saved: {}. No images; share only if you choose.",
                        path.display()
                    ),
                    Ok(None) => String::new(),
                    Err(e) => format!(
                        "\nNumeric diagnostics could not be saved: {e}. Check the export folder; calibration result is unchanged."
                    ),
                };
            };
            let result: lucas_app::trace::Result<Option<String>> = (|| {
                if reason != EndReason::Completed {
                    if let Schedule::Calibration(script) = &test.script {
                        export(
                            script,
                            None,
                            Some("calibration cancelled or tracking stopped"),
                        );
                    }
                    return Ok((!matches!(test.script, Schedule::Glance(_))).then(|| {
                        "Calibration cancelled. Your previous calibration is unchanged.".into()
                    }));
                }
                let (model, validation, save) = match &test.script {
                    Schedule::Glance(_) => return Ok(None),
                    Schedule::Calibration(script) => {
                        let fitted = calibration::fit_session(script, &observations);
                        let error = fitted.as_ref().err().map(|e| e.to_string());
                        export(script, fitted.as_ref().ok(), error.as_deref());
                        let model = fitted?;
                        let validation = model.validation();
                        (model, validation, true)
                    }
                    Schedule::Quick(_, model) => (
                        (**model).clone(),
                        calibration::quick_revalidate(model, &observations)?,
                        false,
                    ),
                };
                let pass = validation.passes(model.target_false_positive_rate());
                if pass && save {
                    let profile = calibration::profile(
                        &model,
                        test.screen.clone(),
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)?
                            .as_millis()
                            .try_into()?,
                    )?;
                    if self.setup_camera.is_some() {
                        self.setup_profile = Some(profile);
                    } else {
                        let store = lucas_store::Store::open()?;
                        store.save_calibration(&profile)?;
                        let mut settings = store.load_settings()?;
                        settings.camera_index = self.cameras[self.selected].index;
                        store.save_settings(&settings)?;
                    }
                }
                if pass && self.runtime_kind.is_some() {
                    self.set_outcome(CalibrationOutcome::Passed);
                    if self.setup_camera.is_none() {
                        context.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
                Ok(Some(format!(
                    "{} · {:.2}° mean error\nMinimap sensitivity {:.1}% · elsewhere false positives {:.2}% · usable frames {:.0}%\n{}\nDistance is assumed; this short validation is not the G2 recording gate.",
                    if pass {
                        "Validation passed"
                    } else {
                        "Validation failed"
                    },
                    validation.mean_error_degrees,
                    validation.true_positive_rate * 100.0,
                    validation.false_positive_rate * 100.0,
                    validation.usable_fraction * 100.0,
                    if pass && save && test.diagnostic_directory.is_some() {
                        "Calibration saved locally. Numeric diagnostic export requested; no images."
                    } else if pass && save {
                        "Calibration saved locally. No images or training samples saved."
                    } else if pass {
                        "Test passed. Saved calibration unchanged."
                    } else if !save {
                        "Test did not pass. Saved calibration unchanged; recalibrate if tracking seems wrong."
                    } else {
                        "Calibration not saved. Check target visibility, screen/minimap alignment and tracking quality. Numeric diagnostics can help distinguish decision overlap from tracking loss."
                    }
                )))
            })();
            match result {
                Ok(Some(text)) => self.calibration_result = Some(text),
                Ok(None) => {}
                Err(e) => {
                    self.calibration_result = Some(format!(
                        "Calibration not saved: {e}. Your previous calibration is unchanged."
                    ))
                }
            }
            if let Some(text) = &mut self.calibration_result {
                text.push_str(&diagnostic_note);
            }
        }
        if self.runtime_kind == Some(RunKind::Quick) {
            // Optional tests return their result to the tray. They never save
            // weights or decide whether the existing calibration remains usable.
            context.send_viewport_cmd(ViewportCommand::Close);
        }
        self.pending = None;
        self.consent = false;
        self.window.set_fullscreen(None);
        context.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(820.0, 780.0)));
        context.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(40.0, 40.0)));
        context.set_cursor_icon(egui::CursorIcon::Default);
    }
    fn arm_test(&mut self, requested: RunKind) {
        self.set_outcome(CalibrationOutcome::Cancelled);
        self.requested = requested;
        self.calibration_result = None;
        self.error = None;
        let Some(handle) = self.window.primary_monitor() else {
            self.error =
                Some("Main monitor unavailable; reconnect the display and try again.".into());
            return;
        };
        self.monitor = Monitor {
            resolution: [handle.size().width, handle.size().height],
            handle,
        };
        self.window
            .set_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(
                self.monitor.handle.clone(),
            ))));
        self.pending = Some(Pending {
            started: Instant::now(),
        });
    }
    fn prepare_test(&mut self, context: &egui::Context) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let viewport = context.input(|i| i.viewport().clone());
        if pending.started.elapsed() > Duration::from_secs(5) {
            self.error = Some("Could not enter fullscreen on the main monitor. Move LUCAS to the main monitor and try again.".into());
            self.finish_test(context, EndReason::Cancelled);
            self.set_outcome(CalibrationOutcome::Failed);
            return;
        }
        let scale = context.pixels_per_point();
        let correct_size = viewport.inner_rect.is_some_and(|r| {
            (r.width() * scale - self.monitor.resolution[0] as f32).abs() < 3.0
                && (r.height() * scale - self.monitor.resolution[1] as f32).abs() < 3.0
                && r.min.x.abs() < 3.0
                && r.min.y.abs() < 3.0
        });
        if viewport.fullscreen != Some(true) || !correct_size || viewport.focused != Some(true) {
            return;
        }
        let result = (|| -> Result<Test, String> {
            let worker = self.worker.as_ref().ok_or("Connect a camera first")?;
            let mut state = worker.shared.lock().unwrap();
            if state
                .latest
                .is_none_or(|d| d.received.elapsed() > Duration::from_secs(1))
                || worker.finished()
            {
                return Err(
                    "No recent camera frames. Reconnect the camera before recording.".into(),
                );
            }
            let minimap =
                MinimapSelection::resolve(&lucas_game::SettingsLocator::windows_defaults(
                    std::env::var_os("LUCAS_LEAGUE_ROOT").map(PathBuf::from),
                ))?;
            let started = Instant::now();
            let position = self.monitor.handle.position();
            let screen = lucas_store::ScreenGeometry {
                left: position.x,
                top: position.y,
                width: self.monitor.resolution[0],
                height: self.monitor.resolution[1],
                scale_factor: scale as f64,
            };
            if self.requested != RunKind::Glance {
                let folder = alignment_store::data_folder().map_err(|e| e.to_string())?;
                let alignments = AlignmentStore::load(&folder).map_err(|e| e.to_string())?;
                let script = CalibrationScript::new(
                    self.monitor.resolution,
                    minimap.settings(),
                    &alignments,
                    [self.measurements[0], self.measurements[1]],
                    self.measurements[2],
                )
                .map_err(|e| e.to_string())?;
                let schedule = if self.requested == RunKind::Quick {
                    let settings = lucas_store::Store::open()
                        .and_then(|s| s.load_settings())
                        .map_err(|e| e.to_string())?;
                    if self.cameras[self.selected].index != settings.camera_index {
                        return Err(
                            "Test gaze must use the calibrated camera; select it or recalibrate."
                                .into(),
                        );
                    }
                    let model = calibration::load(&lucas_store::Store::open().map_err(|e|e.to_string())?, &screen, script.screen().minimap)
                        .map_err(|e|e.to_string())?.ok_or("No passing calibration for this screen/minimap. Run full calibration first.")?;
                    if let Some(expected) = &self.quick_snapshot {
                        calibration::verify_quick_snapshot(
                            &model,
                            self.cameras[self.selected].index,
                            expected,
                        )
                        .map_err(|e| {
                            self.set_outcome(CalibrationOutcome::Changed);
                            e.to_string()
                        })?;
                    }
                    Schedule::Quick(CalibrationScript::quick(&model), Box::new(model))
                } else {
                    Schedule::Calibration(script)
                };
                let diagnostic_directory = (self.requested == RunKind::Calibrate)
                    .then(calibration_diagnostics::directory_from_env)
                    .flatten();
                state.begin_calibration(
                    started,
                    schedule.duration(),
                    diagnostic_directory.is_some(),
                );
                return Ok(Test {
                    started,
                    script: schedule,
                    screen,
                    diagnostic_directory,
                    paint_bounds_px: None,
                });
            }
            let script = Script::new(self.seconds, self.monitor.resolution, minimap.settings())
                .map_err(str::to_owned)?;
            let header = Header {
                schema_version: 2,
                app_version: env!("CARGO_PKG_VERSION").into(),
                started_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_millis()
                    .try_into()
                    .map_err(|_| "clock overflow")?,
                camera: CameraInfo {
                    name: self.cameras[self.selected].name.clone(),
                    resolution: [1280, 720],
                    fps: 30,
                },
                monitor_resolution: self.monitor.resolution,
                dpi_scale: scale as f64,
                lighting_note: self.lighting.clone(),
                script_seconds: self.seconds,
                minimap,
            };
            state
                .start(&trace_directory()?, header, started)
                .map_err(|e| e.to_string())?;
            Ok(Test {
                started,
                script: Schedule::Glance(script),
                screen,
                diagnostic_directory: None,
                paint_bounds_px: None,
            })
        })();
        match result {
            Ok(test) => {
                self.test = Some(test);
                self.pending = None;
            }
            Err(error) => {
                let outcome = if self
                    .runtime_outcome
                    .load(std::sync::atomic::Ordering::Acquire)
                    == CalibrationOutcome::Changed as u8
                {
                    CalibrationOutcome::Changed
                } else {
                    CalibrationOutcome::Failed
                };
                self.error = Some(error);
                self.finish_test(context, EndReason::Cancelled);
                self.set_outcome(outcome);
            }
        }
    }
    fn show_test(&mut self, root: &mut egui::Ui) {
        let context = &root.ctx().clone();
        let expected = self.monitor.resolution;
        let actual = self.window.inner_size();
        if self.window.fullscreen().is_none() || [actual.width, actual.height] != expected {
            self.error = Some("Display geometry changed. Start a new test or calibration.".into());
            self.finish_test(context, EndReason::Cancelled);
            self.set_outcome(CalibrationOutcome::Changed);
            return;
        }
        let test = self.test.as_ref().unwrap();
        let elapsed = test.started.elapsed();
        let focused = context.input(|i| i.viewport().focused == Some(true));
        let cancelled = context.input(|i| i.key_pressed(egui::Key::Escape));
        let is_calibration = !matches!(test.script, Schedule::Glance(_));
        let failed = self.worker.as_ref().is_none_or(|w| {
            let state = w.shared.lock().unwrap();
            w.finished()
                || if is_calibration {
                    !state.is_collecting()
                } else {
                    !state.is_recording()
                }
        });
        let Some(step) = test.script.at(elapsed) else {
            self.finish_test(context, EndReason::Completed);
            return;
        };
        if cancelled || failed {
            self.finish_test(context, EndReason::Cancelled);
            if failed && !cancelled {
                self.set_outcome(CalibrationOutcome::Failed);
            }
            return;
        }
        let phase = if focused {
            step.phase
        } else {
            Phase::Unfocused
        };
        let target = if is_calibration {
            calibration::visible_target(step, focused)
        } else {
            (phase == Phase::Hold).then_some(step.target)
        };
        let shown = Presentation { phase, target };
        self.worker
            .as_ref()
            .unwrap()
            .shared
            .lock()
            .unwrap()
            .present(Instant::now(), shown);
        let mut paint_bounds_px = None;
        // Fixed background brightness across countdown and target holds. No camera
        // preview, animated status meter or gaze marker competes with the target.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_gray(74)))
            .show(root, |ui| {
                let rect = ui.max_rect();
                let scale = f64::from(context.pixels_per_point());
                paint_bounds_px = Some(
                    [rect.left(), rect.top(), rect.right(), rect.bottom()]
                        .map(|x| f64::from(x) * scale),
                );
                let painter = ui.painter();
                if let Some(target) = target {
                    paint_target(painter, rect, target.position);
                } else {
                    let text = if focused {
                        format!(
                            "{}\n{}",
                            test.script.prompt(elapsed),
                            step.remaining.as_secs_f64().ceil() as u32
                        )
                    } else {
                        "Test not focused\nClick this window to return".into()
                    };
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        text,
                        FontId::proportional(28.0),
                        Color32::WHITE,
                    );
                }
                if is_calibration {
                    let latest = self
                        .worker
                        .as_ref()
                        .and_then(|w| w.shared.lock().unwrap().latest);
                    let quality = latest.map_or_else(
                        || "Waiting for camera".into(),
                        |d| {
                            if d.received.elapsed() > Duration::from_secs(1) {
                                "Camera stalled · reconnect and retry".into()
                            } else if d.luminance < 0.15 {
                                "Too dark · add a light in front of you".into()
                            } else {
                                format!(
                                    "Tracking quality {:.0}% · {}",
                                    d.sample.quality * 100.0,
                                    if d.sample.features.is_some() {
                                        "eyes tracked"
                                    } else {
                                        "eyes unknown"
                                    }
                                )
                            }
                        },
                    );
                    painter.text(
                        rect.center_top() + Vec2::new(0.0, 22.0),
                        Align2::CENTER_TOP,
                        quality,
                        FontId::proportional(16.0),
                        Color32::WHITE,
                    );
                    if focused && target.is_some() {
                        painter.text(
                            rect.center_top() + Vec2::new(0.0, 46.0),
                            Align2::CENTER_TOP,
                            test.script.prompt(elapsed),
                            FontId::proportional(16.0),
                            Color32::WHITE,
                        );
                    }
                }
                painter.text(
                    rect.center_bottom() - Vec2::new(0.0, 22.0),
                    Align2::CENTER_BOTTOM,
                    format!(
                        "Target {} / {}   ·   {}",
                        step.target.id + 1,
                        test.script.trials(),
                        if is_calibration {
                            "Esc to cancel without saving"
                        } else {
                            "Esc to stop and save"
                        }
                    ),
                    FontId::proportional(14.0),
                    Color32::from_gray(220),
                );
            });
        if let Some(test) = &mut self.test {
            test.paint_bounds_px = paint_bounds_px;
        }
        context.set_cursor_icon(egui::CursorIcon::None);
    }
    fn show_debug(&mut self, root: &mut egui::Ui) {
        let context = &root.ctx().clone();
        let running = self.worker.as_ref().is_some_and(|w| !w.finished());
        let latest = self
            .worker
            .as_ref()
            .and_then(|w| w.shared.lock().unwrap().latest);
        let error = self
            .worker
            .as_ref()
            .and_then(|w| w.shared.lock().unwrap().error.clone());
        let saved = self
            .worker
            .as_ref()
            .and_then(|w| w.shared.lock().unwrap().saved.clone());
        egui::CentralPanel::default().show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.setup_camera.is_some() && ui.button("Back to setup").clicked() {
                    self.setup_cancelled = true;
                }
                ui.heading(match self.runtime_kind { Some(RunKind::Quick) => "Test gaze (optional)", Some(_) => "Calibrate your minimap gaze", None => "Gaze debug" });
                ui.label(match self.runtime_kind {
                    Some(RunKind::Quick) => "Connect your calibrated camera, then start below. Coaching pauses while this window is open. The result never replaces or revokes your saved calibration.",
                    Some(_) => "Connect your camera, then start below. Only a passing calibration is saved. Coaching pauses while this window is open; close it to cancel.",
                    None => "Check camera placement and lighting before recording your glance test.",
                });
                if let Some(folder) = calibration_diagnostics::directory_from_env() {
                    ui.label(format!("Calibration diagnostics enabled: each full calibration in this session saves gaze/pose numbers to {}. No images or automatic uploads.", folder.display()));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!running && self.pending.is_none() && self.setup_camera.is_none(), |ui| {
                        egui::ComboBox::from_label("Camera").selected_text(self.cameras.get(self.selected).map_or("No camera found", |c| c.name.as_str())).show_ui(ui, |ui| {
                            for (i, camera) in self.cameras.iter().enumerate() { ui.selectable_value(&mut self.selected, i, &camera.name); }
                        });
                        if ui.button("Refresh cameras").clicked() { self.refresh_cameras(); }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(!running && !self.cameras.is_empty(), egui::Button::new("Connect camera")).clicked() {
                        self.worker = Some(Worker::start(self.cameras[self.selected].index, self.models.clone(), context.clone()));
                        self.error = None;
                    }
                    if ui.add_enabled(running, egui::Button::new("Stop camera")).clicked() { self.worker.as_ref().unwrap().stop(); }
                    ui.label("1280 × 720 · MJPEG · 30 fps requested");
                });
                if let Some(source) = self.worker.as_ref().and_then(|w| w.shared.lock().unwrap().source.clone()) {
                    ui.label(source);
                } else {
                    ui.label("Gaze source: UniGaze when installed; otherwise MediaPipe. Set LUCAS_GAZE_SOURCE=mediapipe before launch to select the fallback.");
                }
                if self.cameras.is_empty() { ui.label("Connect a webcam and allow desktop apps under Windows Settings → Privacy → Camera."); }
                ui.add_space(12.0);
                let fresh = running && latest.is_some_and(|d| d.received.elapsed() < Duration::from_secs(1));
                if fresh { diagnostics(ui, latest.unwrap()); }
                else { ui.label(if running { "Waiting for camera frames…" } else { "Camera stopped. No frames are being recorded." }); }
                if let Some(error) = self.error.as_ref().or(error.as_ref()) {
                    ui.add_space(8.0); ui.colored_label(Color32::from_rgb(255,170,150), error);
                }
                ui.add_space(16.0);
                ui.separator();
                if self.runtime_kind == Some(RunKind::Quick) {
                    ui.label("Follow the minimap and centre dots. This four-second test checks your saved calibration without replacing it.");
                } else {
                    ui.heading("Calibrate your minimap gaze");
                    ui.label("About 46 seconds. Follow the dots. Partway through, lean, turn and move a little closer or farther as you normally play. Keep following the dots as you move. Four final targets check the fit, including points just outside the minimap. Use your usual glasses and lighting.");
                    ui.horizontal_wrapped(|ui| {
                        for (label,value) in ["Screen width (mm)","Height (mm)","Assumed distance (mm)"].into_iter().zip(&mut self.measurements) {
                            ui.horizontal(|ui| { ui.label(label); ui.add(egui::DragValue::new(value).range(100.0..=3000.0)); });
                        }
                    });
                    ui.label("Defaults: 24-inch 16:9 screen, 650 mm viewing distance. Use your monitor's physical dimensions, not Windows scaling.");
                    ui.label("Only weights and validation results are saved. Calibration samples stay in memory; the camera never saves images.");
                }
                ui.horizontal(|ui| {
                    if self.runtime_kind != Some(RunKind::Quick)
                        && ui.add_enabled(fresh && self.pending.is_none(), egui::Button::new("Start calibration")).clicked() { self.arm_test(RunKind::Calibrate); }
                    if self.runtime_kind != Some(RunKind::Calibrate)
                        && ui.add_enabled(fresh && self.pending.is_none(), egui::Button::new("Test gaze · 4 seconds")).clicked() { self.arm_test(RunKind::Quick); }
                });
                if let Some(result) = &self.calibration_result { ui.add_space(8.0); ui.label(result); }
                if self.runtime_kind.is_some() { return; }
                ui.add_space(16.0); ui.separator();
                ui.heading("Glance test");
                ui.label("Follow each dot with your eyes; move naturally within the range you calibrated. Look at the countdown between dots. The main monitor will fill the screen. Esc saves a partial test.");
                ui.horizontal(|ui| {
                    ui.label("Duration (seconds)");
                    ui.add(egui::DragValue::new(&mut self.seconds).range(60..=600).speed(6));
                    ui.label("multiples of 6; default 600");
                });
                ui.label("Lighting / camera placement note");
                ui.add(egui::TextEdit::singleline(&mut self.lighting).char_limit(400).hint_text("e.g. daylight, overhead light off, camera top-centre").desired_width(f32::INFINITY));
                ui.add_space(8.0);
                ui.label(RichText::new("This file contains no images.").strong());
                ui.label("Only eye/face features, head pose, quality, timings and shown targets are saved locally. Nothing is uploaded. Share the trace yourself only if you choose.");
                ui.checkbox(&mut self.consent, "I agree to save a numeric trace for this test");
                let valid_duration = validate_duration(self.seconds).is_ok();
                if !valid_duration { ui.label("Choose a duration divisible by 6 seconds."); }
                if ui.add_enabled(fresh && self.consent && valid_duration && self.pending.is_none(), egui::Button::new("Start fullscreen glance test")).clicked() { self.arm_test(RunKind::Glance); }
                if self.pending.is_some() { ui.label("Moving to the main monitor…"); }
                if let Some(path) = saved { ui.label(format!("Trace saved: {}", path.display())); }
                if ui.button("Open traces folder").clicked() {
                    let result = trace_directory().and_then(|path| {
                        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
                        std::process::Command::new("explorer.exe").arg(path).spawn().map_err(|e| e.to_string())?;
                        Ok(())
                    });
                    if let Err(e) = result { self.error = Some(format!("Could not open traces folder: {e}")); }
                }
            });
        });
    }
}
impl eframe::App for DebugApp {
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        if let Some(test) = &self.test {
            if test.started.elapsed() >= test.script.duration() {
                self.finish_test(context, EndReason::Completed);
            } else if context.input(|i| i.viewport().focused != Some(true))
                && let Some(worker) = &self.worker
            {
                worker.shared.lock().unwrap().present(
                    Instant::now(),
                    Presentation {
                        phase: Phase::Unfocused,
                        target: None,
                    },
                );
            }
        }
    }
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        let context = &root.ctx().clone();
        context.request_repaint_after(Duration::from_millis(33));
        if context.input(|i| i.viewport().close_requested()) {
            if self.test.is_some() || self.pending.is_some() {
                self.finish_test(context, EndReason::Cancelled);
            }
            if let Some(worker) = &self.worker {
                worker.shared.lock().unwrap().finish(EndReason::Cancelled);
                worker.stop();
            }
            // Do not classify our intentional worker stop as tracking failure,
            // or replace an already-completed failed result with cancellation.
            return;
        }
        if self.pending.is_some() && context.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.finish_test(context, EndReason::Cancelled);
        }
        self.prepare_test(context);
        if self.test.is_some() {
            self.show_test(root);
        } else {
            self.show_debug(root);
        }
    }
}
impl Drop for DebugApp {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.shared.lock().unwrap().finish(EndReason::Cancelled);
            worker.stop();
        }
    }
}
fn paint_target(painter: &egui::Painter, rect: egui::Rect, position: [f64; 2]) {
    let p = Pos2::new(
        rect.left() + rect.width() * position[0] as f32,
        rect.top() + rect.height() * position[1] as f32,
    );
    painter.circle_stroke(p, 18.0, Stroke::new(3.0, Color32::from_rgb(255, 198, 82)));
    painter.circle_filled(p, 4.0, Color32::WHITE);
}
fn diagnostics(ui: &mut egui::Ui, d: Diagnostic) {
    if d.luminance < 0.15 {
        ui.label("Image too dark. Add a light in front of you, then check tracking quality again.");
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if d.face_found {
                "Face found"
            } else {
                "No face found"
            })
            .strong(),
        );
        ui.label(if d.sample.features.is_some() {
            "Eyes usable"
        } else {
            "Eyes unknown / occluded"
        });
        ui.label(format!(
            "{:.1} fps · {:.2} ms Rust inference CPU/frame",
            d.fps, d.cpu_ms
        ));
    });
    ui.add(egui::ProgressBar::new(d.sample.quality).text(format!(
        "Tracking quality {:.0}% · heuristic, not accuracy",
        d.sample.quality * 100.0
    )));
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label("Raw, uncalibrated features");
            egui::Grid::new("feature-values")
                .striped(true)
                .show(ui, |ui| {
                    for (i, name) in feature_names(d.feature_kind).iter().enumerate() {
                        ui.label(*name);
                        ui.monospace(d.raw.map_or_else(|| "—".into(), |f| format!("{:.4}", f[i])));
                        ui.end_row();
                    }
                });
        });
        ui.add_space(20.0);
        ui.vertical(|ui| {
            ui.label("Coarse camera-space direction");
            let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 150.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_stroke(
                rect.shrink(1.0),
                0.0,
                Stroke::new(1.0, Color32::GRAY),
                egui::StrokeKind::Inside,
            );
            p.line_segment(
                [rect.left_center(), rect.right_center()],
                Stroke::new(1.0, Color32::DARK_GRAY),
            );
            p.line_segment(
                [rect.center_top(), rect.center_bottom()],
                Stroke::new(1.0, Color32::DARK_GRAY),
            );
            if let Some(f) = d.raw {
                let (x, y) = match d.feature_kind {
                    lucas_gaze::features::FeatureKind::MediaPipe => (
                        ((f[0] + f[3]) / 2.0).clamp(0.0, 1.0),
                        ((f[1] + f[4]) / 2.0).clamp(0.0, 1.0),
                    ),
                    lucas_gaze::features::FeatureKind::UniGaze => (
                        (0.5 + f[1] / std::f32::consts::PI).clamp(0.0, 1.0),
                        (0.5 + f[0] / std::f32::consts::PI).clamp(0.0, 1.0),
                    ),
                };
                p.circle_filled(
                    Pos2::new(
                        rect.left() + x * rect.width(),
                        rect.top() + y * rect.height(),
                    ),
                    6.0,
                    Color32::from_rgb(255, 198, 82),
                );
            }
            ui.label("Camera coordinates, not a screen point.");
            ui.label("Run calibration below to learn your minimap gaze.");
            ui.label("CPU = capture / inference thread time.");
        });
    });
}

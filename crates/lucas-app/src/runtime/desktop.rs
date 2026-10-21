//! Windows process/UI boundary. Calibration is offered, never forced over a
//! live game. Subprocesses give eframe its own main thread and camera ownership.
use super::calibration_status::{CalibrationOutcome, CalibrationStatus, Geometry, GeometrySource};
use super::{CalibrationState, Inputs, Runtime, Tick, native_camera::CameraWorker};
use crate::calibration::{self, CalibrationIssue};
use lucas_geometry::Rect;
use lucas_store::{ScreenGeometry, Store};
use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = crate::trace::Result<T>;
struct CalibrationTask {
    child: Child,
    quick: bool,
    generation: u64,
}
pub struct Desktop {
    pub runtime: Runtime,
    camera: Option<CameraWorker>,
    calibration: Option<CalibrationTask>,
    card: Option<Child>,
    controls: Option<(Child, bool)>,
    pub controls_notice: Option<String>,
    delete_requested: bool,
    calibration_status: CalibrationStatus,
    generation: u64,
    origin: Instant,
    models: PathBuf,
    pending: Vec<lucas_store::GameSummary>,
}
impl Desktop {
    pub fn new(origin: Instant) -> Result<Self> {
        let store = Store::open()?;
        let mut runtime = Runtime::new(store.load_settings()?)?;
        // Provisional only: actual game geometry must still match before use.
        let mut calibration_status = CalibrationStatus::default();
        let model = calibration_status.loaded(calibration::load_explained(&store, None));
        runtime.install_calibration(model)?;
        Ok(Self {
            runtime,
            camera: None,
            calibration: None,
            card: None,
            controls: None,
            controls_notice: None,
            delete_requested: false,
            calibration_status,
            generation: 0,
            origin,
            models: std::env::current_exe()?
                .parent()
                .ok_or("no executable directory")?
                .join("models"),
            pending: Vec::new(),
        })
    }
    pub fn calibration_issue(&self) -> Option<&CalibrationIssue> {
        self.calibration_status.issue()
    }
    pub fn calibration_message(&self) -> String {
        self.calibration_status.tray_message(self.calibrating())
    }
    pub fn editing(&self) -> bool {
        self.controls.is_some()
    }
    pub fn launch_controls(&mut self, setup: bool) -> Result<()> {
        if self.editing() || self.calibrating() {
            return Ok(());
        }
        self.camera = None;
        self.close_card();
        if setup {
            // Explicit setup includes recalibration; stale readiness must not
            // return when setup is cancelled or the camera choice changes.
            self.runtime.install_calibration(None)?;
            self.calibration_status
                .block(CalibrationIssue::SetupRunning);
        }
        let child = Command::new(std::env::current_exe()?)
            .arg(if setup {
                "--runtime-setup"
            } else {
                "--runtime-settings"
            })
            .spawn()
            .inspect_err(|error| {
                if setup {
                    self.calibration_status
                        .block(CalibrationIssue::WindowError(error.to_string()));
                }
            })?;
        self.controls = Some((child, setup));
        Ok(())
    }
    pub fn calibrating(&self) -> bool {
        self.calibration.is_some()
    }
    pub fn poll(&mut self, at: Duration, snapshot: Option<&lucas_game::Snapshot>) -> Result<()> {
        let previous = self.runtime.sequence;
        self.runtime.poll(at, snapshot, unix_ms())?;
        self.pending.extend(self.runtime.take_summaries());
        // A new session invalidates a quick check launched for the previous one.
        if previous != self.runtime.sequence {
            self.generation += 1;
            self.camera = None;
            self.close_card();
        }
        if !self.runtime.in_session() {
            self.camera = None;
        }
        Ok(())
    }
    pub fn geometry(
        &mut self,
        screen: ScreenGeometry,
        rect: Rect,
        source: GeometrySource,
    ) -> Result<()> {
        let map = [
            (rect.left() - screen.left as f64) / screen.width as f64,
            (rect.top() - screen.top as f64) / screen.height as f64,
            (rect.right() - screen.left as f64) / screen.width as f64,
            (rect.bottom() - screen.top as f64) / screen.height as f64,
        ];
        let update = self.calibration_status.update_geometry(
            Geometry {
                screen,
                minimap: map,
            },
            source,
            self.runtime.calibration(),
            self.calibrating(),
        );
        if update.changed {
            self.generation += 1;
        }
        if update.reload {
            self.reload()?;
        }
        Ok(())
    }
    fn reload(&mut self) -> Result<()> {
        let store = Store::open()?;
        self.runtime.settings.camera_index = store.load_settings()?.camera_index;
        let geometry = self
            .calibration_status
            .geometry()
            .map(|g| (&g.screen, g.minimap));
        let result = calibration::load_explained(&store, geometry);
        let model = self.calibration_status.loaded(result);
        self.runtime.install_calibration(model)?;
        Ok(())
    }
    pub fn launch_calibration(&mut self) -> Result<()> {
        if self.calibration.is_some() || self.editing() {
            return Ok(());
        }
        self.camera = None; // joined and released before the child can open it
        if self.runtime.calibration() == CalibrationState::QuickCheck && self.runtime.in_session() {
            self.reload()?;
        }
        let quick = self.runtime.begin_calibration()?;
        if !quick {
            self.calibration_status
                .block(CalibrationIssue::FullCalibrationRunning);
        }
        let mut command = Command::new(std::env::current_exe()?);
        if quick {
            let model = self
                .runtime
                .model
                .as_ref()
                .ok_or("calibration unavailable")?;
            command
                .arg("--runtime-quick")
                .arg(calibration::quick_snapshot_id(
                    model,
                    self.runtime.settings().camera_index,
                ));
        } else {
            command.arg("--runtime-calibrate");
        }
        let child = command.spawn().inspect_err(|error| {
            self.calibration_status
                .block(CalibrationIssue::WindowError(error.to_string()));
        })?;
        self.calibration = Some(CalibrationTask {
            child,
            quick,
            generation: self.generation,
        });
        Ok(())
    }
    pub fn tick(&mut self, now: Duration, mut inputs: Inputs) -> Result<Tick> {
        if let Some((child, _)) = &mut self.controls
            && let Some(status) = child.try_wait()?
        {
            let (_, setup) = self.controls.take().unwrap();
            self.delete_requested |=
                status.code() == Some(crate::postgame::DELETE_REQUESTED_EXIT_CODE as i32);
            if status.success() {
                let settings = Store::open()?.load_settings()?;
                self.runtime.update_preferences(&settings)?;
                if setup {
                    self.reload()?;
                }
                self.controls_notice = Some(
                    if setup {
                        "Setup complete — align the minimap and quick-check before play"
                    } else {
                        "Settings saved — training level applies next game"
                    }
                    .into(),
                );
            } else if !self.delete_requested {
                if setup {
                    self.calibration_status.block(if status.code() == Some(3) {
                        CalibrationIssue::SetupCancelled
                    } else {
                        CalibrationIssue::WindowError(format!(
                            "Setup exited with {status}; reopen setup."
                        ))
                    });
                }
                self.controls_notice = Some(
                    if status.code() == Some(3) {
                        "Settings closed; reopen setup from the tray if needed"
                    } else {
                        "Settings window failed; reopen it from the tray to retry"
                    }
                    .into(),
                );
            }
        }
        if let Some(task) = &mut self.calibration
            && let Some(status) = task.child.try_wait()?
        {
            let task = self.calibration.take().unwrap();
            let outcome = CalibrationOutcome::from_code(status.code());
            if task.quick {
                let same_generation = task.generation == self.generation;
                self.runtime
                    .revalidated(status.success() && same_generation)?;
                self.calibration_status
                    .quick_finished(outcome, same_generation);
            } else if status.success() {
                self.reload()?;
            } else {
                self.calibration_status
                    .block(outcome.and_then(|o| o.issue(false)).unwrap_or_else(|| {
                        CalibrationIssue::WindowError(format!(
                            "Calibration exited with {status}; retry full calibration."
                        ))
                    }));
            }
        }
        inputs.paused |= self.calibrating() || self.editing() || self.delete_requested;
        let wanted = self.runtime.camera_wanted(
            inputs.window_exists,
            inputs.gaze_mode,
            self.calibrating() || self.editing() || self.delete_requested,
        );
        if !wanted {
            self.camera = None;
        } else if self.camera.is_none() {
            self.camera = Some(CameraWorker::start(
                self.runtime.settings().camera_index,
                self.models.clone(),
                self.origin,
                self.runtime
                    .model
                    .as_ref()
                    .ok_or("calibration unavailable")?
                    .feature_kind(),
            )?);
        }
        if let Some(notice) = self.camera.as_ref().and_then(CameraWorker::take_notice) {
            self.controls_notice = Some(notice);
        }
        let sample = self.camera.as_ref().and_then(CameraWorker::take_sample);
        let previous = self.runtime.sequence;
        let tick = self.runtime.tick(now, inputs, sample, unix_ms())?;
        if previous != self.runtime.sequence {
            self.generation += 1;
            self.camera = None;
        }
        self.pending.extend(self.runtime.take_summaries());
        if !self.runtime.in_session() {
            self.camera = None;
        }
        if self.runtime.postgame_ready() && !self.editing() && !self.delete_requested {
            for summary in &self.pending {
                Store::open()?.save_summary(summary)?;
            }
            if let Some(id) = self.pending.last().map(|s| s.id.clone()) {
                self.close_card();
                self.card = Some(
                    Command::new(std::env::current_exe()?)
                        .arg("--runtime-postgame")
                        .arg(id)
                        .spawn()?,
                );
            }
            self.pending.clear();
        } else if self.runtime.in_session() {
            self.close_card();
        }
        Ok(tick)
    }
    pub fn deletion_requested(&mut self) -> Result<bool> {
        if let Some(child) = &mut self.card
            && let Some(status) = child.try_wait()?
        {
            self.card = None;
            self.delete_requested |=
                status.code() == Some(crate::postgame::DELETE_REQUESTED_EXIT_CODE as i32);
        }
        Ok(self.delete_requested)
    }
    fn close_card(&mut self) {
        if let Some(mut child) = self.card.take() {
            let _ = child.kill();
            if let Ok(status) = child.wait() {
                self.delete_requested |=
                    status.code() == Some(crate::postgame::DELETE_REQUESTED_EXIT_CODE as i32);
            }
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.camera = None;
        if let Some(mut task) = self.calibration.take() {
            let _ = task.child.kill();
            let _ = task.child.wait();
        }
        if let Some((mut child, _)) = self.controls.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.close_card();
    }
}
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

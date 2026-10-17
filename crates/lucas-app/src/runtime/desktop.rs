//! Windows process/UI boundary. Calibration is offered, never forced over a
//! live game. Subprocesses give eframe its own main thread and camera ownership.
use super::{CalibrationState, Inputs, Runtime, Tick, native_camera::CameraWorker};
use crate::calibration;
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
    delete_requested: bool,
    geometry: Option<(ScreenGeometry, [f64; 4])>,
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
        let model = store
            .load_calibration()
            .ok()
            .flatten()
            .and_then(|c| lucas_gaze::calibration::CalibrationModel::from_bytes(&c.model).ok());
        runtime.install_calibration(model)?;
        Ok(Self {
            runtime,
            camera: None,
            calibration: None,
            card: None,
            delete_requested: false,
            geometry: None,
            generation: 0,
            origin,
            models: std::env::current_exe()?
                .parent()
                .ok_or("no executable directory")?
                .join("models"),
            pending: Vec::new(),
        })
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
    pub fn geometry(&mut self, screen: ScreenGeometry, rect: Rect) -> Result<()> {
        let map = [
            (rect.left() - screen.left as f64) / screen.width as f64,
            (rect.top() - screen.top as f64) / screen.height as f64,
            (rect.right() - screen.left as f64) / screen.width as f64,
            (rect.bottom() - screen.top as f64) / screen.height as f64,
        ];
        let geometry = (screen, map);
        if self.geometry.as_ref() != Some(&geometry) {
            self.generation += 1;
            self.geometry = Some(geometry);
            if self.runtime.calibration() != CalibrationState::Needed && !self.calibrating() {
                self.reload()?;
            }
        }
        Ok(())
    }
    fn reload(&mut self) -> Result<()> {
        self.runtime.settings.camera_index = Store::open()?.load_settings()?.camera_index;
        let model = if let Some((screen, map)) = &self.geometry {
            calibration::load(&Store::open()?, screen, *map).unwrap_or(None)
        } else {
            Store::open()?
                .load_calibration()
                .ok()
                .flatten()
                .and_then(|c| lucas_gaze::calibration::CalibrationModel::from_bytes(&c.model).ok())
        };
        self.runtime.install_calibration(model)?;
        Ok(())
    }
    pub fn launch_calibration(&mut self) -> Result<()> {
        if self.calibration.is_some() {
            return Ok(());
        }
        self.camera = None; // joined and released before the child can open it
        if self.runtime.calibration() == CalibrationState::QuickCheck && self.runtime.in_session() {
            self.reload()?;
        }
        let quick = self.runtime.begin_calibration()?;
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
        let child = command.spawn()?;
        self.calibration = Some(CalibrationTask {
            child,
            quick,
            generation: self.generation,
        });
        Ok(())
    }
    pub fn tick(&mut self, now: Duration, mut inputs: Inputs) -> Result<Tick> {
        if let Some(task) = &mut self.calibration
            && let Some(status) = task.child.try_wait()?
        {
            let task = self.calibration.take().unwrap();
            if task.quick {
                self.runtime
                    .revalidated(status.success() && task.generation == self.generation)?;
            } else if status.success() {
                self.reload()?;
            }
        }
        inputs.paused |= self.calibrating();
        let wanted =
            self.runtime
                .camera_wanted(inputs.window_exists, inputs.gaze_mode, self.calibrating());
        if !wanted {
            self.camera = None;
        } else if self.camera.is_none() {
            self.camera = Some(CameraWorker::start(
                self.runtime.settings().camera_index,
                self.models.clone(),
                self.origin,
            )?);
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
        if self.runtime.postgame_ready() {
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
        self.close_card();
    }
}
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

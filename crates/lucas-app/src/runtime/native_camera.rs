//! A dedicated camera/inference thread with a single numeric latest-sample slot.
//! Disabling waits for acquisition to finish and drops the source before handing
//! the device to calibration. No RGB queue, recording or persistence exists here.
use lucas_gaze::{
    GazeSample, GazeSource,
    camera::NokhwaCamera,
    features::FeatureKind,
    latest::LatestCamera,
    tracker::{ModelGazeSource, Selection, Tracker},
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct CameraWorker {
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<GazeSample>>>,
    handle: Option<JoinHandle<()>>,
    notice: Arc<Mutex<Option<String>>>,
}
impl CameraWorker {
    pub fn start(
        index: u32,
        models: PathBuf,
        origin: Instant,
        expected: FeatureKind,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(None));
        let (signal, slot) = (stop.clone(), latest.clone());
        let notice = Arc::new(Mutex::new(None));
        let message = notice.clone();
        let handle = thread::Builder::new()
            .name("lucas-gaze".into())
            .spawn(move || {
                let run = || -> lucas_gaze::Result<()> {
                    let tracker = Tracker::load_cancellable(&models, Selection::from_env()?, Some(&signal))?;
                    *message.lock().unwrap() = Some(tracker.description().into());
                    if tracker.kind() != expected {
                        return Err(lucas_gaze::Error::Source("Gaze source changed. Run a full calibration with the selected source before playing.".into()));
                    }
                    if signal.load(Ordering::Acquire) { return Ok(()); }
                    let camera = LatestCamera::start(move || {
                        let camera = NokhwaCamera::open_c270(index)?;
                        let origin = camera.timestamp_origin();
                        Ok((camera, origin))
                    })?;
                    let camera_origin = camera.timestamp_origin();
                    let mut source = ModelGazeSource::new(camera, tracker)?;
                    while !signal.load(Ordering::Acquire) {
                        let Some(mut sample) = source.next_sample()? else {
                            break;
                        };
                        sample.timestamp = camera_origin
                            .checked_add(sample.timestamp)
                            .and_then(|acquired| acquired.checked_duration_since(origin))
                            .ok_or(lucas_gaze::Error::InvalidTimestamp)?;
                        if sample.timestamp <= origin.elapsed()
                            && let Ok(mut slot) = slot.lock()
                        {
                            *slot = Some(sample);
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    Ok(())
                };
                // Failure remains unknown and is visible, never a silent CPU
                // fallback. Switching modes/recalibrating explicitly retries.
                if let Err(error) = run() {
                    *message.lock().unwrap() = Some(format!("Gaze stopped: {error}"));
                    *slot.lock().unwrap() = None;
                }
            })?;
        Ok(Self {
            stop,
            latest,
            handle: Some(handle),
            notice,
        })
    }
    pub fn take_notice(&self) -> Option<String> {
        self.notice.lock().ok()?.take()
    }
    pub fn take_sample(&self) -> Option<GazeSample> {
        self.latest.lock().ok()?.take()
    }
}
impl Drop for CameraWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

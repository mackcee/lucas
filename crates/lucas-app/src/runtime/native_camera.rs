//! A dedicated camera/inference thread with a single numeric latest-sample slot.
//! Disabling waits for acquisition to finish and drops the source before handing
//! the device to calibration. No RGB queue, recording or persistence exists here.
use super::camera::CaptureSession;
use lucas_gaze::{
    GazeSample, MediaPipeLandmarker, WebcamGazeSource, camera::NokhwaCamera,
    models::VerifiedModels, runtime::OrtBackend,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct CameraWorker {
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<GazeSample>>>,
    handle: Option<JoinHandle<()>>,
}
impl CameraWorker {
    pub fn start(index: u32, models: PathBuf, origin: Instant) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(None));
        let (signal, slot) = (stop.clone(), latest.clone());
        let handle = thread::Builder::new()
            .name("lucas-gaze".into())
            .spawn(move || {
                let run = || -> lucas_gaze::Result<()> {
                    let backend = OrtBackend::new(VerifiedModels::load(&models)?)?;
                    let camera = NokhwaCamera::open_c270(index)?;
                    // Pair UNIX acquisition timestamps with the app's Instant once;
                    // never stamp inference completion as a gaze observation.
                    let wall = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| lucas_gaze::Error::InvalidTimestamp)?;
                    let mono = origin.elapsed();
                    let epoch = camera.timestamp_epoch();
                    let mut source = CaptureSession::default();
                    source.set_wanted(true, || {
                        WebcamGazeSource::new(camera, MediaPipeLandmarker::new(backend))
                    })?;
                    while !signal.load(Ordering::Acquire) {
                        let Some(mut sample) = source.next_sample()? else {
                            break;
                        };
                        let acquired = epoch + sample.timestamp;
                        sample.timestamp = if acquired >= wall {
                            mono.checked_add(acquired - wall)
                        } else {
                            mono.checked_sub(wall - acquired)
                        }
                        .ok_or(lucas_gaze::Error::InvalidTimestamp)?;
                        if sample.timestamp <= origin.elapsed()
                            && let Ok(mut slot) = slot.lock()
                        {
                            *slot = Some(sample);
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    source.set_wanted(false, || unreachable!())?;
                    Ok(())
                };
                // Failure is represented as unknown; the tray offers a retry by
                // switching modes, never silently treating failure as an away look.
                let _ = run();
            })?;
        Ok(Self {
            stop,
            latest,
            handle: Some(handle),
        })
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

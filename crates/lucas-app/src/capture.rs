//! Dedicated below-normal-priority capture/inference thread. Shared state carries
//! only numbers; the gaze crate clears RGB buffers immediately after inference.
use lucas_app::{
    script::{Phase, Target},
    trace::{self, EndReason, FrameRecord, Header, TraceWriter},
};
use lucas_gaze::{
    FaceLandmarks, Frame, GazeSample, GazeSource, Landmarker, MediaPipeLandmarker,
    WebcamGazeSource, camera::NokhwaCamera, features, models::VerifiedModels, runtime::OrtBackend,
};
use std::{
    fs::{File, OpenOptions},
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct Diagnostic {
    pub sample: GazeSample,
    pub raw: Option<[f32; 10]>,
    pub face_found: bool,
    pub cpu_ms: f64,
    pub fps: f64,
    pub received: Instant,
    /// Mean luma sampled in RAM; no pixels cross the capture-thread boundary.
    pub luminance: f32,
}
#[derive(Clone, Copy, PartialEq)]
pub struct Presentation {
    pub phase: Phase,
    pub target: Option<Target>,
}

struct Recording {
    writer: TraceWriter<BufWriter<File>>,
    started: Instant,
    duration: Duration,
    presentations: Vec<(Instant, Presentation)>,
}
struct Collection {
    started: Instant,
    duration: Duration,
    presentations: Vec<(Instant, Presentation)>,
    observations: Vec<lucas_app::calibration::Observation>,
}
#[derive(Default)]
pub struct Shared {
    pub latest: Option<Diagnostic>,
    pub error: Option<String>,
    pub saved: Option<PathBuf>,
    recording: Option<Recording>,
    recording_path: Option<PathBuf>,
    collection: Option<Collection>,
}
impl Shared {
    pub fn begin_calibration(&mut self, started: Instant, duration: Duration) {
        self.collection = Some(Collection {
            started,
            duration,
            presentations: Vec::new(),
            observations: Vec::new(),
        });
    }
    pub fn take_calibration(&mut self) -> Vec<lucas_app::calibration::Observation> {
        self.collection
            .take()
            .map_or_else(Vec::new, |c| c.observations)
    }
    pub fn is_collecting(&self) -> bool {
        self.collection.is_some()
    }
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }
    pub fn start(
        &mut self,
        directory: &Path,
        header: Header,
        started: Instant,
    ) -> trace::Result<()> {
        std::fs::create_dir_all(directory)?;
        // create_new prevents overwriting a previous test even in the same ms.
        let path = directory.join(format!(
            "glance-{}-{}.jsonl",
            header.started_unix_ms,
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let duration = Duration::from_secs(header.script_seconds);
        let writer = TraceWriter::new(BufWriter::new(file), header)?;
        self.recording = Some(Recording {
            writer,
            started,
            duration,
            presentations: Vec::new(),
        });
        self.recording_path = Some(path);
        self.saved = None;
        self.error = None;
        Ok(())
    }
    /// Called when a new target is submitted to egui, not recomputed from the
    /// capture thread's wall clock. Late frames retain the old displayed label.
    pub fn present(&mut self, at: Instant, presentation: Presentation) {
        if let Some(collection) = &mut self.collection
            && collection
                .presentations
                .last()
                .is_none_or(|(_, p)| *p != presentation)
        {
            collection.presentations.push((at, presentation));
        }
        if let Some(recording) = &mut self.recording
            && recording
                .presentations
                .last()
                .is_none_or(|(_, p)| *p != presentation)
        {
            recording.presentations.push((at, presentation));
        }
    }
    pub fn finish(&mut self, reason: EndReason) {
        if let Some(recording) = self.recording.take() {
            match recording.writer.finish(reason) {
                Ok(file) => match file.get_ref().sync_all() {
                    Ok(()) => self.saved = self.recording_path.take(),
                    Err(e) => self.error = Some(format!("Trace flush failed: {e}")),
                },
                Err(e) => self.error = Some(format!("Trace save failed: {e}")),
            }
        }
    }
    fn record(&mut self, acquired: Instant, diagnostic: Diagnostic) -> trace::Result<()> {
        if let Some(c) = &mut self.collection
            && acquired
                .checked_duration_since(c.started)
                .is_some_and(|t| t < c.duration)
            && let Some((shown_at, presentation)) =
                c.presentations.iter().rev().find(|(at, _)| *at <= acquired)
            && presentation.phase == Phase::Hold
            && acquired.duration_since(*shown_at) >= Duration::from_millis(200)
            && let Some(target) = presentation.target
            && c.observations.len() < 3000
        {
            c.observations.push(lucas_app::calibration::observation(
                target,
                diagnostic.sample,
            ));
        }
        let Some(recording) = &mut self.recording else {
            return Ok(());
        };
        let Some(elapsed) = acquired.checked_duration_since(recording.started) else {
            return Ok(());
        };
        if elapsed >= recording.duration {
            return Ok(());
        }
        let Some((_, shown)) = recording
            .presentations
            .iter()
            .rev()
            .find(|(at, _)| *at <= acquired)
        else {
            return Ok(());
        };
        recording.writer.frame(FrameRecord {
            timestamp_ns: elapsed.as_nanos().try_into()?,
            source_timestamp_ns: diagnostic.sample.timestamp.as_nanos().try_into()?,
            features: diagnostic.sample.features.map(|f| f.as_array()),
            raw_features: diagnostic.raw,
            face_found: diagnostic.face_found,
            quality: diagnostic.sample.quality,
            cpu_ms: diagnostic.cpu_ms,
            phase: shown.phase,
            target: shown.target,
        })
    }
}

pub struct Worker {
    pub shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(index: u32, model_dir: PathBuf, context: eframe::egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (state, signal) = (shared.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            if let Err(error) = run(index, model_dir, &state, &signal, &context) {
                let mut state = state.lock().unwrap();
                state.finish(EndReason::TrackingError);
                state.error = Some(format!(
                    "Camera/tracking stopped: {error}. Close other camera apps, check lighting and reconnect."
                ));
            }
            state.lock().unwrap().finish(EndReason::Cancelled);
            context.request_repaint();
        });
        Self {
            shared,
            stop,
            handle: Some(handle),
        }
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
        // Release the device before setup hands ownership to calibration/runtime.
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Default)]
struct Raw {
    found: bool,
    features: Option<[f32; 10]>,
    luminance: f32,
}
struct ObservedLandmarker<L> {
    inner: L,
    raw: Arc<Mutex<Raw>>,
}
impl<L: Landmarker> Landmarker for ObservedLandmarker<L> {
    fn locate(&mut self, frame: &Frame) -> lucas_gaze::Result<Option<FaceLandmarks>> {
        let face = self.inner.locate(frame)?;
        let mut count = 0;
        let sum: f32 = frame
            .rgb()
            .chunks_exact(3)
            .step_by(64)
            .map(|p| {
                count += 1;
                (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
                    / 255.0
            })
            .sum();
        *self.raw.lock().unwrap() = Raw {
            found: face.is_some(),
            luminance: if count == 0 { 0.0 } else { sum / count as f32 },
            features: face
                .as_ref()
                .and_then(features::extract)
                .map(|(f, _)| f.as_array()),
        };
        Ok(face)
    }
    fn reset(&mut self) {
        self.inner.reset();
        *self.raw.lock().unwrap() = Raw::default();
    }
}
fn run(
    index: u32,
    models: PathBuf,
    state: &Mutex<Shared>,
    stop: &AtomicBool,
    context: &eframe::egui::Context,
) -> trace::Result<()> {
    let camera = NokhwaCamera::open_c270(index)?;
    // Camera acquisition and displayed targets share Instant's monotonic clock.
    let camera_origin = camera.timestamp_origin();
    let raw = Arc::new(Mutex::new(Raw::default()));
    let landmarker = ObservedLandmarker {
        inner: MediaPipeLandmarker::new(OrtBackend::new(VerifiedModels::load(&models)?)?),
        raw: raw.clone(),
    };
    let mut source = WebcamGazeSource::new(camera, landmarker)?;
    let mut recent = std::collections::VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        let cpu = cpu_time::ThreadTime::now();
        let Some(sample) = source.next_sample()? else {
            return Err("camera stream ended".into());
        };
        let cpu_ms = cpu.elapsed().as_secs_f64() * 1000.0;
        let acquired = camera_origin
            .checked_add(sample.timestamp)
            .ok_or("camera timestamp overflow; reconnect the webcam or update its driver")?;
        let received = Instant::now();
        if acquired > received + Duration::from_millis(100) {
            return Err(
                "camera timestamp is in the future; reconnect the webcam or update its driver"
                    .into(),
            );
        }
        recent.push_back(sample.timestamp);
        while recent.len() > 60 {
            recent.pop_front();
        }
        let span = sample.timestamp - recent[0];
        let fps = if span.is_zero() {
            0.0
        } else {
            (recent.len() - 1) as f64 / span.as_secs_f64()
        };
        let raw = raw.lock().unwrap();
        let diagnostic = Diagnostic {
            sample,
            raw: raw.features,
            face_found: raw.found,
            cpu_ms,
            fps,
            received,
            luminance: raw.luminance,
        };
        let mut state = state.lock().unwrap();
        state.latest = Some(diagnostic);
        if let Err(e) = state.record(acquired, diagnostic) {
            // An I/O failure can leave half a JSON line. Do not append an end
            // record or report that partial file as a successfully saved trace.
            state.recording = None;
            state.recording_path = None;
            state.error = Some(format!(
                "Recording stopped: {e}. The file may be incomplete. Check free disk space; start a new test."
            ));
        }
        drop(state);
        context.request_repaint();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucas_app::{settings::MinimapSelection, trace::CameraInfo};

    #[test]
    fn calibration_uses_acquisition_time_settling_and_focus_without_writing() {
        let started = Instant::now();
        let mut shared = Shared::default();
        shared.begin_calibration(started, Duration::from_secs(10));
        let target = |id| Target {
            id,
            position: [0.5, 0.5],
            kind: lucas_app::script::TargetKind::Elsewhere,
        };
        let hold = |id| Presentation {
            phase: Phase::Hold,
            target: Some(target(id)),
        };
        let diagnostic = |ms| Diagnostic {
            sample: GazeSample {
                timestamp: Duration::from_millis(ms),
                features: None,
                quality: 0.0,
            },
            raw: None,
            face_found: false,
            cpu_ms: 1.0,
            fps: 30.0,
            received: started,
            luminance: 0.1,
        };
        shared.present(started + Duration::from_secs(1), hold(0));
        for ms in [900, 1100, 1300] {
            shared
                .record(started + Duration::from_millis(ms), diagnostic(ms))
                .unwrap();
        }
        shared.present(started + Duration::from_secs(2), hold(1));
        // A frame delivered late still belongs to the previously displayed dot.
        shared
            .record(started + Duration::from_millis(1500), diagnostic(1500))
            .unwrap();
        shared
            .record(started + Duration::from_millis(2300), diagnostic(2300))
            .unwrap();
        shared.present(
            started + Duration::from_secs(3),
            Presentation {
                phase: Phase::Unfocused,
                target: None,
            },
        );
        shared
            .record(started + Duration::from_millis(3300), diagnostic(3300))
            .unwrap();
        let observations = shared.take_calibration();
        assert_eq!(
            observations.iter().map(|o| o.target_id).collect::<Vec<_>>(),
            [0, 0, 1]
        );
        assert!(
            observations
                .iter()
                .all(|o| o.labelled.sample.features.is_none())
        );
        assert!(!shared.is_recording());
        assert!(shared.saved.is_none());
        assert!(!shared.is_collecting());
    }
    #[test]
    fn successful_restart_clears_previous_recording_error() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = Shared {
            error: Some("Recording stopped: disk full".into()),
            ..Default::default()
        };
        let mut header = Header {
            schema_version: 1,
            app_version: "test".into(),
            started_unix_ms: 0,
            camera: CameraInfo {
                name: "synthetic".into(),
                resolution: [1280, 720],
                fps: 30,
            },
            monitor_resolution: [1920, 1080],
            dpi_scale: 1.0,
            lighting_note: String::new(),
            script_seconds: 61,
            minimap: MinimapSelection::resolve(&lucas_game::SettingsLocator::default()).unwrap(),
        };
        assert!(
            state
                .start(directory.path(), header.clone(), Instant::now())
                .is_err()
        );
        assert!(state.error.is_some());
        assert!(!state.is_recording());
        header.started_unix_ms = 1;
        header.script_seconds = 60;
        state
            .start(directory.path(), header, Instant::now())
            .unwrap();
        assert!(state.error.is_none());
        assert!(state.is_recording());
        state.finish(EndReason::Completed);
        assert!(state.saved.as_ref().unwrap().is_file());
        assert!(state.error.is_none());
    }
}

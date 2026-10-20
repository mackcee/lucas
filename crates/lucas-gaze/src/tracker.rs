//! Shared selectable gaze source used by setup, calibration, traces and coaching.
use crate::{
    Camera, Error, Frame, GazeFeatures, GazeSample, GazeSource, Landmarker, MediaPipeLandmarker,
    Result,
    features::{self, FeatureKind},
    filter::OneEuro,
    models::VerifiedModels,
    runtime::OrtBackend,
    unigaze::UniGaze,
};
use std::{path::Path, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    Auto,
    UniGaze,
    MediaPipe,
}
impl Selection {
    pub fn from_env() -> Result<Self> {
        match std::env::var("LUCAS_GAZE_SOURCE").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("auto") => Ok(Self::Auto),
            Ok("unigaze") => Ok(Self::UniGaze),
            Ok("mediapipe") => Ok(Self::MediaPipe),
            _ => Err(Error::Source(
                "LUCAS_GAZE_SOURCE must be auto, unigaze or mediapipe".into(),
            )),
        }
    }
}
pub enum Tracker {
    MediaPipe(Box<MediaPipeLandmarker<OrtBackend>>, String),
    UniGaze(UniGaze),
}
impl Tracker {
    pub fn load(models: &Path, selection: Selection) -> Result<Self> {
        Self::load_cancellable(models, selection, None)
    }
    pub fn load_cancellable(
        models: &Path,
        selection: Selection,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Self> {
        if selection == Selection::UniGaze
            || (selection == Selection::Auto && UniGaze::installed(models))
        {
            return UniGaze::start_cancellable(models, cancel).map(Self::UniGaze);
        }
        let description = if selection == Selection::Auto {
            "UniGaze not installed — using MediaPipe fallback. Run models/unigaze/setup.ps1 for NVIDIA CUDA tracking."
        } else {
            "MediaPipe fallback selected (CPU landmark inference)"
        };
        Ok(Self::MediaPipe(
            Box::new(MediaPipeLandmarker::new(OrtBackend::new(
                VerifiedModels::load(models)?,
            )?)),
            description.into(),
        ))
    }
    pub fn description(&self) -> &str {
        match self {
            Self::MediaPipe(_, message) => message,
            Self::UniGaze(source) => &source.description,
        }
    }
    pub fn kind(&self) -> FeatureKind {
        match self {
            Self::MediaPipe(..) => FeatureKind::MediaPipe,
            Self::UniGaze(_) => FeatureKind::UniGaze,
        }
    }
}
#[derive(Clone, Copy, Default)]
pub struct TrackingDiagnostic {
    pub raw: Option<[f32; 10]>,
    pub face_found: bool,
    pub luminance: f32,
}
pub struct ModelGazeSource<C> {
    camera: C,
    tracker: Tracker,
    frame: Option<Frame>,
    filters: [OneEuro; 10],
    previous: Option<Duration>,
    diagnostic: TrackingDiagnostic,
}
impl<C: Camera> ModelGazeSource<C> {
    pub fn new(camera: C, tracker: Tracker) -> Result<Self> {
        let (w, h) = camera.dimensions();
        Ok(Self {
            camera,
            tracker,
            frame: Some(Frame::new(w, h)?),
            filters: [OneEuro::new(1.0, 0.1, 1.0)?; 10],
            previous: None,
            diagnostic: Default::default(),
        })
    }
    pub fn diagnostic(&self) -> TrackingDiagnostic {
        self.diagnostic
    }
    pub fn description(&self) -> &str {
        self.tracker.description()
    }
    pub fn kind(&self) -> FeatureKind {
        self.tracker.kind()
    }
}
impl<C: Camera> GazeSource for ModelGazeSource<C> {
    fn next_sample(&mut self) -> Result<Option<GazeSample>> {
        let mut frame = self.frame.take().ok_or(Error::CaptureFailed)?;
        self.diagnostic = Default::default();
        if !self.camera.capture(&mut frame)? {
            return Ok(None);
        }
        let timestamp = frame.timestamp();
        if self.previous.is_some_and(|t| timestamp <= t) {
            return Err(Error::InvalidTimestamp);
        }
        self.previous = Some(timestamp);
        let kind = self.kind();
        let (features, quality) = match &mut self.tracker {
            Tracker::MediaPipe(landmarker, _) => {
                let face = landmarker.locate(&frame)?;
                let mut count = 0;
                let sum: f32 = frame
                    .rgb()
                    .chunks_exact(3)
                    .step_by(64)
                    .map(|p| {
                        count += 1;
                        (0.2126 * f32::from(p[0])
                            + 0.7152 * f32::from(p[1])
                            + 0.0722 * f32::from(p[2]))
                            / 255.0
                    })
                    .sum();
                self.diagnostic.face_found = face.is_some();
                self.diagnostic.luminance = if count == 0 { 0.0 } else { sum / count as f32 };
                let value = face.as_ref().and_then(features::extract);
                (value.map(|(f, _)| f), value.map_or(0.0, |(_, q)| q))
            }
            Tracker::UniGaze(source) => {
                let (returned, observed) = source.infer(frame)?;
                frame = returned;
                self.diagnostic.face_found = observed.face_found;
                self.diagnostic.luminance = observed.luminance;
                (
                    observed.features.map(GazeFeatures::UniGaze),
                    observed.quality,
                )
            }
        };
        frame.clear();
        self.frame = Some(frame);
        self.diagnostic.raw = features.map(GazeFeatures::as_array);
        let features = if let Some(features) = features {
            let mut values = features.as_array();
            let count = if kind == FeatureKind::UniGaze { 2 } else { 10 };
            for (value, filter) in values[..count].iter_mut().zip(&mut self.filters) {
                *value = filter.filter(timestamp, *value)?;
            }
            Some(GazeFeatures::from_array_for(kind, values))
        } else {
            self.filters.iter_mut().for_each(OneEuro::reset);
            None
        };
        Ok(Some(GazeSample {
            timestamp,
            features,
            quality,
        }))
    }
}

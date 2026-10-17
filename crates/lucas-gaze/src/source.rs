//! Gaze-source boundary for later hardware backends and calibration.
use crate::{
    Camera, Error, FrameRing, GazeFeatures, Landmarker, Result, features, filter::OneEuro,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GazeSample {
    pub timestamp: Duration,
    /// None means unknown/no usable face, NEVER a minimap check.
    pub features: Option<GazeFeatures>,
    /// [0,1] heuristic tracking quality; zero whenever features are absent.
    pub quality: f32,
}
pub trait GazeSource {
    /// None is end-of-source; a sample with absent features is tracking loss.
    fn next_sample(&mut self) -> Result<Option<GazeSample>>;
}

pub struct WebcamGazeSource<C, L> {
    camera: C,
    landmarker: L,
    ring: FrameRing,
    filters: [OneEuro; GazeFeatures::COUNT],
    previous: Option<Duration>,
}
impl<C: Camera, L: Landmarker> WebcamGazeSource<C, L> {
    pub fn new(camera: C, mut landmarker: L) -> Result<Self> {
        let (w, h) = camera.dimensions();
        landmarker.reset();
        Ok(Self {
            camera,
            landmarker,
            ring: FrameRing::new(w, h)?,
            filters: [OneEuro::new(1.0, 0.1, 1.0)?; GazeFeatures::COUNT],
            previous: None,
        })
    }
    fn reset_tracking(&mut self) {
        self.landmarker.reset();
        self.filters.iter_mut().for_each(OneEuro::reset);
        self.previous = None;
    }
}
impl<C: Camera, L: Landmarker> GazeSource for WebcamGazeSource<C, L> {
    fn next_sample(&mut self) -> Result<Option<GazeSample>> {
        let frame = self.ring.next_frame();
        let captured = self.camera.capture(frame);
        if !matches!(captured, Ok(true)) {
            frame.clear();
            self.reset_tracking();
            return captured.map(|_| None);
        }
        let timestamp = frame.timestamp();
        if self.previous.is_some_and(|t| timestamp <= t) {
            frame.clear();
            self.reset_tracking();
            return Err(Error::InvalidTimestamp);
        }
        self.previous = Some(timestamp);
        let face = self.landmarker.locate(frame);
        // Pixels have no consumer after inference, including on the error path.
        frame.clear();
        let face = match face {
            Ok(face) => face,
            Err(error) => {
                self.reset_tracking();
                return Err(error);
            }
        };
        let Some((features, quality)) = face.as_ref().and_then(features::extract) else {
            self.filters.iter_mut().for_each(OneEuro::reset);
            return Ok(Some(GazeSample {
                timestamp,
                features: None,
                quality: 0.0,
            }));
        };
        let mut values = features.as_array();
        for (value, filter) in values.iter_mut().zip(&mut self.filters) {
            *value = filter.filter(timestamp, *value)?;
        }
        Ok(Some(GazeSample {
            timestamp,
            features: Some(GazeFeatures::from_array(values)),
            quality,
        }))
    }
}

/// Numeric, read-only replay seam. Parsing/storage of opt-in debug traces is M4.2.
/// Rejects malformed samples rather than turning them into apparent gaze checks.
pub struct TraceGazeSource<I> {
    samples: I,
    previous: Option<Duration>,
}
impl<I: Iterator<Item = GazeSample>> TraceGazeSource<I> {
    pub fn new(samples: I) -> Self {
        Self {
            samples,
            previous: None,
        }
    }
}
impl<I: Iterator<Item = GazeSample>> GazeSource for TraceGazeSource<I> {
    fn next_sample(&mut self) -> Result<Option<GazeSample>> {
        let Some(sample) = self.samples.next() else {
            return Ok(None);
        };
        if self.previous.is_some_and(|t| sample.timestamp <= t) {
            return Err(Error::InvalidTimestamp);
        }
        if !sample.quality.is_finite()
            || !(0.0..=1.0).contains(&sample.quality)
            || (sample.features.is_none() && sample.quality != 0.0)
            || sample
                .features
                .is_some_and(|f| f.as_array().iter().any(|v| !v.is_finite()))
        {
            return Err(Error::InvalidOutput);
        }
        self.previous = Some(sample.timestamp);
        Ok(Some(sample))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FailingCamera;
    impl Camera for FailingCamera {
        fn dimensions(&self) -> (u32, u32) {
            (2, 2)
        }
        fn capture(&mut self, frame: &mut crate::Frame) -> Result<bool> {
            frame.rgb_mut().fill(77);
            Err(Error::CaptureFailed)
        }
    }
    struct MissingFace;
    impl Landmarker for MissingFace {
        fn locate(&mut self, _: &crate::Frame) -> Result<Option<crate::FaceLandmarks>> {
            Ok(None)
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn capture_errors_leave_all_slots_scrubbed() {
        let mut source = WebcamGazeSource::new(FailingCamera, MissingFace).unwrap();
        assert_eq!(source.next_sample(), Err(Error::CaptureFailed));
        for _ in 0..crate::camera::RING_CAPACITY {
            assert!(source.ring.next_frame().rgb().iter().all(|p| *p == 0));
        }
    }
    #[test]
    fn trace_preserves_unknown_and_end_distinction() {
        let unknown = GazeSample {
            timestamp: Duration::ZERO,
            features: None,
            quality: 0.0,
        };
        let mut source = TraceGazeSource::new([unknown, unknown].into_iter());
        assert_eq!(source.next_sample().unwrap(), Some(unknown));
        assert_eq!(source.next_sample(), Err(Error::InvalidTimestamp));
        assert_eq!(source.next_sample().unwrap(), None);
        let mut source = TraceGazeSource::new(
            [GazeSample {
                quality: 0.2,
                ..unknown
            }]
            .into_iter(),
        );
        assert_eq!(source.next_sample(), Err(Error::InvalidOutput));
    }
}

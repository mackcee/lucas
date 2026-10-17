//! Calibration-to-core seam. Unknown observations always interrupt dwell.
use crate::{GazeSample, calibration::CalibrationModel};
use lucas_core::{CoreError, DetectorConfig, GlanceDetector, GlanceUpdate, Timestamp};

pub struct CalibratedDetector {
    model: CalibrationModel,
    detector: GlanceDetector,
}
impl CalibratedDetector {
    pub fn new(model: CalibrationModel) -> Result<Self, CoreError> {
        let detector = GlanceDetector::new(DetectorConfig {
            enter_probability: model.threshold(),
            exit_probability: model.threshold() * 0.75,
            minimum_samples: 3,
            ..DetectorConfig::default()
        })?;
        Ok(Self { model, detector })
    }
    pub fn model(&self) -> &CalibrationModel {
        &self.model
    }
    pub fn observe(&mut self, sample: GazeSample) -> Result<GlanceUpdate, CoreError> {
        let prediction = self.model.predict(sample);
        let at = u64::try_from(sample.timestamp.as_millis())
            .map_err(|_| CoreError::InvalidConfig("gaze timestamp overflow"))?;
        self.detector.observe(lucas_core::GazeSample {
            at: Timestamp(at),
            p_minimap: prediction.map_or(0.0, |p| p.probability),
            quality: prediction.map_or(0.0, |_| f64::from(sample.quality)),
        })
    }
}

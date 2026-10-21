//! Explain calibration rejection without changing its acceptance criteria.
use lucas_gaze::calibration::{CalibrationModel, Validation};
use lucas_store::{Calibration, ScreenGeometry};

#[derive(Clone, Debug, PartialEq)]
pub enum CalibrationIssue {
    MissingProfile,
    ReadError(String),
    ModelError(String),
    GeometryChanged(Vec<String>),
    ValidationFailed(Validation),
}
impl CalibrationIssue {
    pub fn summary(&self) -> &'static str {
        match self {
            Self::MissingProfile => "no saved calibration",
            Self::ReadError(_) => "saved calibration could not be read",
            Self::ModelError(_) => "saved model is unsupported or invalid",
            Self::GeometryChanged(_) => "screen or minimap changed",
            Self::ValidationFailed(_) => "saved calibration did not pass",
        }
    }
    /// Separate native menu rows keep numeric differences readable, rather than
    /// truncating a single long tooltip. f64 Display preserves tiny differences.
    pub fn details(&self) -> Vec<String> {
        let mut rows = vec![self.summary().to_owned()];
        match self {
            Self::ReadError(error) | Self::ModelError(error) => {
                rows.push(error.clone());
            }
            Self::GeometryChanged(differences) => rows.extend(differences.iter().cloned()),
            Self::ValidationFailed(v) => rows.push(format!(
                "Saved validation: error {}°, sensitivity {}%, false positives {}%, usable {}%",
                v.mean_error_degrees,
                v.true_positive_rate * 100.0,
                v.false_positive_rate * 100.0,
                v.usable_fraction * 100.0,
            )),
            _ => {}
        }
        rows.push("Choose Calibrate gaze for this screen/minimap; coaching stays paused.".into());
        rows
    }
}
impl std::fmt::Display for CalibrationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.details().join("; "))
    }
}
impl std::error::Error for CalibrationIssue {}

/// Used for both the provisional startup load and the authoritative geometry
/// check. A passing, matching saved fit can be used without a per-game test;
/// foreground, tracking and all other coaching gates still apply.
pub fn assess(
    saved: Option<Calibration>,
    geometry: Option<(&ScreenGeometry, [f64; 4])>,
) -> Result<CalibrationModel, CalibrationIssue> {
    let saved = saved.ok_or(CalibrationIssue::MissingProfile)?;
    saved
        .validate()
        .map_err(|e| CalibrationIssue::ReadError(e.to_string()))?;
    let model = CalibrationModel::from_bytes(&saved.model)
        .map_err(|e| CalibrationIssue::ModelError(e.to_string()))?;
    if let Some((screen, minimap)) = geometry {
        let mut differences = Vec::new();
        if saved.screen.left != screen.left || saved.screen.top != screen.top {
            differences.push(format!(
                "Screen origin: saved ({}, {}), current ({}, {})",
                saved.screen.left, saved.screen.top, screen.left, screen.top
            ));
        }
        if saved.screen.width != screen.width || saved.screen.height != screen.height {
            differences.push(format!(
                "Screen size: saved {}×{}, current {}×{} pixels",
                saved.screen.width, saved.screen.height, screen.width, screen.height
            ));
        }
        if saved.screen.scale_factor != screen.scale_factor {
            differences.push(format!(
                "Display scale: saved {}, current {}",
                saved.screen.scale_factor, screen.scale_factor
            ));
        }
        if model.screen().resolution != [screen.width, screen.height] {
            differences.push(format!(
                "Model resolution: saved {:?}, current {}×{}",
                model.screen().resolution,
                screen.width,
                screen.height
            ));
        }
        for ((edge, saved), current) in ["left", "top", "right", "bottom"]
            .into_iter()
            .zip(model.screen().minimap)
            .zip(minimap)
        {
            if !current.is_finite() || (saved - current).abs() > 1e-9 {
                differences.push(format!(
                    "Minimap {edge} (normalized): saved {saved}, current {current}"
                ));
            }
        }
        if !differences.is_empty() {
            return Err(CalibrationIssue::GeometryChanged(differences));
        }
    }
    if !model
        .validation()
        .passes(model.target_false_positive_rate())
    {
        return Err(CalibrationIssue::ValidationFailed(model.validation()));
    }
    Ok(model)
}

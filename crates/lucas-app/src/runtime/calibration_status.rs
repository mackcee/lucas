//! Calibration availability follows the saved fit and actual geometry, not a
//! per-game approval latch. Optional test results are hints, never invalidations.
use super::{CalibrationState, Runtime};
use crate::calibration::{self, CalibrationIssue};
use lucas_store::{ScreenGeometry, Store};

#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub screen: ScreenGeometry,
    pub minimap: [f64; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometrySource {
    Provisional,
    LiveGame,
}
/// Private subprocess exit protocol; never used as a coaching permission gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CalibrationOutcome {
    Passed = 0,
    Cancelled = 3,
    Failed = 4,
    Changed = 5,
}
impl CalibrationOutcome {
    pub fn from_code(code: Option<i32>) -> Option<Self> {
        match code {
            Some(0) => Some(Self::Passed),
            Some(3) => Some(Self::Cancelled),
            Some(4) => Some(Self::Failed),
            Some(5) => Some(Self::Changed),
            _ => None,
        }
    }
}
#[derive(Default)]
pub struct CalibrationStatus {
    geometry: Option<Geometry>,
    source: Option<GeometrySource>,
    issue: Option<CalibrationIssue>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryUpdate {
    pub changed: bool,
    pub reload: bool,
}
impl CalibrationStatus {
    pub fn geometry(&self) -> Option<&Geometry> {
        self.geometry.as_ref()
    }
    pub fn issue(&self) -> Option<&CalibrationIssue> {
        self.issue.as_ref()
    }
    pub fn needed_message(&self) -> String {
        match &self.issue {
            Some(issue) => format!("Calibration needed — {}", issue.summary()),
            None => "Calibration needed — no usable saved calibration".into(),
        }
    }
    /// Unchanged inputs cannot repeatedly reload or toggle calibration state.
    /// A real geometry change, including returning to a saved geometry, is
    /// assessed with exactly the same loader and acceptance rules as startup.
    pub fn update_geometry(
        &mut self,
        geometry: Geometry,
        source: GeometrySource,
    ) -> GeometryUpdate {
        let changed = self.geometry.as_ref() != Some(&geometry);
        let reload = changed || self.source != Some(source);
        self.geometry = Some(geometry);
        self.source = Some(source);
        GeometryUpdate { changed, reload }
    }
    pub fn reload(
        &mut self,
        store: &Store,
        runtime: &mut Runtime,
    ) -> Result<(), lucas_core::CoreError> {
        let geometry = self.geometry.as_ref().map(|g| (&g.screen, g.minimap));
        let model = match calibration::load_explained(store, geometry) {
            Ok(model) => {
                self.issue = None;
                Some(model)
            }
            Err(issue) => {
                self.issue = Some(issue);
                None
            }
        };
        runtime.install_calibration(model)
    }
    /// The desktop and portable regression tests use this same completion path.
    /// Even a successful test cannot restore a genuinely invalidated model.
    pub fn finish_test(
        &self,
        runtime: &mut Runtime,
        outcome: Option<CalibrationOutcome>,
        same_geometry: bool,
    ) -> Result<&'static str, lucas_core::CoreError> {
        runtime.reset_gaze()?;
        Ok(if runtime.calibration() == CalibrationState::Needed {
            "Test finished — calibration is still needed for this screen/minimap"
        } else if !same_geometry || outcome == Some(CalibrationOutcome::Changed) {
            "Test interrupted by changed inputs — saved calibration kept"
        } else {
            match outcome {
                Some(CalibrationOutcome::Passed) => "Test passed — saved calibration unchanged",
                Some(CalibrationOutcome::Cancelled) => "Test cancelled — saved calibration kept",
                Some(CalibrationOutcome::Failed) => {
                    "Test did not pass — saved calibration kept; recalibrate if tracking seems wrong"
                }
                _ => "Test unavailable — saved calibration kept",
            }
        })
    }
}

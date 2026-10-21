//! Portable lifecycle of calibration rejection reasons and provisional geometry.
//! Explicit cancellation/failure always wins over automatic geometry recovery.
use super::CalibrationState;
use crate::calibration::CalibrationIssue;
use lucas_gaze::calibration::CalibrationModel;
use lucas_store::ScreenGeometry;

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
/// Private subprocess exit protocol. Distinguishes closing a window from a
/// completed validation failure; unexpected exit codes remain window errors.
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
    pub fn issue(self, quick: bool) -> Option<CalibrationIssue> {
        match (self, quick) {
            (Self::Passed, _) => None,
            (Self::Cancelled, true) => Some(CalibrationIssue::QuickCheckCancelled),
            (Self::Failed, true) => Some(CalibrationIssue::QuickCheckFailed),
            (Self::Cancelled, false) => Some(CalibrationIssue::FullCalibrationCancelled),
            (Self::Failed, false) => Some(CalibrationIssue::FullCalibrationFailed),
            (Self::Changed, true) => Some(CalibrationIssue::QuickCheckChanged),
            (Self::Changed, false) => Some(CalibrationIssue::FullCalibrationChanged),
        }
    }
}
#[derive(Default)]
pub struct CalibrationStatus {
    geometry: Option<Geometry>,
    live_seen: bool,
    retry_provisional: bool,
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
    pub fn tray_message(&self, active: bool) -> String {
        if active {
            "Calibration / quick check running — coaching paused".into()
        } else if let Some(issue) = &self.issue {
            format!(
                "Calibration needed — {} (see Calibration details)",
                issue.summary()
            )
        } else {
            "Calibration needed — choose Calibrate gaze".into()
        }
    }
    pub fn update_geometry(
        &mut self,
        geometry: Geometry,
        source: GeometrySource,
        state: CalibrationState,
        busy: bool,
    ) -> GeometryUpdate {
        let first_live = source == GeometrySource::LiveGame && !self.live_seen;
        let retry = first_live && self.retry_provisional;
        if source == GeometrySource::LiveGame {
            self.live_seen = true;
            self.retry_provisional = false; // one authoritative retry, not a loop
        }
        let changed = self.geometry.as_ref() != Some(&geometry);
        self.geometry = Some(geometry);
        GeometryUpdate {
            changed,
            reload: !busy && ((changed && state != CalibrationState::Needed) || retry),
        }
    }
    pub fn loaded(
        &mut self,
        result: Result<CalibrationModel, CalibrationIssue>,
    ) -> Option<CalibrationModel> {
        match result {
            Ok(model) => {
                self.issue = None;
                self.retry_provisional = false;
                Some(model)
            }
            Err(issue) => {
                self.retry_provisional =
                    !self.live_seen && matches!(issue, CalibrationIssue::GeometryChanged(_));
                self.issue = Some(issue);
                None
            }
        }
    }
    pub fn block(&mut self, issue: CalibrationIssue) {
        self.retry_provisional = false;
        self.issue = Some(issue);
    }
    /// Called after the runtime accepts a quick-check result.
    pub fn quick_finished(&mut self, outcome: Option<CalibrationOutcome>, same_generation: bool) {
        self.retry_provisional = false;
        self.issue = if !same_generation {
            Some(CalibrationIssue::QuickCheckChanged)
        } else {
            match outcome {
                Some(outcome) => outcome.issue(true),
                None => Some(CalibrationIssue::WindowError(
                    "Quick-check window exited unexpectedly; run full calibration.".into(),
                )),
            }
        };
    }
}

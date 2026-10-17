//! Calibration schedule, in-memory observations and the M5.2 revalidation seam.
//! No camera frames or calibration observations are persisted by this module.
use crate::script::{Phase, Step, Target, TargetKind};
use lucas_gaze::{
    GazeSample,
    calibration::{self, CalibrationModel, FitOptions, LabelledSample, Screen, Validation},
};
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};
use lucas_store::{Calibration, ScreenGeometry, Store};
use std::time::Duration;

pub const SETTLE: Duration = Duration::from_millis(500);
pub const TARGET_TIME: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Fit,
    Validate,
    Quick,
}
#[derive(Clone, Copy, Debug)]
pub struct CalibrationTarget {
    pub target: Target,
    pub stage: Stage,
    pub turned: bool,
}
pub struct CalibrationScript {
    targets: Vec<CalibrationTarget>,
    screen: Screen,
}
impl CalibrationScript {
    pub fn new(
        resolution: [u32; 2],
        settings: MinimapSettings,
        size_mm: [f64; 2],
        distance_mm: f64,
    ) -> calibration::Result<Self> {
        let bounds = Rect::new(0.0, 0.0, resolution[0] as f64, resolution[1] as f64)
            .map_err(|_| calibration::CalibrationError("invalid monitor"))?;
        let map = minimap_rect(bounds, settings, Alignment::default())
            .map_err(|_| calibration::CalibrationError("cannot locate minimap"))?;
        let screen = Screen {
            resolution,
            size_mm,
            distance_mm,
            distance_assumed: true,
            minimap: [
                map.left() / bounds.width(),
                map.top() / bounds.height(),
                map.right() / bounds.width(),
                map.bottom() / bounds.height(),
            ],
        };
        screen.validate()?;
        let mut s = Self {
            targets: Vec::new(),
            screen,
        };
        for y in [0.08, 0.5, 0.92] {
            for x in [0.08, 0.5, 0.92] {
                s.push([x, y], Stage::Fit, false);
            }
        }
        let [l, t, r, b] = screen.minimap;
        let centre = [(l + r) / 2.0, (t + b) / 2.0];
        let beside = if settings.flipped { r + 0.04 } else { l - 0.04 };
        for p in [
            [0.5, 0.5],
            [0.5, 0.92],
            [0.92, 0.08],
            [centre[0], t - 0.04],
            [beside, centre[1]],
        ] {
            s.push(p, Stage::Fit, false);
        }
        // Five dense map positions, with the last two at a second head pose.
        for (i, [x, y]) in [[0.3, 0.3], [0.7, 0.3], [0.5, 0.5], [0.3, 0.7], [0.7, 0.7]]
            .into_iter()
            .enumerate()
        {
            s.push([l + (r - l) * x, t + (b - t) * y], Stage::Fit, i >= 3);
        }
        // Four held-out target occurrences; never enter the fit matrix.
        for (p, turned) in [
            (centre, true),
            ([0.5, 0.5], false),
            ([l + (r - l) * 0.6, t + (b - t) * 0.6], false),
            ([beside, centre[1]], false),
        ] {
            s.push(p, Stage::Validate, turned);
        }
        if s.targets
            .iter()
            .any(|t| t.target.position.iter().any(|v| !(0.0..=1.0).contains(v)))
        {
            return Err(calibration::CalibrationError(
                "monitor cannot accommodate calibration targets",
            ));
        }
        Ok(s)
    }
    pub fn quick(model: &CalibrationModel) -> Self {
        let screen = model.screen();
        let [l, t, r, b] = screen.minimap;
        let mut s = Self {
            targets: Vec::new(),
            screen,
        };
        s.push([(l + r) / 2.0, (t + b) / 2.0], Stage::Quick, false);
        s.push([0.5, 0.5], Stage::Quick, false);
        s
    }
    fn push(&mut self, position: [f64; 2], stage: Stage, turned: bool) {
        let [l, t, r, b] = self.screen.minimap;
        let kind = if position[0] >= l && position[0] <= r && position[1] >= t && position[1] <= b {
            TargetKind::Minimap
        } else {
            TargetKind::Elsewhere
        };
        self.targets.push(CalibrationTarget {
            target: Target {
                id: self.targets.len() as u32,
                position,
                kind,
            },
            stage,
            turned,
        });
    }
    pub fn screen(&self) -> Screen {
        self.screen
    }
    pub fn targets(&self) -> &[CalibrationTarget] {
        &self.targets
    }
    pub fn duration(&self) -> Duration {
        TARGET_TIME * self.targets.len() as u32
    }
    pub fn at(&self, elapsed: Duration) -> Option<(Step, CalibrationTarget)> {
        if elapsed >= self.duration() {
            return None;
        }
        let id = (elapsed.as_nanos() / TARGET_TIME.as_nanos()) as usize;
        let target = *self.targets.get(id)?;
        let within = elapsed - TARGET_TIME * id as u32;
        Some((
            Step {
                target: target.target,
                phase: if within < SETTLE {
                    Phase::Countdown
                } else {
                    Phase::Hold
                },
                remaining: if within < SETTLE {
                    SETTLE - within
                } else {
                    TARGET_TIME - within
                },
            },
            target,
        ))
    }
}
#[derive(Clone, Copy)]
pub struct Observation {
    pub target_id: u32,
    pub labelled: LabelledSample,
}
/// Called by the capture thread only after matching acquisition time to the
/// displayed target, not a UI poll of its latest frame. Unknowns are retained.
pub fn observation(target: Target, sample: GazeSample) -> Observation {
    Observation {
        target_id: target.id,
        labelled: LabelledSample {
            sample,
            target: target.position,
            minimap: target.kind == TargetKind::Minimap,
        },
    }
}
fn validate_observations(
    script: &CalibrationScript,
    observations: &[Observation],
) -> calibration::Result<()> {
    if observations.len() > 3000
        || observations.iter().any(|o| {
            script.targets().get(o.target_id as usize).is_none_or(|t| {
                o.labelled.target != t.target.position
                    || o.labelled.minimap != (t.target.kind == TargetKind::Minimap)
            })
        })
    {
        return Err(calibration::CalibrationError(
            "observations do not match calibration targets",
        ));
    }
    Ok(())
}
pub fn fit_session(
    script: &CalibrationScript,
    observations: &[Observation],
) -> calibration::Result<CalibrationModel> {
    validate_observations(script, observations)?;
    let mut train = Vec::new();
    let mut valid = Vec::new();
    for target in script.targets() {
        let rows: Vec<_> = observations
            .iter()
            .filter(|o| o.target_id == target.target.id)
            .map(|o| o.labelled)
            .collect();
        if rows.len() < 12
            || rows
                .iter()
                .filter(|s| calibration::usable(s.sample).is_some())
                .count()
                < 12
        {
            return Err(calibration::CalibrationError(
                "not enough tracked frames at every target; add front lighting and retry",
            ));
        }
        match target.stage {
            Stage::Fit => train.extend(rows),
            Stage::Validate => valid.extend(rows),
            Stage::Quick => {
                return Err(calibration::CalibrationError(
                    "quick validation cannot train a model",
                ));
            }
        }
    }
    calibration::fit(&train, &valid, script.screen(), FitOptions::default())
}
/// M5.2 calls this after presenting `CalibrationScript::quick`. This never
/// adjusts weights/thresholds or silently promotes failed tracking to a check.
pub fn quick_revalidate(
    model: &CalibrationModel,
    observations: &[Observation],
) -> calibration::Result<Validation> {
    let script = CalibrationScript::quick(model);
    validate_observations(&script, observations)?;
    for target in script.targets() {
        if observations
            .iter()
            .filter(|o| {
                o.target_id == target.target.id
                    && o.labelled.target == target.target.position
                    && o.labelled.minimap == (target.target.kind == TargetKind::Minimap)
            })
            .count()
            < 12
        {
            return Err(calibration::CalibrationError(
                "quick validation needs both minimap and centre targets",
            ));
        }
    }
    model.validate_samples(&observations.iter().map(|o| o.labelled).collect::<Vec<_>>())
}
pub fn save(
    store: &Store,
    model: &CalibrationModel,
    screen: ScreenGeometry,
    created_at_unix_ms: u64,
) -> crate::trace::Result<()> {
    if [screen.width, screen.height] != model.screen().resolution {
        return Err("calibration monitor changed".into());
    }
    if !model
        .validation()
        .passes(model.target_false_positive_rate())
    {
        return Err("calibration did not pass; previous weights retained".into());
    }
    store.save_calibration(&Calibration {
        schema_version: 1,
        model: model.to_bytes(),
        screen,
        created_at_unix_ms,
        validation_error_degrees: Some(model.validation().mean_error_degrees),
    })?;
    Ok(())
}
/// Reject changed resolution/DPI/origin/minimap rather than reuse stale geometry.
pub fn load(
    store: &Store,
    screen: &ScreenGeometry,
    minimap: [f64; 4],
) -> crate::trace::Result<Option<CalibrationModel>> {
    let Some(saved) = store.load_calibration()? else {
        return Ok(None);
    };
    let model = CalibrationModel::from_bytes(&saved.model)?;
    if &saved.screen != screen
        || model.screen().resolution != [screen.width, screen.height]
        || model
            .screen()
            .minimap
            .iter()
            .zip(minimap)
            .any(|(a, b)| !b.is_finite() || (a - b).abs() > 1e-9)
        || !model
            .validation()
            .passes(model.target_false_positive_rate())
    {
        return Ok(None);
    }
    Ok(Some(model))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_covers_grid_cluster_head_poses_and_four_validation_targets() {
        for flipped in [false, true] {
            for scale in [0.0, 1.0, 3.0] {
                let s = CalibrationScript::new(
                    [1920, 1080],
                    MinimapSettings {
                        minimap_scale: scale,
                        flipped,
                    },
                    [531.0, 299.0],
                    650.0,
                )
                .unwrap();
                assert_eq!(s.duration(), Duration::from_secs(46));
                assert_eq!(
                    s.targets
                        .iter()
                        .filter(|t| t.stage == Stage::Validate)
                        .count(),
                    4
                );
                assert!(
                    s.targets.iter().all(|t| t
                        .target
                        .position
                        .iter()
                        .all(|v| (0.0..=1.0).contains(v)))
                );
                assert!(
                    s.targets
                        .iter()
                        .any(|t| t.turned && t.target.kind == TargetKind::Minimap)
                );
                assert_eq!(s.at(Duration::ZERO).unwrap().0.phase, Phase::Countdown);
                assert_eq!(s.at(SETTLE).unwrap().0.phase, Phase::Hold);
                assert!(s.at(s.duration()).is_none());
                assert!(fit_session(&s, &[]).is_err());
            }
        }
    }
}

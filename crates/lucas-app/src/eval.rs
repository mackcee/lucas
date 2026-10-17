//! Offline G2 evaluation of the existing numeric glance-test format. Splits are
//! whole target occurrences, never adjacent frames from the same fixation.
use crate::{
    calibration::CalibrationScript,
    script::{Phase, Script, TargetKind},
    trace::{EndReason, FrameRecord, Trace},
};
use lucas_gaze::{
    calibration::{CalibrationModel, FitOptions, LabelledSample, Screen, fit},
    decision::CalibratedDetector,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
type Result<T> = crate::trace::Result<T>;
/// Human saccade / One-Euro settling after each displayed target or focus return.
const SETTLING_NS: u64 = 300_000_000;
fn settled_frames(frames: &[FrameRecord]) -> Vec<&FrameRecord> {
    let mut presentation = None;
    let mut settled = Vec::new();
    for frame in frames {
        let Some(target) = frame.target else {
            presentation = None;
            continue;
        };
        let (id, since) = *presentation.get_or_insert((target.id, frame.timestamp_ns));
        if id != target.id {
            presentation = Some((target.id, frame.timestamp_ns));
            continue;
        }
        if frame.timestamp_ns.saturating_sub(since) >= SETTLING_NS {
            settled.push(frame);
        }
    }
    settled
}
#[derive(Debug)]
pub struct Evaluation {
    pub minimap_trials: usize,
    pub detected_trials: usize,
    pub false_detections: usize,
    pub elsewhere_seconds: f64,
    pub usable_fraction: f64,
    pub coverage: f64,
    pub completed: bool,
}
impl Evaluation {
    pub fn detection_rate(&self) -> f64 {
        if self.minimap_trials == 0 {
            0.0
        } else {
            self.detected_trials as f64 / self.minimap_trials as f64
        }
    }
    pub fn false_per_minute(&self) -> f64 {
        if self.elsewhere_seconds <= 0.0 {
            f64::INFINITY
        } else {
            self.false_detections as f64 * 60.0 / self.elsewhere_seconds
        }
    }
    pub fn passes(&self) -> bool {
        self.completed
            && self.minimap_trials > 0
            && self.elsewhere_seconds > 0.0
            && self.usable_fraction >= 0.8
            && self.coverage >= 0.8
            && self.detection_rate() >= 0.9
            && self.false_per_minute() <= 1.0
    }
}
pub struct EvaluationRun {
    pub model: CalibrationModel,
    pub evaluation: Evaluation,
    pub training_trials: usize,
    pub validation_trials: usize,
}
fn trials(trace: &Trace) -> Result<BTreeMap<u32, TargetKind>> {
    let script = Script::new(
        trace.header.script_seconds,
        trace.header.monitor_resolution,
        trace.header.minimap.settings(),
    )?;
    let mut result = BTreeMap::new();
    // Absent trials remain misses / missing coverage, never silently disappear.
    for id in 0..script.trials() {
        let target = script
            .at(Duration::from_secs(u64::from(id) * 6 + 2))
            .unwrap()
            .target;
        result.insert(id, target.kind);
    }
    for f in &trace.frames {
        if let Some(t) = f.target {
            let expected = script
                .at(Duration::from_secs(u64::from(t.id) * 6 + 2))
                .ok_or("target outside script")?
                .target;
            if t != expected || f.phase != Phase::Hold {
                return Err("trace labels do not match the recorded script geometry".into());
            }
            result.insert(t.id, t.kind);
        }
    }
    Ok(result)
}
fn labelled(frames: &[FrameRecord], ids: &BTreeSet<u32>) -> Vec<LabelledSample> {
    settled_frames(frames)
        .into_iter()
        .filter_map(|f| {
            f.target
                .filter(|t| ids.contains(&t.id))
                .map(|t| LabelledSample {
                    sample: f.sample(),
                    target: t.position,
                    minimap: t.kind == TargetKind::Minimap,
                })
        })
        .collect()
}
/// With no separate calibration trace, 50%/25%/25% of each class's occurrences
/// (chronological, rounded down for fit/validation) go to fit/threshold/test.
/// Separate calibration: 75%/25% fit/threshold, and ALL evaluation trials tested.
pub fn evaluate(
    trace: &Trace,
    separate: Option<&Trace>,
    size_mm: [f64; 2],
    distance_mm: f64,
) -> Result<EvaluationRun> {
    let calibration = separate.unwrap_or(trace);
    if separate.is_some()
        && (calibration.header.monitor_resolution != trace.header.monitor_resolution
            || calibration.header.minimap.settings() != trace.header.minimap.settings())
    {
        return Err("calibration and evaluation monitor/minimap geometry differ".into());
    }
    let available = trials(calibration)?;
    let all_eval = trials(trace)?;
    let (mut training, mut validation, mut testing) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for kind in [TargetKind::Minimap, TargetKind::Elsewhere] {
        let ids: Vec<_> = available
            .iter()
            .filter_map(|(id, k)| (*k == kind).then_some(*id))
            .collect();
        if ids.len() < 4 {
            return Err(
                "need at least four whole target occurrences per class to split calibration safely"
                    .into(),
            );
        }
        let n_val = ids.len() / 4;
        let n_fit = if separate.is_some() {
            ids.len() - n_val
        } else {
            ids.len() / 2
        };
        training.extend(&ids[..n_fit]);
        validation.extend(&ids[n_fit..n_fit + n_val]);
        if separate.is_none() {
            testing.extend(&ids[n_fit + n_val..]);
        }
    }
    if separate.is_some() {
        testing.extend(all_eval.keys());
    }
    let screen = CalibrationScript::new(
        calibration.header.monitor_resolution,
        calibration.header.minimap.settings(),
        &crate::timer::alignment_store::AlignmentStore::default(),
        size_mm,
        distance_mm,
    )?
    .screen();
    let model = fit(
        &labelled(&calibration.frames, &training),
        &labelled(&calibration.frames, &validation),
        screen,
        FitOptions::default(),
    )?;
    let evaluation = score(trace, &model, &testing)?;
    Ok(EvaluationRun {
        model,
        evaluation,
        training_trials: training.len(),
        validation_trials: validation.len(),
    })
}
fn score(trace: &Trace, model: &CalibrationModel, ids: &BTreeSet<u32>) -> Result<Evaluation> {
    let mut result = Evaluation {
        minimap_trials: 0,
        detected_trials: 0,
        false_detections: 0,
        elsewhere_seconds: 0.0,
        usable_fraction: 0.0,
        coverage: 0.0,
        completed: trace.end.reason == EndReason::Completed,
    };
    let mut frame_count = 0;
    let mut valid_count = 0;
    let mut total_seconds = 0.0;
    let expected = trials(trace)?;
    let settled = settled_frames(&trace.frames);
    for id in ids {
        let frames: Vec<_> = settled
            .iter()
            .filter(|f| f.target.is_some_and(|t| t.id == *id))
            .collect();
        let kind = *expected.get(id).ok_or("evaluation trial outside script")?;
        let mut detector = CalibratedDetector::new(model.clone())?;
        let mut detected = false;
        let mut seconds = 0.0;
        for (index, f) in frames.iter().enumerate() {
            frame_count += 1;
            if model.predict(f.sample()).is_some() {
                valid_count += 1;
            }
            if index > 0 {
                let dt = (f.timestamp_ns - frames[index - 1].timestamp_ns) as f64 / 1e9;
                // Missing capture time cannot manufacture a large denominator.
                if dt <= 0.25 {
                    seconds += dt;
                }
            }
            let update = detector.observe(f.sample())?;
            if update.check_started {
                if kind == TargetKind::Minimap {
                    detected = true;
                } else {
                    result.false_detections += 1;
                }
            }
        }
        seconds = seconds.min(crate::script::HOLD.as_secs_f64() - SETTLING_NS as f64 / 1e9);
        if kind == TargetKind::Minimap {
            result.minimap_trials += 1;
            if detected {
                result.detected_trials += 1;
            }
        } else {
            result.elsewhere_seconds += seconds;
        }
        total_seconds += seconds;
    }
    if frame_count > 0 {
        result.usable_fraction = valid_count as f64 / frame_count as f64;
    }
    result.coverage = (total_seconds
        / (ids.len() as f64 * (crate::script::HOLD.as_secs_f64() - SETTLING_NS as f64 / 1e9)))
        .min(1.0);
    Ok(result)
}

/// Explicit defaults for replay only; UI lets the player enter physical size.
/// Every model marks the viewing distance as assumed.
pub fn default_screen_measurements() -> ([f64; 2], f64) {
    ([531.0, 299.0], 650.0)
}
pub fn screen_description(screen: Screen) -> String {
    format!(
        "{}×{} px; {:.0}×{:.0} mm; assumed distance {:.0} mm",
        screen.resolution[0],
        screen.resolution[1],
        screen.size_mm[0],
        screen.size_mm[1],
        screen.distance_mm
    )
}

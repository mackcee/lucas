//! Explicit opt-in numeric calibration evidence. No images, camera identity,
//! free-text notes, uploads, or automatic export to the saved user profile.
use crate::{
    calibration::{CalibrationScript, Observation, Stage},
    script::Phase,
};
use lucas_gaze::{
    GazeFeatures, GazeSample,
    calibration::{CalibrationModel, Validation},
    features::FeatureKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const ENV: &str = "LUCAS_CALIBRATION_DIAGNOSTICS";
pub const MAX_FRAMES: usize = 3000;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Display {
    pub monitor: lucas_store::ScreenGeometry,
    /// Actual target canvas left/top/right/bottom in physical client pixels.
    pub paint_bounds_px: Option<[f64; 4]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub timestamp_ns: u64,
    pub elapsed_ms: f64,
    pub target_id: Option<u32>,
    pub phase: Phase,
    pub visible_ms: Option<f64>,
    pub hold_ms: Option<f64>,
    pub accepted_for_fit: bool,
    pub feature_kind: FeatureKind,
    pub filtered: Option<[f32; 10]>,
    pub raw: Option<[f32; 10]>,
    pub quality: f32,
    pub acquisition_to_result_ms: f64,
    pub luminance: f32,
}
pub fn directory_from_env() -> Option<PathBuf> {
    std::env::var_os(ENV)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}
fn validation(v: Validation) -> Value {
    json!({"mean_error_degrees":v.mean_error_degrees,"true_positive_rate":v.true_positive_rate,
        "false_positive_rate":v.false_positive_rate,"usable_fraction":v.usable_fraction,
        "positives":v.positives,"negatives":v.negatives})
}
fn sample(
    source: FeatureKind,
    features: Option<[f32; 10]>,
    quality: f32,
    timestamp_ns: u64,
) -> GazeSample {
    GazeSample {
        timestamp: Duration::from_nanos(timestamp_ns),
        quality,
        features: features.map(|f| GazeFeatures::from_array_for(source, f)),
    }
}
fn prediction(model: Option<&CalibrationModel>, sample: GazeSample) -> Value {
    model.map_or(Value::Null, |m| {
        let p = m.predict(sample);
        json!({"screen":m.screen_estimate(sample),"known":p.is_some(),
            "score":p.map(|p|p.probability),"entry":p.is_some_and(|p|p.probability>=m.threshold()),
            "at_half":p.is_some_and(|p|p.probability>=0.5)})
    })
}
pub fn report(
    script: &CalibrationScript,
    observations: &[Observation],
    frames: &[Frame],
    model: Option<&CalibrationModel>,
    fit_error: Option<&str>,
    desktop: Option<&Display>,
) -> crate::trace::Result<Value> {
    if observations.len() > MAX_FRAMES || frames.len() > MAX_FRAMES {
        return Err("too many calibration diagnostic frames".into());
    }
    let s = script.screen();
    let targets:Vec<_>=script.targets().iter().map(|t|json!({"id":t.target.id,"position":t.target.position,
        "kind":t.target.kind,"stage":match t.stage {Stage::Fit=>"fit",Stage::Validate=>"validate",Stage::Quick=>"quick"},"movement":t.turned})).collect();
    let rows:Vec<_>=observations.iter().map(|o| {
        let f=o.labelled.sample.features;
        json!({"target_id":o.target_id,"target":o.labelled.target,"minimap":o.labelled.minimap,
            "timestamp_ns":o.labelled.sample.timestamp.as_nanos() as u64,
            "feature_kind":f.map(|f|f.kind()),"features":f.map(|f|f.as_array()),"quality":o.labelled.sample.quality,
            "prediction":prediction(model,o.labelled.sample)})
    }).collect();
    let captured:Vec<_>=frames.iter().map(|f|json!({"frame":f,
        "filtered_prediction":prediction(model,sample(f.feature_kind,f.filtered,f.quality,f.timestamp_ns)),
        "raw_prediction":prediction(model,sample(f.feature_kind,f.raw,f.quality,f.timestamp_ns))})).collect();
    let per_target:Vec<_>=script.targets().iter().map(|t| {
        let samples:Vec<_>=observations.iter().filter(|o|o.target_id==t.target.id).collect();
        let scores:Vec<_>=samples.iter().filter_map(|o|model.and_then(|m|m.predict(o.labelled.sample))).collect();
        let errors:Vec<_>=samples.iter().filter_map(|o|model.and_then(|m|m.screen_estimate(o.labelled.sample))).map(|p|s.angular_error(p,t.target.position)).collect();
        json!({"target_id":t.target.id,"frames":samples.len(),"known":scores.len(),
            "at_half":scores.iter().filter(|p|p.probability>=0.5).count(),
            "at_selected":scores.iter().filter(|p|model.is_some_and(|m|p.probability>=m.threshold())).count(),
            "max_score":scores.iter().map(|p|p.probability).reduce(f64::max),
            "mean_error_degrees":(!errors.is_empty()).then(||errors.iter().sum::<f64>()/errors.len() as f64)})
    }).collect();
    Ok(
        json!({"schema_version":1,"app_version":env!("CARGO_PKG_VERSION"),"numbers_only":true,"desktop":desktop,
        "screen":{"resolution":s.resolution,"size_mm":s.size_mm,"distance_mm":s.distance_mm,"distance_assumed":s.distance_assumed,"minimap":s.minimap},
        "targets":targets,"observations":rows,"captured_frames":captured,"per_target":per_target,"fit_error":fit_error,
        "model":model.map(|m|json!({"feature_kind":m.feature_kind(),"weights":m.to_bytes(),"validation":validation(m.validation()),
            "threshold":m.threshold(),"target_false_positive_rate":m.target_false_positive_rate(),
            "training_residual_scales":m.residual_scales(),"effective_entry_rectangle":m.decision_rectangle(),
            "angular_error_zone_diagnostic_only":m.inflated_zone()}))}),
    )
}
/// None does absolutely no serialization or file I/O. Failures/cancelled runs
/// are exportable too; saving diagnostics does not save/promote a calibration.
pub fn export(
    directory: Option<&Path>,
    script: &CalibrationScript,
    observations: &[Observation],
    frames: &[Frame],
    model: Option<&CalibrationModel>,
    fit_error: Option<&str>,
    desktop: Option<&Display>,
) -> crate::trace::Result<Option<PathBuf>> {
    let Some(directory) = directory else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec_pretty(&report(
        script,
        observations,
        frames,
        model,
        fit_error,
        desktop,
    )?)?;
    std::fs::create_dir_all(directory)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = directory.join(format!("calibration-{stamp}-{}.json", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(Some(path))
}

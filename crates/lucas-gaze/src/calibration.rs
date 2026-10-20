//! Per-user calibration. Pure deterministic fitting; no camera or persistence I/O.
//! Training and threshold-validation observations must come from distinct targets.
use crate::{GazeFeatures, GazeSample, features::FeatureKind};
use lucas_geometry::{Rect, ViewingGeometry, gaze_zone};
use nalgebra::{SMatrix, SVector, Vector3};

const N: usize = GazeFeatures::COUNT;
const P: usize = N + 1;
const MAGIC: &[u8; 8] = b"LUCASC02";
pub const MINIMUM_QUALITY: f32 = 0.5;
pub type Result<T> = std::result::Result<T, CalibrationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationError(pub &'static str);
impl std::fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for CalibrationError {}

/// Physical dimensions are supplied by the player, not inferred from DPI scaling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub resolution: [u32; 2],
    pub size_mm: [f64; 2],
    pub distance_mm: f64,
    pub distance_assumed: bool,
    /// Normalized monitor coordinates, left/top/right/bottom.
    pub minimap: [f64; 4],
}
impl Screen {
    pub fn validate(self) -> Result<()> {
        if self.resolution.contains(&0)
            || !self
                .size_mm
                .into_iter()
                .chain([self.distance_mm])
                .all(|x| x.is_finite() && (50.0..=5000.0).contains(&x))
            || self
                .minimap
                .iter()
                .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return Err(CalibrationError(
                "invalid screen dimensions, distance or minimap",
            ));
        }
        rect(self.minimap)?;
        Ok(())
    }
    pub fn zone(self, error_degrees: f64) -> Result<[f64; 4]> {
        self.validate()?;
        let zone = gaze_zone(
            rect(self.minimap)?,
            ViewingGeometry {
                bounds: rect([0.0, 0.0, 1.0, 1.0])?,
                width_mm: self.size_mm[0],
                height_mm: self.size_mm[1],
                distance_mm: self.distance_mm,
            },
            error_degrees,
        )
        .map_err(|_| CalibrationError("invalid validation error"))?
        .ok_or(CalibrationError("minimap is outside monitor"))?;
        Ok([zone.left(), zone.top(), zone.right(), zone.bottom()])
    }
    pub fn angular_error(self, predicted: [f64; 2], target: [f64; 2]) -> f64 {
        let ray = |p: [f64; 2]| {
            Vector3::new(
                (p[0] - 0.5) * self.size_mm[0],
                (p[1] - 0.5) * self.size_mm[1],
                self.distance_mm,
            )
            .normalize()
        };
        ray(predicted)
            .dot(&ray(target))
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }
}
fn rect(r: [f64; 4]) -> Result<Rect> {
    Rect::new(r[0], r[1], r[2], r[3]).map_err(|_| CalibrationError("invalid rectangle"))
}

#[derive(Debug, Clone, Copy)]
pub struct LabelledSample {
    pub sample: GazeSample,
    pub target: [f64; 2],
    pub minimap: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct FitOptions {
    /// Empirical per-frame operating point, NOT a guarantee on unseen recordings.
    pub target_false_positive_rate: f64,
    pub regularization: f64,
}
impl Default for FitOptions {
    fn default() -> Self {
        Self {
            target_false_positive_rate: 0.001,
            regularization: 0.01,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Validation {
    pub mean_error_degrees: f64,
    pub true_positive_rate: f64,
    pub false_positive_rate: f64,
    pub usable_fraction: f64,
    pub positives: u32,
    pub negatives: u32,
}
impl Validation {
    /// Calibration quality screen only. G2 is independently scored on unseen trials.
    pub fn passes(self, target_fpr: f64) -> bool {
        self.positives >= 12
            && self.negatives >= 12
            && self.usable_fraction >= 0.8
            && self.mean_error_degrees <= 5.0
            && self.true_positive_rate >= 0.9
            && self.false_positive_rate <= target_fpr
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationModel {
    mean: [f64; N],
    scale: [f64; N],
    /// Per-axis training residual scales for the four-sided rectangular score.
    residual: [f64; 2],
    kind: FeatureKind,
    ridge: [[f64; P]; 2],
    screen: Screen,
    threshold: f64,
    target_fpr: f64,
    validation: Validation,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prediction {
    pub probability: f64,
    /// Normalized screen estimate, for diagnostics; not a precise gaze cursor.
    pub screen: [f64; 2],
}

/// Also checks eye apertures on replay/import, which may bypass feature extraction.
pub fn usable(sample: GazeSample) -> Option<GazeFeatures> {
    let f = sample.features?;
    if !(MINIMUM_QUALITY..=1.0).contains(&sample.quality)
        || f.as_array().iter().any(|v| !v.is_finite())
        || match f {
            GazeFeatures::Landmarks(f) => {
                f.eyes.iter().any(|e| {
                    !(0.06..0.65).contains(&e.aperture)
                        || !(-0.25..=1.25).contains(&e.horizontal)
                        || !(-0.5..=1.5).contains(&e.vertical)
                }) || f.head.yaw.abs() > 1.2
                    || f.head.pitch.abs() > 1.2
                    || f.head.relative_distance <= 0.0
            }
            GazeFeatures::UniGaze(v) => {
                v[0].abs() > 1.57
                    || v[1].abs() > 1.57
                    || !(0.06..0.65).contains(&v[2])
                    || !(0.06..0.65).contains(&v[3])
                    || v[4].abs() > 1.4
                    || v[5].abs() > 1.4
                    || v[9] <= 0.0
            }
        }
    {
        return None;
    }
    Some(f)
}
impl CalibrationModel {
    pub fn feature_kind(&self) -> FeatureKind {
        self.kind
    }
    pub fn screen(&self) -> Screen {
        self.screen
    }
    pub fn threshold(&self) -> f64 {
        self.threshold
    }
    pub fn target_false_positive_rate(&self) -> f64 {
        self.target_fpr
    }
    pub fn validation(&self) -> Validation {
        self.validation
    }
    pub fn inflated_zone(&self) -> [f64; 4] {
        self.screen
            .zone(self.validation.mean_error_degrees)
            .expect("validated model geometry")
    }
    fn vector(&self, f: GazeFeatures) -> SVector<f64, P> {
        let a = f.as_array();
        SVector::from_fn(|i, _| {
            if i == N {
                1.0
            } else if self.kind == FeatureKind::UniGaze && i >= 2 {
                // Two-angle personal readout. Pose/apertures guard quality but
                // must not fit a spurious pose-to-target shortcut in 46 seconds.
                0.0
            } else {
                (f64::from(a[i]) - self.mean[i]) / self.scale[i]
            }
        })
    }
    fn point(&self, f: GazeFeatures) -> [f64; 2] {
        let x = self.vector(f);
        self.ridge.map(|w| SVector::from(w).dot(&x))
    }
    /// None = unknown (tracking loss, blink, low quality, off-screen estimate).
    /// All four edges of the actual minimap bound positive scores. Error-inflated
    /// geometry remains diagnostic only, never an independent positive check.
    pub fn predict(&self, sample: GazeSample) -> Option<Prediction> {
        let f = usable(sample)?;
        if f.kind() != self.kind {
            return None;
        }
        let point = self.point(f);
        if point
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return None;
        }
        let [l, t, r, b] = self.screen.minimap;
        // All four sides must agree: above/right is not a minimap look just
        // because a half-plane classifier extrapolates a positive score.
        let probability = [
            sigmoid((point[0] - l) / self.residual[0]),
            sigmoid((r - point[0]) / self.residual[0]),
            sigmoid((point[1] - t) / self.residual[1]),
            sigmoid((b - point[1]) / self.residual[1]),
        ]
        .into_iter()
        .fold(1.0, f64::min);
        Some(Prediction {
            probability,
            screen: point,
        })
    }
    pub fn validate_samples(&self, samples: &[LabelledSample]) -> Result<Validation> {
        check_labels(samples)?;
        if samples
            .iter()
            .filter_map(|s| s.sample.features)
            .any(|f| f.kind() != self.kind)
        {
            return Err(CalibrationError(
                "gaze source changed; recalibrate with the selected source",
            ));
        }
        let (mut positive, mut negative, mut tp, mut fp, mut valid, mut known, mut error) =
            (0, 0, 0, 0, 0, 0, 0.0);
        for s in samples {
            if s.minimap {
                positive += 1;
            } else {
                negative += 1;
            }
            if let Some(f) = usable(s.sample) {
                valid += 1;
                error += self.screen.angular_error(self.point(f), s.target);
            }
            if let Some(prediction) = self.predict(s.sample) {
                known += 1;
                if prediction.probability >= self.threshold {
                    if s.minimap {
                        tp += 1;
                    } else {
                        fp += 1;
                    }
                }
            }
        }
        if positive == 0 || negative == 0 || valid == 0 {
            return Err(CalibrationError(
                "validation needs usable minimap and elsewhere observations",
            ));
        }
        Ok(Validation {
            mean_error_degrees: error / f64::from(valid),
            true_positive_rate: f64::from(tp) / f64::from(positive),
            false_positive_rate: f64::from(fp) / f64::from(negative),
            usable_fraction: f64::from(known) / samples.len() as f64,
            positives: positive,
            negatives: negative,
        })
    }
    /// Fixed little-endian format, versioned by magic. Only weights, geometry and
    /// validation aggregates; no per-frame features. Version 2 pins the source
    /// and rectangular decision; old half-plane calibrations must be repeated.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        for v in self.screen.resolution {
            b.extend(v.to_le_bytes());
        }
        b.push(u8::from(self.screen.distance_assumed));
        b.push(u8::from(self.kind == FeatureKind::UniGaze));
        for v in self
            .mean
            .into_iter()
            .chain(self.scale)
            .chain(self.residual)
            .chain(self.ridge.into_iter().flatten())
            .chain(self.screen.size_mm)
            .chain([self.screen.distance_mm])
            .chain(self.screen.minimap)
            .chain([
                self.threshold,
                self.target_fpr,
                self.validation.mean_error_degrees,
                self.validation.true_positive_rate,
                self.validation.false_positive_rate,
                self.validation.usable_fraction,
            ])
        {
            b.extend(v.to_le_bytes());
        }
        b.extend(self.validation.positives.to_le_bytes());
        b.extend(self.validation.negatives.to_le_bytes());
        b
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        const LEN: usize = 8 + 8 + 2 + (N * 2 + 2 + P * 2 + 7 + 6) * 8 + 8;
        if bytes.len() != LEN || &bytes[..8] != MAGIC || bytes[16] > 1 || bytes[17] > 1 {
            return Err(CalibrationError(
                "unsupported or corrupt calibration weights",
            ));
        }
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let kind = if bytes[17] == 0 {
            FeatureKind::MediaPipe
        } else {
            FeatureKind::UniGaze
        };
        let mut cursor = 18;
        let mut next = || {
            let v = f64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;
            v
        };
        let mean = std::array::from_fn(|_| next());
        let scale = std::array::from_fn(|_| next());
        let residual = std::array::from_fn(|_| next());
        let ridge = std::array::from_fn(|_| std::array::from_fn(|_| next()));
        let size_mm = [next(), next()];
        let distance_mm = next();
        let minimap = std::array::from_fn(|_| next());
        let threshold = next();
        let target_fpr = next();
        let validation = Validation {
            mean_error_degrees: next(),
            true_positive_rate: next(),
            false_positive_rate: next(),
            usable_fraction: next(),
            positives: u32_at(LEN - 8),
            negatives: u32_at(LEN - 4),
        };
        let model = Self {
            mean,
            scale,
            residual,
            kind,
            ridge,
            screen: Screen {
                resolution: [u32_at(8), u32_at(12)],
                size_mm,
                distance_mm,
                distance_assumed: bytes[16] == 1,
                minimap,
            },
            threshold,
            target_fpr,
            validation,
        };
        model.screen.validate()?;
        model.screen.zone(validation.mean_error_degrees)?;
        if model
            .mean
            .into_iter()
            .chain(model.scale)
            .chain(model.residual)
            .chain(model.ridge.into_iter().flatten())
            .any(|x| !x.is_finite())
            || model.scale.iter().any(|x| *x < 1e-3)
            || model.residual.iter().any(|x| *x < 1e-3)
            || !(0.5..=1.0).contains(&threshold)
            || !(0.0..=0.1).contains(&target_fpr)
            || [
                validation.true_positive_rate,
                validation.false_positive_rate,
                validation.usable_fraction,
            ]
            .iter()
            .any(|x| !(0.0..=1.0).contains(x))
            || validation.positives < 12
            || validation.negatives < 12
        {
            return Err(CalibrationError("invalid calibration parameters"));
        }
        Ok(model)
    }
}
fn sigmoid(x: f64) -> f64 {
    (1.0 / (1.0 + (-x.clamp(-35.0, 35.0)).exp())).clamp(f64::EPSILON, 1.0 - f64::EPSILON)
}
fn check_labels(samples: &[LabelledSample]) -> Result<()> {
    if samples.len() > 120_000
        || samples.iter().any(|s| {
            s.target
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        })
    {
        return Err(CalibrationError(
            "invalid or excessive calibration observations",
        ));
    }
    Ok(())
}
/// Conservative quantile: ties at the boundary are excluded, never rounded down.
fn select_threshold(mut negatives: Vec<f64>, fpr: f64) -> f64 {
    negatives.sort_by(|a, b| b.total_cmp(a));
    let allowed = (fpr * negatives.len() as f64).floor() as usize;
    negatives[allowed].next_up().clamp(0.5, 1.0)
}
pub fn fit(
    training: &[LabelledSample],
    validation: &[LabelledSample],
    screen: Screen,
    options: FitOptions,
) -> Result<CalibrationModel> {
    screen.validate()?;
    check_labels(training)?;
    check_labels(validation)?;
    if !(0.0..=0.1).contains(&options.target_false_positive_rate)
        || !options.regularization.is_finite()
        || options.regularization <= 0.0
    {
        return Err(CalibrationError("invalid calibration fit options"));
    }
    let rows: Vec<_> = training
        .iter()
        .filter_map(|s| usable(s.sample).map(|f| (*s, f)))
        .collect();
    let positives = rows.iter().filter(|(s, _)| s.minimap).count();
    let negatives = rows.len() - positives;
    if positives < 12 || negatives < 12 {
        return Err(CalibrationError(
            "need at least 12 usable training frames of each class; improve lighting and retry",
        ));
    }
    let kind = rows[0].1.kind();
    if rows.iter().any(|(_, f)| f.kind() != kind) {
        return Err(CalibrationError("cannot mix gaze sources in calibration"));
    }
    let mut m = CalibrationModel {
        mean: [0.0; N],
        scale: [0.0; N],
        residual: [1e-3; 2],
        kind,
        ridge: [[0.0; P]; 2],
        screen,
        threshold: 0.5,
        target_fpr: options.target_false_positive_rate,
        validation: Validation {
            mean_error_degrees: 0.0,
            true_positive_rate: 0.0,
            false_positive_rate: 0.0,
            usable_fraction: 0.0,
            positives: 0,
            negatives: 0,
        },
    };
    for (_, f) in &rows {
        for (mean, x) in m.mean.iter_mut().zip(f.as_array()) {
            *mean += f64::from(x) / rows.len() as f64;
        }
    }
    for (_, f) in &rows {
        for (i, x) in f.as_array().iter().enumerate() {
            m.scale[i] += (f64::from(*x) - m.mean[i]).powi(2) / rows.len() as f64;
        }
    }
    m.scale = m.scale.map(|s| s.sqrt().max(1e-3));
    let vectors: Vec<_> = rows.iter().map(|(s, f)| (*s, m.vector(*f))).collect();
    let mut gram = SMatrix::<f64, P, P>::zeros();
    let mut rhs = [SVector::<f64, P>::zeros(); 2];
    for (s, x) in &vectors {
        gram += x * x.transpose() / rows.len() as f64;
        for (axis, rhs) in rhs.iter_mut().enumerate() {
            *rhs += x * s.target[axis] / rows.len() as f64;
        }
    }
    for i in 0..P {
        gram[(i, i)] += if i == N { 1e-9 } else { options.regularization };
    }
    let solve = gram
        .cholesky()
        .ok_or(CalibrationError("singular calibration fit"))?;
    m.ridge = rhs.map(|b| solve.solve(&b).into());
    m.residual = std::array::from_fn(|axis| {
        (rows
            .iter()
            .map(|(s, f)| (m.point(*f)[axis] - s.target[axis]).powi(2))
            .sum::<f64>()
            / rows.len() as f64)
            .sqrt()
            .max(1e-3)
    });
    for minimap in [false, true] {
        if validation
            .iter()
            .filter(|s| s.minimap == minimap && usable(s.sample).is_some())
            .count()
            < 12
        {
            return Err(CalibrationError(
                "need at least 12 usable held-out frames of each class",
            ));
        }
    }
    // Error inflation is measured on held-out targets, never on the training fit.
    m.validation = m.validate_samples(validation)?;
    if m.validation.positives < 12 || m.validation.negatives < 12 {
        return Err(CalibrationError(
            "need at least 12 held-out frames of each class",
        ));
    }
    let negative_scores: Vec<_> = validation
        .iter()
        .filter(|s| !s.minimap)
        .map(|s| m.predict(s.sample).map_or(0.0, |p| p.probability))
        .collect();
    m.threshold = select_threshold(negative_scores, options.target_false_positive_rate);
    m.validation = m.validate_samples(validation)?;
    // Validate the complete persisted contract before exposing it.
    CalibrationModel::from_bytes(&m.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn threshold_excludes_ties_at_the_false_positive_budget() {
        let negatives = vec![0.7; 100];
        let threshold = select_threshold(negatives.clone(), 0.01);
        assert_eq!(negatives.iter().filter(|p| **p >= threshold).count(), 0);
        assert!(threshold > 0.7);
        let scores: Vec<_> = (0..100).map(|i| i as f64 / 100.0).collect();
        let threshold = select_threshold(scores.clone(), 0.01);
        assert_eq!(scores.iter().filter(|p| **p >= threshold).count(), 1);
    }
}

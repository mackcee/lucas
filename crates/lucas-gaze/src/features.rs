//! Eye-relative iris features and a weak-perspective canonical-face fit.
//! These are calibration inputs, not an uncalibrated screen point or metric pose.
use crate::{FaceLandmarks, Landmark};
use nalgebra::{Matrix3, Rotation3, Vector3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeFeatures {
    /// 0=left / 1=right corner in the unmirrored image, not anatomical left/right.
    pub horizontal: f32,
    /// 0=upper lid / 1=lower lid, projected perpendicular to the corner axis.
    pub vertical: f32,
    /// Lid separation / corner separation; low aperture is not a usable gaze.
    pub aperture: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadPose {
    /// Radians about camera y (yaw), x (pitch), z (roll); y down, z away.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Inverse fitted face scale in image-width units. Relative, NOT metres.
    pub relative_distance: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandmarkFeatures {
    /// Anatomical right eye (image-left), then anatomical left eye (image-right).
    pub eyes: [EyeFeatures; 2],
    pub head: HeadPose,
}
impl LandmarkFeatures {
    pub const COUNT: usize = 10;
    pub fn as_array(self) -> [f32; Self::COUNT] {
        [
            self.eyes[0].horizontal,
            self.eyes[0].vertical,
            self.eyes[0].aperture,
            self.eyes[1].horizontal,
            self.eyes[1].vertical,
            self.eyes[1].aperture,
            self.head.yaw,
            self.head.pitch,
            self.head.roll,
            self.head.relative_distance,
        ]
    }
    pub fn from_array(v: [f32; Self::COUNT]) -> Self {
        Self {
            eyes: [
                EyeFeatures {
                    horizontal: v[0],
                    vertical: v[1],
                    aperture: v[2],
                },
                EyeFeatures {
                    horizontal: v[3],
                    vertical: v[4],
                    aperture: v[5],
                },
            ],
            head: HeadPose {
                yaw: v[6],
                pitch: v[7],
                roll: v[8],
                relative_distance: v[9],
            },
        }
    }
}

/// Feature identity is persisted with calibration and traces; models are never
/// interchangeable. UniGaze angles are camera-space radians, not iris ratios.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    #[default]
    MediaPipe,
    UniGaze,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GazeFeatures {
    Landmarks(LandmarkFeatures),
    /// pitch, yaw, right/left aperture, head pitch/yaw/roll, face x/y/z (mm).
    UniGaze([f32; 10]),
}
impl GazeFeatures {
    pub const COUNT: usize = 10;
    pub fn kind(self) -> FeatureKind {
        match self {
            Self::Landmarks(_) => FeatureKind::MediaPipe,
            Self::UniGaze(_) => FeatureKind::UniGaze,
        }
    }
    pub fn as_array(self) -> [f32; 10] {
        match self {
            Self::Landmarks(f) => f.as_array(),
            Self::UniGaze(v) => v,
        }
    }
    pub fn from_array(v: [f32; 10]) -> Self {
        Self::Landmarks(LandmarkFeatures::from_array(v))
    }
    pub fn from_array_for(kind: FeatureKind, v: [f32; 10]) -> Self {
        match kind {
            FeatureKind::MediaPipe => Self::from_array(v),
            FeatureKind::UniGaze => Self::UniGaze(v),
        }
    }
    pub fn landmarks_mut(&mut self) -> Option<&mut LandmarkFeatures> {
        match self {
            Self::Landmarks(f) => Some(f),
            Self::UniGaze(_) => None,
        }
    }
}

/// Returns None for closed/occluded eyes, invalid geometry, out-of-frame eyes,
/// or a poor fit. Quality is a [0,1] heuristic, NOT calibrated accuracy.
pub fn extract(face: &FaceLandmarks) -> Option<(GazeFeatures, f32)> {
    let (w, h) = face.image_size;
    if w == 0
        || h == 0
        || !face.presence.is_finite()
        || !(0.0..=1.0).contains(&face.presence)
        || face.points.iter().any(|p| {
            !(-2.0..=3.0).contains(&p.x)
                || !(-2.0..=3.0).contains(&p.y)
                || !(-4.0..=4.0).contains(&p.z)
        })
    {
        return None;
    }
    let points = &face.points;
    let eyes = [
        eye(points, [33, 133, 159, 145, 468], w, h)?,
        eye(points, [362, 263, 386, 374, 473], w, h)?,
    ];
    let (head, residual) = fit_head(face)?;
    let aperture = eyes
        .iter()
        .map(|e| ((e.aperture - 0.06) / 0.12).clamp(0.0, 1.0))
        .fold(1.0_f32, f32::min);
    let fit_quality = (1.0 - residual / 0.25).clamp(0.0, 1.0);
    let resolution = ((points[263].x - points[33].x).abs() * w as f32 / 80.0).clamp(0.0, 1.0);
    let quality = face.presence * aperture * fit_quality * resolution;
    if quality <= 0.0 {
        return None;
    }
    Some((
        GazeFeatures::Landmarks(LandmarkFeatures { eyes, head }),
        quality,
    ))
}
fn eye(points: &[Landmark; 478], indices: [usize; 5], w: u32, h: u32) -> Option<EyeFeatures> {
    let p = indices.map(|i| points[i]);
    if p.iter()
        .any(|p| !(0.0..=1.0).contains(&p.x) || !(0.0..=1.0).contains(&p.y))
    {
        return None;
    }
    let xy = p.map(|p| [p.x * w as f32, p.y * h as f32]);
    let dx = xy[1][0] - xy[0][0];
    let dy = xy[1][1] - xy[0][1];
    let width = dx.hypot(dy);
    if width < 4.0 {
        return None;
    }
    let along =
        |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]) * dx + (a[1] - b[1]) * dy) / (width * width);
    let across = |a: [f32; 2], b: [f32; 2]| (-(a[0] - b[0]) * dy + (a[1] - b[1]) * dx) / width;
    let gap = across(xy[3], xy[2]);
    let aperture = gap / width;
    if !(0.06..0.65).contains(&aperture) {
        return None;
    }
    let horizontal = along(xy[4], xy[0]);
    let vertical = across(xy[4], xy[2]) / gap;
    if !(-0.25..=1.25).contains(&horizontal) || !(-0.5..=1.5).contains(&vertical) {
        return None;
    }
    Some(EyeFeatures {
        horizontal,
        vertical,
        aperture,
    })
}

// MediaPipe canonical_face_model.obj v0.10.32, vertices 33,263,1,152,10,61,291.
// Apache-2.0, Copyright 2020 The MediaPipe Authors. See root NOTICE.
const CANONICAL: [(usize, [f32; 3]); 7] = [
    (33, [-4.445859, 2.663991, 3.173422]),
    (263, [4.445859, 2.663991, 3.173422]),
    (1, [0.0, -1.126865, 7.475604]),
    (152, [0.0, -9.403378, 4.264492]),
    (10, [0.0, 8.261778, 4.481535]),
    (61, [-2.456206, -4.342621, 4.283884]),
    (291, [2.456206, -4.342621, 4.283884]),
];
fn fit_head(face: &FaceLandmarks) -> Option<(HeadPose, f32)> {
    let aspect = face.image_size.1 as f32 / face.image_size.0 as f32;
    // Canonical interocular span = 1; turn canonical y-up/z-forward to camera axes.
    let canonical = CANONICAL.map(|(_, p)| Vector3::new(p[0], -p[1], -p[2]) / 8.891718);
    let observed = CANONICAL.map(|(i, _)| {
        let p = face.points[i];
        Vector3::new(p.x, p.y * aspect, p.z)
    });
    let mean_c = canonical.iter().sum::<Vector3<f32>>() / 7.0;
    let mean_o = observed.iter().sum::<Vector3<f32>>() / 7.0;
    let c = canonical.map(|v| v - mean_c);
    let o = observed.map(|v| v - mean_o);
    let mut covariance = Matrix3::zeros();
    for (c, o) in c.iter().zip(o) {
        covariance += c * o.transpose();
    }
    let svd = covariance.svd(true, true);
    if svd.singular_values[1] < 1e-7 {
        return None;
    }
    let u = svd.u?;
    let vt = svd.v_t?;
    let mut correction = Matrix3::identity();
    correction[(2, 2)] = (vt.transpose() * u.transpose()).determinant().signum();
    let rotation = vt.transpose() * correction * u.transpose();
    let denominator: f32 = c.iter().map(|v| v.norm_squared()).sum();
    let scale: f32 = c
        .iter()
        .zip(o)
        .map(|(c, o)| o.dot(&(rotation * c)))
        .sum::<f32>()
        / denominator;
    if !scale.is_finite() || scale <= 0.01 {
        return None;
    }
    let residual = (c
        .iter()
        .zip(o)
        .map(|(c, o)| (o - rotation * c * scale).norm_squared())
        .sum::<f32>()
        / 7.0)
        .sqrt()
        / scale;
    let (pitch, yaw, roll) = Rotation3::from_matrix_unchecked(rotation).euler_angles();
    Some((
        HeadPose {
            yaw,
            pitch,
            roll,
            relative_distance: 1.0 / scale,
        },
        residual,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_similarity_recovers_pose_and_scale() {
        let mut face = FaceLandmarks {
            points: [Landmark::default(); 478],
            presence: 1.0,
            timestamp: std::time::Duration::ZERO,
            image_size: (1280, 720),
        };
        let rotation = Rotation3::from_euler_angles(0.15, -0.2, 0.1);
        for (i, p) in CANONICAL {
            let p = rotation * (Vector3::new(p[0], -p[1], -p[2]) / 8.891718) * 0.3
                + Vector3::new(0.5, 0.3, 0.0);
            face.points[i] = Landmark {
                x: p.x,
                y: p.y * 1280.0 / 720.0,
                z: p.z,
            };
        }
        let (head, residual) = fit_head(&face).unwrap();
        assert!((head.pitch - 0.15).abs() < 1e-5);
        assert!((head.yaw + 0.2).abs() < 1e-5);
        assert!((head.roll - 0.1).abs() < 1e-5);
        assert!((head.relative_distance - 1.0 / 0.3).abs() < 1e-5);
        assert!(residual < 1e-5);
    }
    #[test]
    fn iris_is_relative_to_eye_not_frame_and_blinks_are_unknown() {
        let mut p = [Landmark::default(); 478];
        for (i, x, y) in [
            (33, 0.2, 0.3),
            (133, 0.4, 0.3),
            (159, 0.3, 0.28),
            (145, 0.3, 0.32),
            (468, 0.3, 0.3),
        ] {
            p[i] = Landmark { x, y, z: 0.0 };
        }
        let e = eye(&p, [33, 133, 159, 145, 468], 1000, 1000).unwrap();
        assert!((e.horizontal - 0.5).abs() < 1e-6 && (e.vertical - 0.5).abs() < 1e-6);
        p[145].y = p[159].y;
        assert!(eye(&p, [33, 133, 159, 145, 468], 1000, 1000).is_none());
    }
}

//! MediaPipe short-range BlazeFace + Face Landmarker v2 decoding and ROI tracking.
//! Constants follow MediaPipe v0.10.32 face_detector_graph.cc and
//! face_landmarks_detector_graph.cc (Apache-2.0; see NOTICE).

use crate::{Error, Frame, Result, runtime::InferenceBackend};
use std::time::Duration;
use zeroize::Zeroize;

pub const LANDMARK_COUNT: usize = 478;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Landmark {
    /// Unmirrored full-image coordinates: x / width, y / height, z / width.
    /// z points away from the camera; its origin is model-relative, not metric.
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
#[derive(Clone)]
pub struct FaceLandmarks {
    pub points: [Landmark; LANDMARK_COUNT],
    pub presence: f32,
    pub timestamp: Duration,
    pub image_size: (u32, u32),
}
pub trait Landmarker {
    fn locate(&mut self, frame: &Frame) -> Result<Option<FaceLandmarks>>;
    fn reset(&mut self);
}

#[derive(Clone, Copy, Debug)]
struct Roi {
    cx: f32,
    cy: f32,
    side: f32,
    angle: f32,
}
impl Roi {
    fn project(&self, x: f32, y: f32) -> (f32, f32) {
        let (sin, cos) = self.angle.sin_cos();
        (
            self.cx + self.side * (cos * (x - 0.5) - sin * (y - 0.5)),
            self.cy + self.side * (sin * (x - 0.5) + cos * (y - 0.5)),
        )
    }
    fn valid(&self, width: u32, height: u32) -> bool {
        [self.cx, self.cy, self.side, self.angle]
            .iter()
            .all(|x| x.is_finite())
            && self.side >= 8.0
            && self.side <= width.max(height) as f32 * 2.0
            && self.cx >= 0.0
            && self.cx < width as f32
            && self.cy >= 0.0
            && self.cy < height as f32
    }
}
#[derive(Default, Debug, Clone, Copy)]
pub struct InferenceStats {
    pub detections: u64,
    pub meshes: u64,
}

pub struct MediaPipeLandmarker<B> {
    backend: B,
    roi: Option<Roi>,
    previous: Option<(Duration, u32, u32)>,
    detector_input: Vec<f32>,
    mesh_input: Vec<f32>,
    stats: InferenceStats,
}
impl<B: InferenceBackend> MediaPipeLandmarker<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            roi: None,
            previous: None,
            detector_input: vec![0.0; 128 * 128 * 3],
            mesh_input: vec![0.0; 256 * 256 * 3],
            stats: InferenceStats::default(),
        }
    }
    pub fn stats(&self) -> InferenceStats {
        self.stats
    }
    fn locate_inner(&mut self, frame: &Frame) -> Result<Option<FaceLandmarks>> {
        let (w, h) = (frame.width(), frame.height());
        if let Some((t, pw, ph)) = self.previous {
            if frame.timestamp() <= t {
                return Err(Error::InvalidTimestamp);
            }
            if frame.timestamp() - t > Duration::from_millis(250) || (w, h) != (pw, ph) {
                self.roi = None;
            }
        }
        self.previous = Some((frame.timestamp(), w, h));
        let roi = match self.roi {
            Some(roi) => roi,
            None => {
                let whole = Roi {
                    cx: w as f32 / 2.0,
                    cy: h as f32 / 2.0,
                    side: w.max(h) as f32,
                    angle: 0.0,
                };
                sample(frame, whole, 128, true, &mut self.detector_input);
                self.stats.detections += 1;
                let result = self.backend.detect(&self.detector_input);
                self.detector_input.as_mut_slice().zeroize();
                let output = result?;
                let Some(roi) = decode_detection(&output.boxes, &output.scores, whole)? else {
                    return Ok(None);
                };
                if !roi.valid(w, h) {
                    return Ok(None);
                }
                roi
            }
        };
        sample(frame, roi, 256, false, &mut self.mesh_input);
        self.stats.meshes += 1;
        let result = self.backend.mesh(&self.mesh_input);
        self.mesh_input.as_mut_slice().zeroize();
        let output = result?;
        if output.coordinates.len() != LANDMARK_COUNT * 3
            || !output.presence_logit.is_finite()
            || !output.coordinates.iter().all(|v| v.is_finite())
        {
            return Err(Error::InvalidOutput);
        }
        let presence = sigmoid(output.presence_logit);
        if presence < 0.5 {
            self.roi = None;
            return Ok(None);
        }
        let points = std::array::from_fn(|i| {
            let (x, y) = roi.project(
                output.coordinates[i * 3] / 256.0,
                output.coordinates[i * 3 + 1] / 256.0,
            );
            Landmark {
                x: x / w as f32,
                y: y / h as f32,
                z: output.coordinates[i * 3 + 2] / 256.0 * roi.side / w as f32,
            }
        });
        self.roi = next_roi(&points, w, h).filter(|r| r.valid(w, h));
        if self.roi.is_none() {
            return Ok(None);
        }
        Ok(Some(FaceLandmarks {
            points,
            presence,
            timestamp: frame.timestamp(),
            image_size: (w, h),
        }))
    }
}
impl<B: InferenceBackend> Landmarker for MediaPipeLandmarker<B> {
    fn locate(&mut self, frame: &Frame) -> Result<Option<FaceLandmarks>> {
        let result = self.locate_inner(frame);
        if result.is_err() {
            self.reset();
        }
        result
    }
    fn reset(&mut self) {
        self.roi = None;
        self.previous = None;
    }
}
impl<B> Drop for MediaPipeLandmarker<B> {
    fn drop(&mut self) {
        self.detector_input.zeroize();
        self.mesh_input.zeroize();
    }
}
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x.clamp(-100.0, 100.0)).exp())
}

/// Two anchors per stride-8 cell, six per stride-16 cell: 512+384=896.
fn anchor(index: usize) -> (f32, f32) {
    let (cell, size) = if index < 512 {
        (index / 2, 16)
    } else {
        ((index - 512) / 6, 8)
    };
    (
        ((cell % size) as f32 + 0.5) / size as f32,
        ((cell / size) as f32 + 0.5) / size as f32,
    )
}
#[derive(Clone, Copy)]
struct Detection {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    eyes: [f32; 4],
    score: f32,
}
fn iou(a: Detection, b: Detection) -> f32 {
    let width =
        ((a.x + a.w / 2.0).min(b.x + b.w / 2.0) - (a.x - a.w / 2.0).max(b.x - b.w / 2.0)).max(0.0);
    let height =
        ((a.y + a.h / 2.0).min(b.y + b.h / 2.0) - (a.y - a.h / 2.0).max(b.y - b.h / 2.0)).max(0.0);
    let intersection = width * height;
    intersection / (a.w * a.h + b.w * b.h - intersection).max(f32::EPSILON)
}
fn decode_detection(boxes: &[f32], scores: &[f32], image: Roi) -> Result<Option<Roi>> {
    if boxes.len() != 896 * 16
        || scores.len() != 896
        || !boxes.iter().chain(scores).all(|x| x.is_finite())
    {
        return Err(Error::InvalidOutput);
    }
    let mut detections = Vec::new();
    for (i, &logit) in scores.iter().enumerate() {
        let score = sigmoid(logit);
        if score < 0.6 {
            continue;
        }
        let r = &boxes[i * 16..i * 16 + 16];
        let (ax, ay) = anchor(i);
        if r[2] <= 0.0 || r[3] <= 0.0 {
            continue;
        }
        detections.push(Detection {
            x: ax + r[0] / 128.0,
            y: ay + r[1] / 128.0,
            w: r[2] / 128.0,
            h: r[3] / 128.0,
            eyes: [
                ax + r[4] / 128.0,
                ay + r[5] / 128.0,
                ax + r[6] / 128.0,
                ay + r[7] / 128.0,
            ],
            score,
        });
    }
    let Some(best) = detections
        .iter()
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .copied()
    else {
        return Ok(None);
    };
    // Single-player weighted NMS: merge overlaps of the most confident face.
    let mut sum = [0.0; 8];
    let mut weight = 0.0;
    for d in detections.iter().filter(|d| iou(best, **d) > 0.3) {
        let values = [
            d.x, d.y, d.w, d.h, d.eyes[0], d.eyes[1], d.eyes[2], d.eyes[3],
        ];
        for (s, v) in sum.iter_mut().zip(values) {
            *s += v * d.score;
        }
        weight += d.score;
    }
    for s in &mut sum {
        *s /= weight;
    }
    let (cx, cy) = image.project(sum[0], sum[1]);
    Ok(Some(Roi {
        cx,
        cy,
        side: sum[2].max(sum[3]) * image.side * 1.5,
        angle: (sum[7] - sum[5]).atan2(sum[6] - sum[4]),
    }))
}
fn next_roi(points: &[Landmark; LANDMARK_COUNT], w: u32, h: u32) -> Option<Roi> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for p in &points[..468] {
        min_x = min_x.min(p.x * w as f32);
        max_x = max_x.max(p.x * w as f32);
        min_y = min_y.min(p.y * h as f32);
        max_y = max_y.max(p.y * h as f32);
    }
    let a = points[33];
    let b = points[263];
    if ((b.x - a.x) * w as f32).hypot((b.y - a.y) * h as f32) < 4.0 {
        return None;
    }
    Some(Roi {
        cx: (min_x + max_x) / 2.0,
        cy: (min_y + max_y) / 2.0,
        side: (max_x - min_x).max(max_y - min_y) * 1.5,
        angle: ((b.y - a.y) * h as f32).atan2((b.x - a.x) * w as f32),
    })
}

/// Bilinear NHWC resampling. Detector uses zero letterbox padding and [-1,1];
/// mesh uses border replication and [0,1], as in the model metadata.
fn sample(frame: &Frame, roi: Roi, size: usize, detector: bool, output: &mut [f32]) {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    for y in 0..size {
        for x in 0..size {
            let (sx, sy) = roi.project(
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            let outside = sx < 0.0 || sy < 0.0 || sx >= w as f32 || sy >= h as f32;
            let sx = (sx - 0.5).clamp(0.0, (w - 1) as f32);
            let sy = (sy - 0.5).clamp(0.0, (h - 1) as f32);
            let x0 = sx as usize;
            let y0 = sy as usize;
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let dx = sx - x0 as f32;
            let dy = sy - y0 as f32;
            for c in 0..3 {
                let read = |x, y| frame.rgb()[(y * w + x) * 3 + c] as f32;
                let value = if detector && outside {
                    0.0
                } else {
                    (read(x0, y0) * (1.0 - dx) + read(x1, y0) * dx) * (1.0 - dy)
                        + (read(x0, y1) * (1.0 - dx) + read(x1, y1) * dx) * dy
                };
                output[(y * size + x) * 3 + c] = if detector {
                    value / 127.5 - 1.0
                } else {
                    value / 255.0
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct BrokenMesh;
    impl InferenceBackend for BrokenMesh {
        fn detect(&mut self, _: &[f32]) -> Result<crate::runtime::DetectionOutput> {
            let mut boxes = vec![0.0; 896 * 16];
            let mut scores = vec![-100.0; 896];
            let (ax, ay) = anchor(0);
            scores[0] = 10.0;
            boxes[..8].copy_from_slice(&[
                (0.5 - ax) * 128.0,
                (0.5 - ay) * 128.0,
                40.0,
                40.0,
                (0.4 - ax) * 128.0,
                (0.5 - ay) * 128.0,
                (0.6 - ax) * 128.0,
                (0.5 - ay) * 128.0,
            ]);
            Ok(crate::runtime::DetectionOutput { boxes, scores })
        }
        fn mesh(&mut self, _: &[f32]) -> Result<crate::runtime::MeshOutput> {
            Ok(crate::runtime::MeshOutput {
                coordinates: vec![0.0; 1433],
                presence_logit: 10.0,
            })
        }
    }
    #[test]
    fn malformed_mesh_resets_tracking_and_scrubs_staging() {
        let mut tracker = MediaPipeLandmarker::new(BrokenMesh);
        let mut frame = Frame::new(640, 480).unwrap();
        frame.rgb_mut().fill(77);
        assert!(matches!(tracker.locate(&frame), Err(Error::InvalidOutput)));
        assert!(tracker.roi.is_none() && tracker.previous.is_none());
        assert!(
            tracker
                .detector_input
                .iter()
                .chain(&tracker.mesh_input)
                .all(|v| *v == 0.0)
        );
        assert!(matches!(tracker.locate(&frame), Err(Error::InvalidOutput)));
        assert_eq!(tracker.stats.detections, 2);
    }
    #[test]
    fn malformed_detector_outputs_fail_closed() {
        let roi = Roi {
            cx: 320.0,
            cy: 240.0,
            side: 640.0,
            angle: 0.0,
        };
        assert!(matches!(
            decode_detection(&[], &[], roi),
            Err(Error::InvalidOutput)
        ));
        let mut boxes = vec![0.0; 896 * 16];
        let scores = vec![-100.0; 896];
        boxes[0] = f32::NAN;
        assert!(matches!(
            decode_detection(&boxes, &scores, roi),
            Err(Error::InvalidOutput)
        ));
    }
    #[test]
    fn weighted_nms_merges_overlapping_anchors() {
        let mut boxes = vec![0.0; 896 * 16];
        let mut scores = vec![-100.0; 896];
        for i in 0..2 {
            scores[i] = 2.0;
            let (ax, ay) = anchor(i);
            let x = 0.49 + i as f32 * 0.02;
            let r = &mut boxes[i * 16..i * 16 + 16];
            r[0] = (x - ax) * 128.0;
            r[1] = (0.5 - ay) * 128.0;
            r[2] = 40.0;
            r[3] = 40.0;
            r[4] = (x - 0.1 - ax) * 128.0;
            r[5] = (0.5 - ay) * 128.0;
            r[6] = (x + 0.1 - ax) * 128.0;
            r[7] = r[5];
        }
        let roi = decode_detection(
            &boxes,
            &scores,
            Roi {
                cx: 320.0,
                cy: 240.0,
                side: 640.0,
                angle: 0.0,
            },
        )
        .unwrap()
        .unwrap();
        assert!((roi.cx - 320.0).abs() < 0.01 && (roi.cy - 240.0).abs() < 0.01);
        assert!((roi.side - 300.0).abs() < 0.01 && roi.angle.abs() < 0.001);
    }
    #[test]
    fn anchor_layout_and_letterbox_projection() {
        assert_eq!(anchor(0), (0.03125, 0.03125));
        assert_eq!(anchor(511), (0.96875, 0.96875));
        assert_eq!(anchor(512), (0.0625, 0.0625));
        assert_eq!(anchor(895), (0.9375, 0.9375));
        let roi = Roi {
            cx: 640.0,
            cy: 360.0,
            side: 1280.0,
            angle: 0.0,
        };
        assert_eq!(roi.project(0.0, 0.0), (0.0, -280.0));
    }
    #[test]
    fn normalization_padding_and_rotation() {
        let mut f = Frame::new(4, 2).unwrap();
        f.rgb_mut().fill(255);
        let mut out = vec![0.0; 4 * 4 * 3];
        sample(
            &f,
            Roi {
                cx: 2.0,
                cy: 1.0,
                side: 4.0,
                angle: 0.0,
            },
            4,
            true,
            &mut out,
        );
        assert_eq!(&out[..12], &[-1.0; 12]);
        assert_eq!(&out[12..24], &[1.0; 12]);
        let roi = Roi {
            cx: 0.0,
            cy: 0.0,
            side: 2.0,
            angle: std::f32::consts::FRAC_PI_2,
        };
        let (x, y) = roi.project(1.0, 0.5);
        assert!(x.abs() < 1e-6 && (y - 1.0).abs() < 1e-6);
    }
}

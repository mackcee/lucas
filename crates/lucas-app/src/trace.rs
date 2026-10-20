//! Versioned JSON-lines traces. Only scalar metadata and derived numeric features
//! can cross this boundary; neither Frame nor image buffers are accepted.
use crate::{
    script::{Phase, Target, TargetKind},
    settings::MinimapSelection,
};
use lucas_gaze::{GazeFeatures, GazeSample, GazeSource, source::TraceGazeSource};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
    time::Duration,
};

pub const FEATURE_NAMES: [&str; 10] = [
    "right_eye_horizontal",
    "right_eye_vertical",
    "right_eye_aperture",
    "left_eye_horizontal",
    "left_eye_vertical",
    "left_eye_aperture",
    "head_yaw_rad",
    "head_pitch_rad",
    "head_roll_rad",
    "head_relative_distance",
];
const MAX_LINE: u64 = 16 * 1024;
const MAX_FRAMES: usize = 120_000; // Covers a maximum 600-second trace at 60 fps with headroom.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraInfo {
    pub name: String,
    pub resolution: [u32; 2],
    /// Negotiated capture rate; observed frame rate is computed from the samples.
    pub fps: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub schema_version: u32,
    pub app_version: String,
    pub started_unix_ms: u64,
    pub camera: CameraInfo,
    pub monitor_resolution: [u32; 2],
    pub dpi_scale: f64,
    pub lighting_note: String,
    pub script_seconds: u64,
    pub minimap: MinimapSelection,
}
impl Header {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err("unsupported trace schema version".into());
        }
        if self.camera.name.len() > 1024
            || self.app_version.len() > 128
            || self.lighting_note.len() > 2048
            || self.camera.resolution.contains(&0)
            || !(1..=60).contains(&self.camera.fps)
            || !self.dpi_scale.is_finite()
            || !(0.5..=8.0).contains(&self.dpi_scale)
        {
            return Err("invalid trace metadata".into());
        }
        crate::script::Script::new(
            self.script_seconds,
            self.monitor_resolution,
            self.minimap.settings(),
        )?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameRecord {
    /// Camera acquisition time relative to the recording start, not UI receipt.
    pub timestamp_ns: u64,
    /// Original GazeSource timestamp relative to camera-open (preserved for audit).
    pub source_timestamp_ns: u64,
    /// One-Euro-smoothed uncalibrated eye/pose values in FEATURE_NAMES order.
    pub features: Option<[f32; 10]>,
    /// Same extraction before smoothing, useful for later calibration experiments.
    pub raw_features: Option<[f32; 10]>,
    pub face_found: bool,
    pub quality: f32,
    pub cpu_ms: f64,
    pub phase: Phase,
    /// Present only for a displayed, focused hold target. Countdown is unlabelled.
    pub target: Option<Target>,
}
impl FrameRecord {
    pub fn sample(&self) -> GazeSample {
        GazeSample {
            timestamp: Duration::from_nanos(self.timestamp_ns),
            features: self.features.map(GazeFeatures::from_array),
            quality: self.quality,
        }
    }
    fn validate(&self, header: &Header) -> Result<()> {
        TraceGazeSource::new(std::iter::once(self.sample())).next_sample()?;
        if self
            .raw_features
            .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
            || self.raw_features.is_some() != self.features.is_some()
            || (!self.face_found && self.features.is_some())
            || !self.cpu_ms.is_finite()
            || self.cpu_ms < 0.0
            || self.timestamp_ns > header.script_seconds * 1_000_000_000
            || (self.phase == Phase::Hold) != self.target.is_some()
            || self.target.is_some_and(|t| {
                t.id >= (header.script_seconds / 6) as u32
                    || t.position
                        .iter()
                        .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
            })
        {
            return Err("invalid numeric trace frame or target".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Completed,
    Cancelled,
    TrackingError,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct End {
    pub frames: usize,
    pub reason: EndReason,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum Record {
    Header(Header),
    Frame(FrameRecord),
    End(End),
}

pub struct TraceWriter<W: Write> {
    output: W,
    header: Header,
    frames: usize,
    previous: Option<(u64, u64)>,
}
impl<W: Write> TraceWriter<W> {
    pub fn new(mut output: W, header: Header) -> Result<Self> {
        header.validate()?;
        write_record(&mut output, &Record::Header(header.clone()))?;
        output.flush()?;
        Ok(Self {
            output,
            header,
            frames: 0,
            previous: None,
        })
    }
    pub fn frame(&mut self, frame: FrameRecord) -> Result<()> {
        frame.validate(&self.header)?;
        check_order(self.previous, &frame)?;
        if self.frames >= MAX_FRAMES {
            return Err("trace frame limit exceeded".into());
        }
        let timestamps = (frame.timestamp_ns, frame.source_timestamp_ns);
        write_record(&mut self.output, &Record::Frame(frame))?;
        self.previous = Some(timestamps);
        self.frames += 1;
        // Bound data loss on unexpected shutdown without flushing every frame.
        if self.frames.is_multiple_of(self.header.camera.fps as usize) {
            self.output.flush()?;
        }
        Ok(())
    }
    pub fn finish(mut self, reason: EndReason) -> Result<W> {
        write_record(
            &mut self.output,
            &Record::End(End {
                frames: self.frames,
                reason,
            }),
        )?;
        self.output.flush()?;
        Ok(self.output)
    }
}
fn write_record(output: &mut impl Write, record: &Record) -> Result<()> {
    let bytes = serde_json::to_vec(record)?;
    if bytes.len() as u64 + 1 > MAX_LINE {
        return Err("trace line too long".into());
    }
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    Ok(())
}
fn check_order(previous: Option<(u64, u64)>, f: &FrameRecord) -> Result<()> {
    if previous.is_some_and(|(t, s)| f.timestamp_ns <= t || f.source_timestamp_ns <= s) {
        return Err("trace timestamps must strictly increase".into());
    }
    Ok(())
}
fn read_record(input: &mut impl BufRead) -> Result<Option<Record>> {
    let mut line = String::new();
    use std::io::Read;
    let bytes = input.take(MAX_LINE + 1).read_line(&mut line)?;
    if bytes == 0 {
        return Ok(None);
    }
    if bytes as u64 > MAX_LINE {
        return Err("trace line too long".into());
    }
    Ok(Some(serde_json::from_str(&line)?))
}
pub struct Trace {
    pub header: Header,
    pub frames: Vec<FrameRecord>,
    pub end: End,
}
pub fn read_trace(mut input: impl BufRead) -> Result<Trace> {
    let Some(Record::Header(header)) = read_record(&mut input)? else {
        return Err("trace must start with a header".into());
    };
    header.validate()?;
    let mut frames = Vec::new();
    let mut previous = None;
    loop {
        match read_record(&mut input)? {
            Some(Record::Frame(f)) => {
                f.validate(&header)?;
                check_order(previous, &f)?;
                if frames.len() >= MAX_FRAMES {
                    return Err("trace frame limit exceeded".into());
                }
                previous = Some((f.timestamp_ns, f.source_timestamp_ns));
                frames.push(f);
            }
            Some(Record::End(end)) if end.frames == frames.len() => {
                if read_record(&mut input)?.is_some() {
                    return Err("data after trace end".into());
                }
                return Ok(Trace {
                    header,
                    frames,
                    end,
                });
            }
            None => {
                return Err(
                    "incomplete trace: no end record (recording may have been interrupted)".into(),
                );
            }
            _ => return Err("unexpected trace record or frame-count mismatch".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Distribution {
    pub count: usize,
    pub mean: f64,
    pub stddev: f64,
    pub min: f64,
    pub max: f64,
    m2: f64,
}
impl Distribution {
    fn add(&mut self, value: f64) {
        if self.count == 0 {
            self.min = value;
            self.max = value;
        }
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (value - self.mean);
        self.stddev = (self.m2 / self.count as f64).sqrt();
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }
}
#[derive(Debug, Default)]
pub struct Summary {
    pub frames: usize,
    pub valid_frames: usize,
    pub fps: f64,
    pub distributions: BTreeMap<TargetKind, [Distribution; 10]>,
}
impl Summary {
    pub fn valid_share(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.valid_frames as f64 / self.frames as f64
        }
    }
    pub fn print(&self, mut output: impl Write) -> io::Result<()> {
        writeln!(
            output,
            "Frames: {}\nValid-frame share: {:.2}%\nObserved FPS: {:.2}",
            self.frames,
            self.valid_share() * 100.0,
            self.fps
        )?;
        writeln!(
            output,
            "Feature distributions: focused hold frames only; population stddev; no detection-rate evaluation."
        )?;
        for kind in [TargetKind::Minimap, TargetKind::Elsewhere] {
            writeln!(output, "{kind:?}:")?;
            if let Some(stats) = self.distributions.get(&kind) {
                for (name, stat) in FEATURE_NAMES.iter().zip(stats) {
                    writeln!(
                        output,
                        "  {name}: n={} mean={:.5} std={:.5} min={:.5} max={:.5}",
                        stat.count, stat.mean, stat.stddev, stat.min, stat.max
                    )?;
                }
            } else {
                writeln!(output, "  No valid labelled frames")?;
            }
        }
        Ok(())
    }
}
/// All samples are consumed through the M4.1 GazeSource replay boundary, including
/// tracking loss and countdown. Only labelled hold frames enter distributions.
pub fn summarize(trace: &Trace) -> Result<Summary> {
    let mut source = TraceGazeSource::new(trace.frames.iter().map(FrameRecord::sample));
    let mut result = Summary::default();
    let mut first = None;
    let mut last = Duration::ZERO;
    for frame in &trace.frames {
        let sample = source.next_sample()?.ok_or("unexpected end of replay")?;
        first.get_or_insert(sample.timestamp);
        last = sample.timestamp;
        result.frames += 1;
        if let Some(features) = sample.features {
            result.valid_frames += 1;
            if frame.phase == Phase::Hold
                && let Some(target) = frame.target
            {
                let stats = result
                    .distributions
                    .entry(target.kind)
                    .or_insert([Distribution::default(); 10]);
                for (stat, value) in stats.iter_mut().zip(features.as_array()) {
                    stat.add(value as f64);
                }
            }
        }
    }
    if let Some(first) = first
        && last > first
    {
        result.fps = (result.frames - 1) as f64 / (last - first).as_secs_f64();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header() -> Header {
        Header {
            schema_version: 1,
            app_version: "test".into(),
            started_unix_ms: 0,
            camera: CameraInfo {
                name: "synthetic".into(),
                resolution: [1280, 720],
                fps: 30,
            },
            monitor_resolution: [1920, 1080],
            dpi_scale: 1.5,
            lighting_note: "daylight".into(),
            script_seconds: 60,
            minimap: MinimapSelection {
                source: crate::settings::MinimapSource::PersistedSettings,
                minimap_scale: 3.0,
                flipped: true,
            },
        }
    }
    fn frame(n: u64, value: Option<f32>, kind: Option<TargetKind>) -> FrameRecord {
        FrameRecord {
            timestamp_ns: n * 100_000_000,
            source_timestamp_ns: n * 100_000_000,
            features: value.map(|v| [v; 10]),
            raw_features: value.map(|v| [v; 10]),
            face_found: value.is_some(),
            quality: if value.is_some() { 0.8 } else { 0.0 },
            cpu_ms: 3.0,
            phase: if kind.is_some() {
                Phase::Hold
            } else {
                Phase::Countdown
            },
            target: kind.map(|kind| Target {
                id: 0,
                position: [0.5, 0.5],
                kind,
            }),
        }
    }
    fn synthetic() -> Vec<u8> {
        let mut writer = TraceWriter::new(Vec::new(), header()).unwrap();
        for f in [
            frame(0, Some(1.0), Some(TargetKind::Minimap)),
            frame(1, Some(3.0), Some(TargetKind::Minimap)),
            frame(2, None, Some(TargetKind::Elsewhere)),
            frame(3, Some(99.0), None),
            frame(4, Some(5.0), Some(TargetKind::Elsewhere)),
        ] {
            writer.frame(f).unwrap();
        }
        writer.finish(EndReason::Completed).unwrap()
    }
    #[test]
    fn full_600_second_1080p_60fps_trace_roundtrips() {
        let mut header = header();
        header.camera.resolution = [1920, 1080];
        header.camera.fps = 60;
        header.script_seconds = 600;
        let mut writer = TraceWriter::new(Vec::new(), header.clone()).unwrap();
        for n in 0..36_000 {
            let mut f = frame(0, None, None);
            f.timestamp_ns = n * 1_000_000_000 / 60;
            f.source_timestamp_ns = f.timestamp_ns + 123_000_000;
            writer.frame(f).unwrap();
        }
        let bytes = writer.finish(EndReason::Completed).unwrap();
        let trace = read_trace(bytes.as_slice()).unwrap();
        assert_eq!(trace.header, header);
        assert_eq!(trace.end.frames, 36_000);
        assert!((summarize(&trace).unwrap().fps - 60.0).abs() < 0.001);
    }
    #[test]
    fn roundtrip_and_replay_summary() {
        let bytes = synthetic();
        let trace = read_trace(bytes.as_slice()).unwrap();
        assert_eq!(trace.header, header());
        assert_eq!(
            trace.frames[1],
            frame(1, Some(3.0), Some(TargetKind::Minimap))
        );
        assert_eq!(trace.end.reason, EndReason::Completed);
        let summary = summarize(&trace).unwrap();
        assert_eq!(summary.frames, 5);
        assert_eq!(summary.valid_share(), 0.8);
        assert_eq!(summary.fps, 10.0);
        let stat = summary.distributions[&TargetKind::Minimap][0];
        assert_eq!(
            (stat.count, stat.mean, stat.stddev, stat.min, stat.max),
            (2, 2.0, 1.0, 1.0, 3.0)
        );
        assert_eq!(summary.distributions[&TargetKind::Elsewhere][0].mean, 5.0);
        let mut again = TraceWriter::new(Vec::new(), trace.header).unwrap();
        for frame in trace.frames {
            again.frame(frame).unwrap();
        }
        assert_eq!(again.finish(trace.end.reason).unwrap(), bytes);
    }
    #[test]
    fn rejects_malformed_truncated_nonfinite_and_extra_data() {
        assert!(read_trace(b"{}\n".as_slice()).is_err());
        let bytes = synthetic();
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            read_trace(
                text.replace("\"schema_version\":1", "\"schema_version\":2")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            read_trace(
                text.replace("\"quality\":0.8", "\"quality\":2.0")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            read_trace(
                text.replace("\"cpu_ms\":3.0", "\"pixels\":[1,2,3],\"cpu_ms\":3.0")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            read_trace(
                text.lines()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(read_trace(format!("{text}{{}}\n").as_bytes()).is_err());
        assert!(read_trace(vec![b'x'; MAX_LINE as usize + 1].as_slice()).is_err());
        let mut writer = TraceWriter::new(Vec::new(), header()).unwrap();
        assert!(writer.frame(frame(0, Some(f32::NAN), None)).is_err());
        writer.frame(frame(1, None, None)).unwrap();
        assert!(writer.frame(frame(1, None, None)).is_err());
    }
    #[test]
    fn empty_and_single_frame_summaries_are_finite() {
        for count in 0..=1 {
            let mut writer = TraceWriter::new(Vec::new(), header()).unwrap();
            if count == 1 {
                writer.frame(frame(0, None, None)).unwrap();
            }
            let bytes = writer.finish(EndReason::Cancelled).unwrap();
            let s = summarize(&read_trace(bytes.as_slice()).unwrap()).unwrap();
            assert_eq!(s.fps, 0.0);
            assert_eq!(s.valid_share(), 0.0);
        }
    }
}

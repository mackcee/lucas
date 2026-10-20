//! Supervised, local CUDA child. Only one request can be in flight. A bounded
//! reader owns the pipes; timeout/drop kills and reaps the process before joining.
use crate::{Error, Frame, Result};
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const START_TIMEOUT: Duration = Duration::from_secs(120);
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REPLY: u64 = 16 * 1024;

fn failure(message: impl Into<String>) -> Error {
    Error::Source(message.into())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub timestamp_ns: u64,
    pub features: Option<[f32; 10]>,
    pub quality: f32,
    pub face_found: bool,
    pub luminance: f32,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCheck {
    pub provider: String,
    pub gpu: String,
    pub inferences: u32,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub model_median_ms: f64,
    pub model_p95_ms: f64,
    pub allocated_mib: f64,
    pub reserved_mib: f64,
    pub peak_mib: f64,
}
impl std::fmt::Display for ModelCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Provider: {}\nGPU: {}\nInferences: {} (3 warmups excluded)\nPipeline median/p95: {:.2}/{:.2} ms\nUniGaze median/p95: {:.2}/{:.2} ms\nCUDA memory allocated/reserved/peak: {:.1}/{:.1}/{:.1} MiB\nNo camera opened. This is not a G2 accuracy test.",
            self.provider,
            self.gpu,
            self.inferences,
            self.median_ms,
            self.p95_ms,
            self.model_median_ms,
            self.model_p95_ms,
            self.allocated_mib,
            self.reserved_mib,
            self.peak_mib
        )
    }
}
enum Request {
    Frame(Frame),
    Check,
}
type Reply = Result<(Option<Frame>, serde_json::Value)>;
pub struct UniGaze {
    child: Child,
    input: Option<SyncSender<Request>>,
    output: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
    pub description: String,
}
fn read_reply(reader: &mut impl BufRead) -> Result<serde_json::Value> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_REPLY + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| failure("UniGaze pipe read failed"))?;
    if bytes.len() as u64 > MAX_REPLY || bytes.last() != Some(&b'\n') {
        return Err(failure(
            "UniGaze exited or sent an incomplete/oversized reply",
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| failure("UniGaze sent invalid JSON"))?;
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        return Err(failure(format!("UniGaze: {error}")));
    }
    Ok(value)
}
impl UniGaze {
    pub fn installation(models: &Path) -> PathBuf {
        models.join("unigaze")
    }
    pub fn installed(models: &Path) -> bool {
        Self::installation(models).join("READY").is_file()
    }
    pub fn start(models: &Path) -> Result<Self> {
        Self::start_cancellable(models, None)
    }
    pub fn start_cancellable(models: &Path, cancel: Option<&AtomicBool>) -> Result<Self> {
        let root = Self::installation(models);
        #[cfg(windows)]
        let python = root.join(".venv/Scripts/python.exe");
        #[cfg(not(windows))]
        let python = root.join(".venv/bin/python");
        if !Self::installed(models) || !python.is_file() {
            return Err(failure(
                "UniGaze is not installed. Run models/unigaze/setup.ps1, or select LUCAS_GAZE_SOURCE=mediapipe.",
            ));
        }
        let mut command = Command::new(python);
        command
            .arg("-I")
            .arg("-u")
            .arg(root.join("worker.py"))
            .arg("--root")
            .arg(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000 | 0x00004000); // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
        }
        Self::spawn(command, cancel)
    }
    fn spawn(mut command: Command, cancel: Option<&AtomicBool>) -> Result<Self> {
        let mut child = command
            .spawn()
            .map_err(|e| failure(format!("Cannot start UniGaze: {e}; rerun setup.ps1")))?;
        let mut input = child.stdin.take().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let (tx, requests) = mpsc::sync_channel(1);
        let (replies, rx) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let ready = read_reply(&mut reader).map(|v| (None, v));
            let ok = ready.is_ok();
            if replies.send(ready).is_err() || !ok {
                return;
            }
            while let Ok(request) = requests.recv() {
                let mut frame = match request {
                    Request::Frame(f) => Some(f),
                    Request::Check => None,
                };
                let result = (|| {
                    input.write_all(b"LUG1")?;
                    let (w, h, at) = frame.as_ref().map_or((0, 0, 0), |f| {
                        (f.width(), f.height(), f.timestamp().as_nanos() as u64)
                    });
                    input.write_all(&w.to_le_bytes())?;
                    input.write_all(&h.to_le_bytes())?;
                    input.write_all(&at.to_le_bytes())?;
                    if let Some(f) = &mut frame {
                        let result = input.write_all(f.rgb());
                        f.clear();
                        result?;
                    }
                    input.flush()
                })()
                .map_err(|_: std::io::Error| failure("UniGaze pipe write failed; child stopped"))
                .and_then(|_| read_reply(&mut reader));
                let ok = result.is_ok();
                if replies.send(result.map(|value| (frame, value))).is_err() || !ok {
                    break;
                }
            }
        });
        let mut source = Self {
            child,
            input: Some(tx),
            output: rx,
            thread: Some(thread),
            description: String::new(),
        };
        let (_, ready) = source.receive_cancellable(START_TIMEOUT, cancel)?;
        if ready.get("ready").and_then(|v| v.as_bool()) != Some(true)
            || !ready
                .get("provider")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("CUDA cuda:0"))
        {
            return Err(failure(
                "UniGaze did not confirm CUDA execution; refusing CPU fallback",
            ));
        }
        source.description = format!(
            "{} · {}",
            ready["provider"].as_str().unwrap(),
            ready["gpu"].as_str().unwrap_or("unknown GPU")
        );
        Ok(source)
    }
    fn stop(&mut self) {
        // kill before joining: the pipe worker could be blocked on write or read.
        let _ = self.child.kill();
        self.input.take();
        let _ = self.child.wait();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
    fn receive(&mut self, timeout: Duration) -> Reply {
        self.receive_cancellable(timeout, None)
    }
    fn receive_cancellable(&mut self, timeout: Duration, cancel: Option<&AtomicBool>) -> Reply {
        let started = Instant::now();
        loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                self.stop();
                return Err(failure("UniGaze startup cancelled; process stopped"));
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.stop();
                return Err(failure(
                    "UniGaze response timed out; process stopped. Check CUDA with --check-gaze-model.",
                ));
            }
            match self
                .output
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop();
                    return Err(failure(
                        "UniGaze process disconnected. Check CUDA with --check-gaze-model.",
                    ));
                }
            }
        }
    }
    pub fn infer(&mut self, frame: Frame) -> Result<(Frame, Observation)> {
        let at =
            u64::try_from(frame.timestamp().as_nanos()).map_err(|_| Error::InvalidTimestamp)?;
        self.input
            .as_ref()
            .ok_or(Error::InferenceFailed)?
            .send(Request::Frame(frame))
            .map_err(|_| failure("UniGaze process stopped"))?;
        let (frame, value) = self.receive(FRAME_TIMEOUT)?;
        let observation: Observation =
            serde_json::from_value(value).map_err(|_| Error::InvalidOutput)?;
        if observation.timestamp_ns != at
            || !(0.0..=1.0).contains(&observation.quality)
            || !(0.0..=1.0).contains(&observation.luminance)
            || (observation.features.is_none() && observation.quality != 0.0)
            || (!observation.face_found && observation.features.is_some())
            || observation
                .features
                .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
        {
            return Err(Error::InvalidOutput);
        }
        Ok((frame.ok_or(Error::InvalidOutput)?, observation))
    }
    pub fn check(&mut self) -> Result<ModelCheck> {
        self.input
            .as_ref()
            .ok_or(Error::InferenceFailed)?
            .send(Request::Check)
            .map_err(|_| Error::InferenceFailed)?;
        let (_, value) = self.receive(CHECK_TIMEOUT)?;
        let check: ModelCheck = serde_json::from_value(value).map_err(|_| Error::InvalidOutput)?;
        if check.inferences != 20
            || !check.provider.contains("CUDA (cuda:0)")
            || [
                check.median_ms,
                check.p95_ms,
                check.model_median_ms,
                check.model_p95_ms,
                check.allocated_mib,
                check.reserved_mib,
                check.peak_mib,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err(Error::InvalidOutput);
        }
        Ok(check)
    }
}
impl Drop for UniGaze {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_rejects_bad_oversized_and_error_replies() {
        for data in [
            b"{}".to_vec(),
            b"not-json\n".to_vec(),
            vec![b'x'; MAX_REPLY as usize + 1],
            b"{\"error\":\"CUDA unavailable\"}\n".to_vec(),
        ] {
            assert!(read_reply(&mut data.as_slice()).is_err());
        }
        assert_eq!(
            read_reply(&mut b"{\"ready\":true}\n".as_slice()).unwrap()["ready"],
            true
        );
    }
    #[cfg(unix)]
    #[test]
    fn real_pipes_preserve_timestamps_unknowns_and_scrub_frames() {
        for mode in ["known", "unknown", "wrong_timestamp", "crash"] {
            let mut command = Command::new("python3");
            command
                .arg("-I")
                .arg("-u")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/unigaze_stub.py"
                ))
                .arg(mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut source = UniGaze::spawn(command, None).unwrap();
            let mut frame = Frame::new(1280, 720).unwrap();
            frame.set_timestamp(Duration::from_nanos(123_456_789));
            frame.rgb_mut().fill(91);
            let result = source.infer(frame);
            if matches!(mode, "wrong_timestamp" | "crash") {
                assert!(result.is_err());
            } else {
                let (frame, observed) = result.unwrap();
                assert_eq!(observed.timestamp_ns, 123_456_789);
                assert!(frame.rgb().iter().all(|v| *v == 0));
                assert_eq!(observed.features.is_none(), mode == "unknown");
                assert_eq!(observed.quality, if mode == "unknown" { 0.0 } else { 0.9 });
            }
            source.stop();
            assert!(source.child.try_wait().unwrap().is_some());
        }
    }
    #[cfg(unix)]
    #[test]
    fn cancellation_interrupts_startup_without_waiting_for_a_ready_message() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "read value"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let cancelled = AtomicBool::new(true);
        match UniGaze::spawn(command, Some(&cancelled)) {
            Ok(_) => panic!("cancelled startup must fail"),
            Err(error) => assert!(error.to_string().contains("startup cancelled")),
        }
    }
    #[cfg(unix)]
    #[test]
    fn process_must_confirm_cuda_and_is_reaped_on_drop() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "printf '%s\\n' '{\"ready\":true,\"provider\":\"CPU\"}'; read value",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        assert!(UniGaze::spawn(command, None).is_err());
        let mut command = Command::new("sh");
        command.args(["-c", "printf '%s\\n' '{\"ready\":true,\"provider\":\"CUDA cuda:0\",\"gpu\":\"test\"}'; read value"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut source = UniGaze::spawn(command, None).unwrap();
        assert!(source.receive(Duration::from_millis(10)).is_err());
        assert!(source.child.try_wait().unwrap().is_some());
    }
}

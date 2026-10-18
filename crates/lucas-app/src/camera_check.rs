//! Headless camera smoke check. Only format and aggregate timing leave RAM.

#[cfg(windows)]
pub fn run() -> Result<(), String> {
    // Keep camera creation, capture and drop on one dedicated thread, like the app.
    std::thread::spawn(|| {
        use lucas_gaze::camera::{Camera, NokhwaCamera};
        use std::time::Duration;

        let mut camera =
            NokhwaCamera::open_c270(0).map_err(|error| format!("camera open: {error}"))?;
        let format = camera.format();
        let (width, height) = camera.dimensions();
        println!(
            "Negotiated: {width}x{height} {:?} {} fps (camera index 0)",
            format.format(),
            format.frame_rate()
        );
        let origin = camera.timestamp_origin();
        let report = capture::collect(&mut camera, origin, Duration::from_secs(3))?;
        println!("{report}");
        Ok(())
    })
    .join()
    .map_err(|_| "camera check thread panicked".to_owned())?
}

#[cfg(not(windows))]
pub fn run() -> Result<(), String> {
    Err("--check-camera requires Windows 10/11".into())
}

#[cfg(any(windows, test))]
mod capture {
    use lucas_gaze::camera::{Camera, Frame};
    use std::time::{Duration, Instant};

    pub(super) struct Report {
        frames: usize,
        elapsed: Duration,
        min: Duration,
        median: Duration,
        max: Duration,
    }

    impl Report {
        fn new(mut ages: Vec<Duration>, elapsed: Duration) -> Result<Self, String> {
            if ages.is_empty() || elapsed.is_zero() {
                return Err("camera check received no frames or no elapsed capture time".into());
            }
            ages.sort_unstable();
            let frames = ages.len();
            let middle = frames / 2;
            let median = if frames.is_multiple_of(2) {
                ages[middle - 1] + (ages[middle] - ages[middle - 1]) / 2
            } else {
                ages[middle]
            };
            Ok(Self {
                frames,
                elapsed,
                min: ages[0],
                median,
                max: ages[frames - 1],
            })
        }

        fn effective_fps(&self) -> f64 {
            self.frames as f64 / self.elapsed.as_secs_f64()
        }
    }

    impl std::fmt::Display for Report {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "Frames received: {} in {:.3} s\nEffective fps: {:.2}\nAcquisition age ms (min/median/max): {:.3}/{:.3}/{:.3}",
                self.frames,
                self.elapsed.as_secs_f64(),
                self.effective_fps(),
                self.min.as_secs_f64() * 1000.0,
                self.median.as_secs_f64() * 1000.0,
                self.max.as_secs_f64() * 1000.0,
            )
        }
    }

    pub(super) fn collect(
        camera: &mut impl Camera,
        origin: Instant,
        duration: Duration,
    ) -> Result<Report, String> {
        let (width, height) = camera.dimensions();
        let mut frame = Frame::new(width, height).map_err(|error| error.to_string())?;
        let mut ages = Vec::new();
        let started = Instant::now();
        loop {
            let received = camera
                .capture(&mut frame)
                .map_err(|error| format!("camera capture: {error}"))?;
            let age = origin.elapsed().checked_sub(frame.timestamp());
            frame.clear();
            if !received {
                return Err("camera capture: stream ended before the check completed".into());
            }
            ages.push(age.ok_or("camera capture: acquisition timestamp is in the future")?);
            if started.elapsed() >= duration {
                break;
            }
        }
        Report::new(ages, started.elapsed())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn report_uses_received_frames_and_sorted_acquisition_ages() {
            let report = Report::new(
                [30, 10, 20].map(Duration::from_millis).to_vec(),
                Duration::from_secs(3),
            )
            .unwrap();
            assert_eq!(report.frames, 3);
            assert_eq!(report.effective_fps(), 1.0);
            assert_eq!(report.min, Duration::from_millis(10));
            assert_eq!(report.median, Duration::from_millis(20));
            assert_eq!(report.max, Duration::from_millis(30));
            assert_eq!(
                report.to_string(),
                "Frames received: 3 in 3.000 s\nEffective fps: 1.00\nAcquisition age ms (min/median/max): 10.000/20.000/30.000"
            );
        }

        #[test]
        fn even_median_and_empty_capture() {
            let report = Report::new(
                [20, 10].map(Duration::from_millis).to_vec(),
                Duration::from_secs(1),
            )
            .unwrap();
            assert_eq!(report.median, Duration::from_millis(15));
            assert!(Report::new(vec![], Duration::from_secs(3)).is_err());
        }

        struct FakeCamera {
            origin: Instant,
            error: Option<lucas_gaze::Error>,
            ended: bool,
            future: bool,
            captures: usize,
        }
        impl Camera for FakeCamera {
            fn dimensions(&self) -> (u32, u32) {
                (1, 1)
            }
            fn capture(&mut self, frame: &mut Frame) -> lucas_gaze::Result<bool> {
                assert!(frame.rgb().iter().all(|&byte| byte == 0));
                if let Some(error) = self.error.take() {
                    return Err(error);
                }
                frame.rgb_mut().fill(123);
                self.captures += 1;
                frame.set_timestamp(if self.future {
                    self.origin.elapsed() + Duration::from_secs(60)
                } else {
                    self.origin.elapsed()
                });
                // Make the short test collection finish after one capture.
                std::thread::sleep(Duration::from_millis(2));
                Ok(!self.ended)
            }
        }
        fn fake() -> FakeCamera {
            FakeCamera {
                origin: Instant::now(),
                error: None,
                ended: false,
                future: false,
                captures: 0,
            }
        }

        #[test]
        fn capture_path_collects_only_numeric_timings() {
            let mut camera = fake();
            let origin = camera.origin;
            let report = collect(&mut camera, origin, Duration::from_millis(1)).unwrap();
            assert_eq!(report.frames, camera.captures);
            assert!(report.frames > 0);
        }

        #[test]
        fn capture_errors_and_early_end_are_not_success() {
            for (error, ended, future, expected) in [
                (
                    Some(lucas_gaze::Error::CameraTimestamp),
                    false,
                    false,
                    "no usable acquisition timestamps",
                ),
                (
                    Some(lucas_gaze::Error::CaptureFailed),
                    false,
                    false,
                    "camera frame capture failed",
                ),
                (None, true, false, "stream ended"),
                (None, false, true, "timestamp is in the future"),
            ] {
                let mut camera = fake();
                camera.error = error;
                camera.ended = ended;
                camera.future = future;
                let origin = camera.origin;
                let error = collect(&mut camera, origin, Duration::from_millis(1))
                    .err()
                    .unwrap();
                assert!(error.contains(expected), "{error}");
            }
        }
    }
}

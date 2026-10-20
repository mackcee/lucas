//! Synchronous capture into three reusable RGB buffers; there is no frame queue.
//! The file source only READS pre-existing test images, never writes webcam data.

use crate::{Error, Result};
use std::{path::PathBuf, time::Duration};
use zeroize::Zeroize;

#[cfg(any(test, all(windows, feature = "camera")))]
mod timestamp;

const MAX_PIXELS: u32 = 1920 * 1080;
pub const RING_CAPACITY: usize = 3;

/// Numeric MJPEG mode, safe to expose without any camera pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureMode {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}
impl CaptureMode {
    pub const FALLBACK: Self = Self {
        width: 1280,
        height: 720,
        fps: 30,
    };
}

/// Only reported MJPEG modes are supplied here. Auto upgrades to 60 fps,
/// choosing the largest bounded image at that rate; 30 fps retains the C270 mode.
#[cfg(any(test, all(windows, feature = "camera")))]
fn preferred_mode(modes: impl Iterator<Item = CaptureMode>, prefer_60: bool) -> CaptureMode {
    modes
        .filter(|m| prefer_60 && m.fps == 60 && m.width > 0 && m.height > 0)
        .filter(|m| {
            m.width
                .checked_mul(m.height)
                .is_some_and(|p| p <= MAX_PIXELS)
        })
        .max_by_key(|m| (m.width * m.height, m.width))
        .unwrap_or(CaptureMode::FALLBACK)
}

#[cfg(any(test, all(windows, feature = "camera")))]
fn with_fallback<T>(
    mode: CaptureMode,
    mut open: impl FnMut(CaptureMode) -> Result<T>,
) -> Result<T> {
    match open(mode) {
        Ok(camera) => Ok(camera),
        Err(_) if mode != CaptureMode::FALLBACK => open(CaptureMode::FALLBACK),
        Err(error) => Err(error),
    }
}

/// Owned RGB8 storage. Intentionally not Clone, Serialize, or pixel-bearing Debug.
/// Drop scrubs our allocation; OS/driver buffers and crash dumps are outside this
/// guarantee. Applications must not enable process dumps containing camera RAM.
pub struct Frame {
    width: u32,
    height: u32,
    timestamp: Duration,
    rgb: Vec<u8>,
}

impl Frame {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let pixels = width.checked_mul(height).ok_or(Error::InvalidFrame)?;
        if pixels == 0 || pixels > MAX_PIXELS {
            return Err(Error::InvalidFrame);
        }
        Ok(Self {
            width,
            height,
            timestamp: Duration::ZERO,
            rgb: vec![0; pixels as usize * 3],
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn timestamp(&self) -> Duration {
        self.timestamp
    }
    pub fn set_timestamp(&mut self, timestamp: Duration) {
        self.timestamp = timestamp;
    }
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }
    pub fn rgb_mut(&mut self) -> &mut [u8] {
        &mut self.rgb
    }
    pub fn clear(&mut self) {
        self.rgb.as_mut_slice().zeroize();
    }
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("timestamp", &self.timestamp)
            .field("rgb", &"[REDACTED]")
            .finish()
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        self.clear();
    }
}

pub struct FrameRing {
    frames: [Frame; RING_CAPACITY],
    next: usize,
}
impl FrameRing {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            frames: [
                Frame::new(width, height)?,
                Frame::new(width, height)?,
                Frame::new(width, height)?,
            ],
            next: 0,
        })
    }
    pub fn next_frame(&mut self) -> &mut Frame {
        let slot = self.next;
        self.next = (self.next + 1) % RING_CAPACITY;
        &mut self.frames[slot]
    }
}

pub trait Camera {
    fn dimensions(&self) -> (u32, u32);
    /// Fill the caller's buffer. `false` is end-of-stream, NOT tracking loss.
    /// Implementations must not enqueue, persist, or log frames.
    fn capture(&mut self, frame: &mut Frame) -> Result<bool>;
}

/// Finite still-image sequence for deterministic tests. A path and timestamp
/// iterator avoids retaining decoded images in RAM. Images must match dimensions.
pub struct ImageCamera<I> {
    frames: I,
    dimensions: (u32, u32),
}
impl<I: Iterator<Item = (Duration, PathBuf)>> ImageCamera<I> {
    pub fn new(frames: I, dimensions: (u32, u32)) -> Result<Self> {
        Frame::new(dimensions.0, dimensions.1)?;
        Ok(Self { frames, dimensions })
    }
}
impl<I: Iterator<Item = (Duration, PathBuf)>> Camera for ImageCamera<I> {
    fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }
    fn capture(&mut self, frame: &mut Frame) -> Result<bool> {
        frame.clear();
        if (frame.width, frame.height) != self.dimensions {
            return Err(Error::InvalidFrame);
        }
        let Some((timestamp, path)) = self.frames.next() else {
            return Ok(false);
        };
        let mut reader = image::ImageReader::open(path).map_err(|_| Error::ImageReadFailed)?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(frame.width);
        limits.max_image_height = Some(frame.height);
        limits.max_alloc = Some(u64::from(MAX_PIXELS) * 16);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| Error::ImageReadFailed)?
            .into_rgb8();
        if image.dimensions() != self.dimensions {
            return Err(Error::InvalidFrame);
        }
        let mut pixels = image.into_raw();
        frame.rgb.copy_from_slice(&pixels);
        pixels.zeroize();
        frame.timestamp = timestamp;
        Ok(true)
    }
}

#[cfg(any(test, all(windows, feature = "camera")))]
fn decode_mjpeg(data: &[u8], timestamp: Duration, frame: &mut Frame) -> Result<()> {
    use image::ImageDecoder;
    frame.clear();
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(data))
        .map_err(|_| Error::CaptureFailed)?;
    if decoder.dimensions() != (frame.width, frame.height)
        || decoder.color_type() != image::ColorType::Rgb8
    {
        return Err(Error::InvalidFrame);
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(u64::from(MAX_PIXELS) * 16);
    decoder
        .set_limits(limits)
        .map_err(|_| Error::CaptureFailed)?;
    if decoder.read_image(&mut frame.rgb).is_err() {
        frame.clear();
        return Err(Error::CaptureFailed);
    }
    frame.timestamp = timestamp;
    Ok(())
}

#[cfg(all(windows, feature = "camera"))]
mod windows {
    use super::*;
    use nokhwa::utils::{
        CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
    };
    use nokhwa_bindings_windows::wmf::MediaFoundationDevice;
    use std::time::Instant;
    use timestamp::CaptureClock;

    /// Windows Media Foundation, synchronous mode (not nokhwa's threaded callback).
    /// C270 HD default is 1280x720 MJPEG at 30fps; software decoding, no mozjpeg.
    pub struct NokhwaCamera {
        camera: MediaFoundationDevice,
        origin: Instant,
        clock: CaptureClock,
        dimensions: (u32, u32),
    }
    impl NokhwaCamera {
        /// Call on the dedicated capture/inference thread. Drops release the camera.
        pub fn open_c270(index: u32) -> Result<Self> {
            Self::open(index, 1280, 720, 30)
        }
        /// Auto prefers a reported 60 fps MJPEG mode. Each negotiation attempt
        /// owns a fresh device so a rejected mode cannot poison the 30 fps retry.
        pub fn open_preferred(index: u32, prefer_60: bool) -> Result<Self> {
            lower_thread_priority()?;
            let mode = if prefer_60 {
                let mut camera = MediaFoundationDevice::new(CameraIndex::Index(index))
                    .map_err(|_| Error::CameraUnavailable)?;
                let available = camera.compatible_format_list().unwrap_or_default();
                preferred_mode(
                    available
                        .iter()
                        .filter(|f| f.format() == FrameFormat::MJPEG)
                        .map(|f| CaptureMode {
                            width: f.width(),
                            height: f.height(),
                            fps: f.frame_rate(),
                        }),
                    true,
                )
            } else {
                CaptureMode::FALLBACK
            };
            with_fallback(mode, |m| Self::open(index, m.width, m.height, m.fps))
        }
        pub fn mode(&self) -> CaptureMode {
            let format = self.format();
            CaptureMode {
                width: format.width(),
                height: format.height(),
                fps: format.frame_rate(),
            }
        }
        /// Monotonic origin of acquisition timestamps, shared with displayed
        /// targets. Neither inference delays nor wall-clock changes move it.
        pub fn timestamp_origin(&self) -> Instant {
            self.origin
        }
        /// Negotiated media type, refreshed and checked exactly during open.
        pub fn format(&self) -> CameraFormat {
            self.camera.format()
        }
        pub fn open(index: u32, width: u32, height: u32, fps: u32) -> Result<Self> {
            Frame::new(width, height)?;
            if fps == 0 {
                return Err(Error::InvalidFrame);
            }
            lower_thread_priority()?;
            let format = CameraFormat::new_from(width, height, FrameFormat::MJPEG, fps);
            let request = RequestedFormat::with_formats(
                RequestedFormatType::Exact(format),
                &[FrameFormat::MJPEG],
            );
            let mut camera = MediaFoundationDevice::new(CameraIndex::Index(index))
                .map_err(|_| Error::CameraUnavailable)?;
            let available = camera
                .compatible_format_list()
                .map_err(|_| Error::CameraUnavailable)?;
            let desired = request
                .fulfill(&available)
                .ok_or(Error::CameraUnavailable)?;
            camera
                .set_format(desired)
                .map_err(|_| Error::CameraUnavailable)?;
            if camera
                .format_refreshed()
                .map_err(|_| Error::CameraUnavailable)?
                != format
            {
                return Err(Error::CameraUnavailable);
            }
            let origin = Instant::now();
            camera
                .start_stream()
                .map_err(|_| Error::CameraUnavailable)?;
            let clock = CaptureClock::new();
            Ok(Self {
                camera,
                origin,
                clock,
                dimensions: (width, height),
            })
        }
    }
    impl Camera for NokhwaCamera {
        fn dimensions(&self) -> (u32, u32) {
            self.dimensions
        }
        fn capture(&mut self, frame: &mut Frame) -> Result<bool> {
            frame.clear();
            if (frame.width, frame.height) != self.dimensions {
                return Err(Error::InvalidFrame);
            }
            loop {
                let (compressed, raw) = self
                    .camera
                    .raw_sample_bytes()
                    .map_err(|_| Error::CaptureFailed)?;
                let before = self.origin.elapsed();
                let system_now = mf_system_time().ok();
                let received = self.origin.elapsed();
                let raw = raw.filter(|_| {
                    system_now.is_some() && received - before <= Duration::from_millis(1)
                });
                let Some(timestamp) =
                    self.clock
                        .map(raw, received, system_now.unwrap_or_default())?
                else {
                    continue;
                };
                decode_mjpeg(&compressed, timestamp, frame)?;
                return Ok(true);
            }
        }
    }
    #[allow(unsafe_code)]
    fn mf_system_time() -> Result<Duration> {
        // SAFETY: MFGetSystemTime takes no pointers or handles and has no
        // caller preconditions; it reads the system's QPC-based MF clock.
        let ticks = unsafe { ::windows::Win32::Media::MediaFoundation::MFGetSystemTime() };
        let nanos = u64::try_from(ticks)
            .ok()
            .and_then(|ticks| ticks.checked_mul(100))
            .ok_or(Error::CameraTimestamp)?;
        Ok(Duration::from_nanos(nanos))
    }
    fn lower_thread_priority() -> Result<()> {
        thread_priority::set_winapi_thread_priority(
            thread_priority::thread_native_id(),
            thread_priority::WinAPIThreadPriority::BelowNormal,
        )
        .map_err(|_| Error::ThreadPriority)
    }
}
#[cfg(all(windows, feature = "camera"))]
pub use windows::NokhwaCamera;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_prefers_largest_reported_60fps_within_pixel_budget() {
        let modes = [
            CaptureMode {
                width: 3840,
                height: 2160,
                fps: 60,
            },
            CaptureMode {
                width: 1280,
                height: 720,
                fps: 60,
            },
            CaptureMode {
                width: 1920,
                height: 1080,
                fps: 30,
            },
            CaptureMode {
                width: 1920,
                height: 1080,
                fps: 60,
            },
            CaptureMode {
                width: 1920,
                height: 1080,
                fps: 120,
            },
            CaptureMode {
                width: u32::MAX,
                height: 2,
                fps: 60,
            },
            CaptureMode {
                width: 0,
                height: 1080,
                fps: 60,
            },
        ];
        assert_eq!(preferred_mode(modes.into_iter(), true), modes[3]);
        assert_eq!(
            preferred_mode(modes.into_iter(), false),
            CaptureMode::FALLBACK
        );
        assert_eq!(preferred_mode([modes[1]].into_iter(), true), modes[1]);
        assert_eq!(
            preferred_mode([CaptureMode::FALLBACK, modes[2]].into_iter(), true),
            CaptureMode::FALLBACK
        );
        assert_eq!(preferred_mode([].into_iter(), true), CaptureMode::FALLBACK);
    }
    #[test]
    fn rejected_upgrade_retries_legacy_mode_and_propagates_failure() {
        let preferred = CaptureMode {
            width: 1920,
            height: 1080,
            fps: 60,
        };
        let mut attempts = Vec::new();
        let result = with_fallback(preferred, |mode| {
            attempts.push(mode);
            if mode == preferred {
                Err(Error::CameraUnavailable)
            } else {
                Ok(mode)
            }
        });
        assert_eq!(result, Ok(CaptureMode::FALLBACK));
        assert_eq!(attempts, [preferred, CaptureMode::FALLBACK]);
        assert_eq!(
            with_fallback(preferred, |_| Err::<(), _>(Error::CameraUnavailable)),
            Err(Error::CameraUnavailable)
        );
        let mut count = 0;
        let _ = with_fallback(CaptureMode::FALLBACK, |_| {
            count += 1;
            Err::<(), _>(Error::CameraUnavailable)
        });
        assert_eq!(count, 1);
    }
    #[test]
    fn fixed_ring_reuses_and_scrubs_slots() {
        let mut ring = FrameRing::new(2, 2).unwrap();
        let pointer = ring.next_frame().rgb.as_ptr();
        ring.next_frame();
        ring.next_frame();
        let frame = ring.next_frame();
        assert_eq!(frame.rgb.as_ptr(), pointer);
        frame.rgb.fill(213);
        assert!(!format!("{frame:?}").contains("213"));
        frame.clear();
        assert!(frame.rgb.iter().all(|x| *x == 0));
    }
    #[test]
    fn jpeg_decoding_is_bounded_and_scrubs_failures() {
        let jpeg = include_bytes!("../tests/fixtures/portrait.jpg");
        let (w, h) = image::load_from_memory(jpeg)
            .unwrap()
            .to_rgb8()
            .dimensions();
        let mut frame = Frame::new(w, h).unwrap();
        decode_mjpeg(jpeg, Duration::ZERO, &mut frame).unwrap();
        assert!(frame.rgb().iter().any(|p| *p > 0));
        assert_eq!(
            decode_mjpeg(b"invalid jpeg", Duration::ZERO, &mut frame,),
            Err(Error::CaptureFailed)
        );
        assert!(frame.rgb().iter().all(|p| *p == 0));
        assert_eq!(
            decode_mjpeg(jpeg, Duration::ZERO, &mut Frame::new(2, 2).unwrap(),),
            Err(Error::InvalidFrame)
        );
    }
    #[test]
    fn delayed_decoding_preserves_acquisition_timing_and_gaps() {
        let jpeg = include_bytes!("../tests/fixtures/portrait.jpg");
        let (w, h) = image::load_from_memory(jpeg)
            .unwrap()
            .to_rgb8()
            .dimensions();
        let mut frame = Frame::new(w, h).unwrap();
        let acquired = Duration::from_millis(33);
        std::thread::sleep(Duration::from_millis(300));
        decode_mjpeg(jpeg, acquired, &mut frame).unwrap();
        assert_eq!(frame.timestamp(), Duration::from_millis(33));
        let first = frame.timestamp();
        decode_mjpeg(jpeg, Duration::from_millis(66), &mut frame).unwrap();
        assert_eq!(frame.timestamp() - first, Duration::from_millis(33));
        let second = frame.timestamp();
        decode_mjpeg(jpeg, Duration::from_millis(400), &mut frame).unwrap();
        assert_eq!(frame.timestamp() - second, Duration::from_millis(334));
    }
    #[test]
    fn rejects_oversized_and_empty_frames() {
        for (w, h) in [(0, 10), (u32::MAX, 2), (1921, 1080)] {
            assert!(Frame::new(w, h).is_err());
        }
    }
}

//! Capture independently of inference into a single replaceable slot. Slow
//! inference drops frames, never changes their original acquisition timestamps.
use crate::{Camera, Error, Frame, Result};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};
#[derive(Default)]
struct Slot {
    frame: Option<Frame>,
    ended: bool,
    error: Option<Error>,
}
pub struct LatestCamera {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    dimensions: (u32, u32),
    origin: Instant,
}
impl LatestCamera {
    /// Factory runs on the capture thread, including thread-affine camera open.
    pub fn start<C: Camera + 'static>(
        open: impl FnOnce() -> Result<(C, Instant)> + Send + 'static,
    ) -> Result<Self> {
        let slot = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (state, signal) = (slot.clone(), stop.clone());
        let (tx, rx) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let run = || -> Result<()> {
                let (mut camera, origin) = match open() {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = tx.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                let dimensions = camera.dimensions();
                let _ = tx.send(Ok((dimensions, origin)));
                let mut frame = Frame::new(dimensions.0, dimensions.1)?;
                while !signal.load(Ordering::Acquire) {
                    if !camera.capture(&mut frame)? {
                        break;
                    }
                    let mut slot = state.0.lock().unwrap();
                    // Reuse the replaced buffer when inference has not taken it.
                    frame = match slot.frame.replace(frame) {
                        Some(reusable) => reusable,
                        None => Frame::new(dimensions.0, dimensions.1)?,
                    };
                    frame.clear();
                    state.1.notify_one();
                }
                Ok(())
            };
            let result = run();
            let mut slot = state.0.lock().unwrap();
            slot.ended = true;
            slot.error = result.err();
            state.1.notify_one();
        });
        match rx.recv().map_err(|_| Error::CaptureFailed)? {
            Ok((dimensions, origin)) => Ok(Self {
                slot,
                stop,
                thread: Some(thread),
                dimensions,
                origin,
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
    pub fn timestamp_origin(&self) -> Instant {
        self.origin
    }
}
impl Camera for LatestCamera {
    fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }
    fn capture(&mut self, frame: &mut Frame) -> Result<bool> {
        frame.clear();
        if (frame.width(), frame.height()) != self.dimensions {
            return Err(Error::InvalidFrame);
        }
        let mut slot = self.slot.0.lock().unwrap();
        while slot.frame.is_none() && !slot.ended {
            slot = self.slot.1.wait(slot).unwrap();
        }
        if let Some(error) = slot.error.take() {
            return Err(error);
        }
        if let Some(mut latest) = slot.frame.take() {
            std::mem::swap(frame, &mut latest);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
impl Drop for LatestCamera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    struct Burst(u64);
    impl Camera for Burst {
        fn dimensions(&self) -> (u32, u32) {
            (2, 2)
        }
        fn capture(&mut self, frame: &mut Frame) -> Result<bool> {
            if self.0 == 100 {
                return Ok(false);
            }
            self.0 += 1;
            frame.set_timestamp(Duration::from_millis(self.0));
            frame.rgb_mut().fill(self.0 as u8);
            Ok(true)
        }
    }
    #[test]
    fn slow_consumer_gets_latest_acquisition_not_a_queue() {
        let mut camera = LatestCamera::start(|| Ok((Burst(0), Instant::now()))).unwrap();
        // Deterministic wait for the producer, not a timing-sensitive sleep.
        camera.thread.take().unwrap().join().unwrap();
        let mut frame = Frame::new(2, 2).unwrap();
        assert!(camera.capture(&mut frame).unwrap());
        assert_eq!(frame.timestamp(), Duration::from_millis(100));
        assert_eq!(frame.rgb(), &[100; 12]);
        assert!(!camera.capture(&mut frame).unwrap());
        assert_eq!(frame.rgb(), &[0; 12]);
    }
}

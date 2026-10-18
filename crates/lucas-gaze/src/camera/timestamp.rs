//! Normalize nokhwa 0.4.6's `stream_epoch + ReadSample time` into elapsed
//! monotonic acquisition time. Live MF sources normally use the QPC-based MF
//! system clock, but some sources report time relative to stream start instead.
use crate::{Error, Result};
use std::time::Duration;

// Allow small clock-pairing/driver jitter, not an uptime-sized clock mismatch.
const CLOCK_TOLERANCE: Duration = Duration::from_millis(100);
const MAX_FRAME_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
enum Domain {
    System,
    Stream,
}

pub(super) struct CaptureClock {
    stream_epoch: Duration,
    stream_start: Duration,
    domain: Option<Domain>,
    last: Option<Duration>,
    last_acquired: Option<Duration>,
}

impl CaptureClock {
    pub(super) fn new(stream_epoch: Duration, stream_start: Duration) -> Self {
        Self {
            stream_epoch,
            stream_start,
            domain: None,
            last: None,
            last_acquired: None,
        }
    }

    /// `received` is measured immediately after frame receipt, BEFORE JPEG
    /// decoding/inference. `system_now` is MFGetSystemTime at that same point.
    pub(super) fn map(
        &mut self,
        encoded: Option<Duration>,
        received: Duration,
        system_now: Duration,
    ) -> Result<Duration> {
        let raw = match encoded {
            Some(time) if time >= self.stream_epoch => Some(time - self.stream_epoch),
            // Our epoch is sampled just after open_stream returns, whereas
            // nokhwa samples just before returning. A near-zero stream sample
            // can therefore precede our epoch by that small return-path delay.
            Some(time) if self.stream_epoch - time <= CLOCK_TOLERANCE => None,
            Some(_) => return Err(Error::CameraTimestamp),
            None => None,
        };
        let candidate = match raw.filter(|time| !time.is_zero()) {
            None => {
                // nokhwa discards zero (and missing) sample times. Acquisition
                // cannot be recovered in this case: receipt is the best bound,
                // never decode or inference completion. Do not lock the domain
                // until a real timestamp arrives.
                received
            }
            Some(raw) => {
                let system = if raw <= system_now {
                    received.checked_sub(system_now - raw)
                } else if raw - system_now <= CLOCK_TOLERANCE {
                    Some(received)
                } else {
                    None
                };
                let stream = self.stream_start.checked_add(raw);
                let plausible = |candidate: Option<Duration>| {
                    candidate.filter(|time| {
                        if *time > received {
                            *time - received <= CLOCK_TOLERANCE
                        } else {
                            received - *time <= MAX_FRAME_AGE
                        }
                    })
                };
                let system = plausible(system);
                let stream = plausible(stream);
                let (domain, time) = match self.domain {
                    Some(Domain::System) => (Domain::System, system),
                    Some(Domain::Stream) => (Domain::Stream, stream),
                    None if system.is_some() => (Domain::System, system),
                    None => (Domain::Stream, stream),
                };
                let time = time.ok_or(Error::CameraTimestamp)?;
                // Never reinterpret garbage or a clock reset as another domain.
                let time = time.min(received);
                if self
                    .last_acquired
                    .is_some_and(|last| last.saturating_sub(time) > CLOCK_TOLERANCE)
                {
                    return Err(Error::CameraTimestamp);
                }
                self.domain = Some(domain);
                self.last_acquired = Some(time);
                time
            }
        };
        let timestamp = match self.last {
            Some(last) if candidate <= last => {
                // Quantized/repeated times and a real sample following a
                // receipt-time fallback must still advance the gaze filter.
                // A fallback is an upper bound, not evidence of a clock reset.
                last.checked_add(Duration::from_nanos(1))
                    .filter(|time| *time <= received)
                    .ok_or(Error::CameraTimestamp)?
            }
            _ => candidate,
        };
        self.last = Some(timestamp);
        Ok(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: Duration = Duration::from_secs(1_700_000_000);
    const UPTIME: Duration = Duration::from_secs(15_480); // 4.3 hours, C270 report
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn qpc_samples_preserve_age_and_frame_gaps_not_receipt_spacing() {
        let mut clock = CaptureClock::new(EPOCH, ms(0));
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(33)), ms(50), UPTIME + ms(50)),
            Ok(ms(33))
        );
        // A slow inference delays the next read, but not the frame's acquisition.
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(66)), ms(350), UPTIME + ms(350)),
            Ok(ms(66))
        );
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(400)), ms(450), UPTIME + ms(450)),
            Ok(ms(400))
        );
    }

    #[test]
    fn stream_relative_samples_include_stream_start_and_preserve_age() {
        let mut clock = CaptureClock::new(EPOCH, ms(200));
        assert_eq!(
            clock.map(Some(EPOCH + ms(33)), ms(250), UPTIME),
            Ok(ms(233))
        );
        assert_eq!(
            clock.map(Some(EPOCH + ms(66)), ms(550), UPTIME),
            Ok(ms(266))
        );
    }

    #[test]
    fn zero_and_missing_use_pre_decode_receipt_then_accept_real_samples() {
        for missing in [None, Some(EPOCH), Some(EPOCH - ms(1))] {
            let mut clock = CaptureClock::new(EPOCH, ms(0));
            assert_eq!(clock.map(missing, ms(50), UPTIME + ms(50)), Ok(ms(50)));
            // First valid sample is slightly older than the fallback receipt.
            let next = clock
                .map(Some(EPOCH + UPTIME + ms(49)), ms(60), UPTIME + ms(60))
                .unwrap();
            assert_eq!(next, ms(50) + Duration::from_nanos(1));
            assert_eq!(clock.map(None, ms(80), UPTIME + ms(80)), Ok(ms(80)));
        }
    }

    #[test]
    fn buffered_sample_after_missing_time_does_not_look_like_clock_reset() {
        let mut clock = CaptureClock::new(EPOCH, ms(0));
        clock.map(None, ms(300), UPTIME + ms(300)).unwrap();
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(66)), ms(350), UPTIME + ms(350)),
            Ok(ms(300) + Duration::from_nanos(1))
        );
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(400)), ms(450), UPTIME + ms(450)),
            Ok(ms(400))
        );
    }

    #[test]
    fn epoch_estimate_error_and_small_future_jitter_are_bounded() {
        let mut clock = CaptureClock::new(EPOCH + ms(2), ms(2));
        // The post-open epoch estimate overestimates frame age by just 2ms.
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(33)), ms(50), UPTIME + ms(50)),
            Ok(ms(31))
        );
        let mut clock = CaptureClock::new(EPOCH, ms(0));
        assert_eq!(
            clock.map(Some(EPOCH + UPTIME + ms(51)), ms(50), UPTIME + ms(50)),
            Ok(ms(50))
        );
    }

    #[test]
    fn duplicate_samples_advance_but_backward_jumps_fail() {
        let mut clock = CaptureClock::new(EPOCH, ms(0));
        assert_eq!(
            clock.map(Some(EPOCH + ms(200)), ms(220), UPTIME),
            Ok(ms(200))
        );
        assert_eq!(
            clock.map(Some(EPOCH + ms(200)), ms(250), UPTIME),
            Ok(ms(200) + Duration::from_nanos(1))
        );
        assert_eq!(
            clock.map(Some(EPOCH + ms(1)), ms(260), UPTIME),
            Err(Error::CameraTimestamp)
        );
    }

    #[test]
    fn garbage_future_stale_and_overflow_times_fail_without_panicking() {
        for (encoded, received, system) in [
            (Some(EPOCH + UPTIME + ms(1000)), ms(50), UPTIME + ms(50)),
            (Some(EPOCH - Duration::from_secs(1)), ms(50), UPTIME),
            (Some(Duration::MAX), ms(50), UPTIME),
            (Some(EPOCH + ms(1)), Duration::from_secs(31), UPTIME),
        ] {
            let mut clock = CaptureClock::new(EPOCH, ms(0));
            assert_eq!(
                clock.map(encoded, received, system),
                Err(Error::CameraTimestamp)
            );
        }
    }

    #[test]
    fn established_clock_domain_cannot_switch_after_reset() {
        let mut clock = CaptureClock::new(EPOCH, ms(0));
        clock
            .map(Some(EPOCH + UPTIME + ms(33)), ms(50), UPTIME + ms(50))
            .unwrap();
        assert_eq!(
            clock.map(Some(EPOCH + ms(66)), ms(80), UPTIME + ms(80)),
            Err(Error::CameraTimestamp)
        );
    }
}

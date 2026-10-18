use crate::{Error, Result};
use std::time::Duration;

const MAX_FRAME_AGE: Duration = Duration::from_secs(30);
const INVALID_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Domain {
    System,
    Stream,
}

pub(super) struct CaptureClock {
    stream_start: Duration,
    domain: Option<Domain>,
    last: Option<Duration>,
    invalid_since: Option<Duration>,
}

impl CaptureClock {
    pub(super) fn new(stream_start: Duration) -> Self {
        Self {
            stream_start,
            domain: None,
            last: None,
            invalid_since: None,
        }
    }

    pub(super) fn map(
        &mut self,
        raw: Option<Duration>,
        received: Duration,
        system_now: Duration,
    ) -> Result<Option<Duration>> {
        let candidate = raw.filter(|time| !time.is_zero()).and_then(|raw| {
            let plausible = |candidate: Option<Duration>| {
                candidate.filter(|time| {
                    *time >= self.stream_start
                        && *time <= received
                        && received - *time <= MAX_FRAME_AGE
                })
            };
            let system = plausible(
                system_now
                    .checked_sub(raw)
                    .and_then(|age| received.checked_sub(age)),
            );
            let stream = plausible(self.stream_start.checked_add(raw));
            match self.domain {
                Some(Domain::System) => system.map(|time| (Domain::System, time)),
                Some(Domain::Stream) => stream.map(|time| (Domain::Stream, time)),
                None => match (system, stream) {
                    (Some(time), None) => Some((Domain::System, time)),
                    (None, Some(time)) => Some((Domain::Stream, time)),
                    (Some(a), Some(b)) if a == b => Some((Domain::System, a)),
                    _ => None,
                },
            }
        });
        if let Some((domain, time)) = candidate
            && self.last.is_none_or(|last| time > last)
        {
            self.domain = Some(domain);
            self.last = Some(time);
            self.invalid_since = None;
            return Ok(Some(time));
        }
        let since = *self.invalid_since.get_or_insert(received);
        if received.saturating_sub(since) >= INVALID_TIMEOUT {
            Err(Error::CameraTimestamp)
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPTIME: Duration = Duration::from_secs(15_480);
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn qpc_samples_preserve_age_and_frame_gaps_not_receipt_spacing() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(
            clock.map(Some(UPTIME + ms(33)), ms(50), UPTIME + ms(50)),
            Ok(Some(ms(33)))
        );
        assert_eq!(
            clock.map(Some(UPTIME + ms(66)), ms(350), UPTIME + ms(350)),
            Ok(Some(ms(66)))
        );
        assert_eq!(
            clock.map(Some(UPTIME + ms(400)), ms(450), UPTIME + ms(450)),
            Ok(Some(ms(400)))
        );
    }

    #[test]
    fn delayed_open_return_does_not_shift_raw_qpc_acquisition() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(
            clock.map(Some(UPTIME + ms(230)), ms(250), UPTIME + ms(250)),
            Ok(Some(ms(230)))
        );
    }

    #[test]
    fn stream_relative_samples_include_backend_start_and_preserve_age() {
        let mut clock = CaptureClock::new(ms(200));
        assert_eq!(clock.map(Some(ms(33)), ms(250), UPTIME), Ok(Some(ms(233))));
        assert_eq!(clock.map(Some(ms(66)), ms(550), UPTIME), Ok(Some(ms(266))));
    }

    #[test]
    fn zero_and_missing_are_skipped_then_real_samples_are_unchanged() {
        for missing in [None, Some(Duration::ZERO)] {
            let mut clock = CaptureClock::new(ms(0));
            assert_eq!(clock.map(missing, ms(300), UPTIME + ms(300)), Ok(None));
            assert_eq!(
                clock.map(Some(UPTIME + ms(66)), ms(350), UPTIME + ms(350)),
                Ok(Some(ms(66)))
            );
            assert_eq!(clock.map(None, ms(380), UPTIME + ms(380)), Ok(None));
            assert_eq!(
                clock.map(Some(UPTIME + ms(400)), ms(450), UPTIME + ms(450)),
                Ok(Some(ms(400)))
            );
        }
    }

    #[test]
    fn future_samples_are_not_clamped_to_receipt() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(
            clock.map(Some(UPTIME + ms(51)), ms(50), UPTIME + ms(50)),
            Ok(None)
        );
        assert_eq!(
            clock.map(Some(UPTIME + ms(60)), ms(80), UPTIME + ms(80)),
            Ok(Some(ms(60)))
        );
    }

    #[test]
    fn duplicates_and_backward_samples_are_skipped_without_retiming() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(clock.map(Some(ms(200)), ms(220), UPTIME), Ok(Some(ms(200))));
        assert_eq!(clock.map(Some(ms(200)), ms(250), UPTIME), Ok(None));
        assert_eq!(clock.map(Some(ms(199)), ms(260), UPTIME), Ok(None));
        assert_eq!(clock.map(Some(ms(1)), ms(270), UPTIME), Ok(None));
        assert_eq!(clock.map(Some(ms(233)), ms(300), UPTIME), Ok(Some(ms(233))));
    }

    #[test]
    fn future_stale_and_overflow_times_are_skipped_without_panicking() {
        for (raw, received, system) in [
            (Some(UPTIME + ms(1000)), ms(50), UPTIME + ms(50)),
            (Some(Duration::MAX), ms(50), UPTIME),
            (Some(ms(1)), Duration::from_secs(31), UPTIME),
        ] {
            let mut clock = CaptureClock::new(ms(1));
            assert_eq!(clock.map(raw, received, system), Ok(None));
        }
    }

    #[test]
    fn persistent_missing_or_implausible_times_fail_after_three_seconds() {
        for invalid in [None, Some(Duration::ZERO), Some(Duration::MAX)] {
            let mut clock = CaptureClock::new(ms(0));
            assert_eq!(clock.map(invalid, ms(50), UPTIME + ms(50)), Ok(None));
            assert_eq!(clock.map(invalid, ms(3049), UPTIME + ms(3049)), Ok(None));
            assert_eq!(
                clock.map(invalid, ms(3050), UPTIME + ms(3050)),
                Err(Error::CameraTimestamp)
            );
        }
    }

    #[test]
    fn valid_frame_resets_the_invalid_timestamp_timeout() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(clock.map(None, ms(50), UPTIME), Ok(None));
        assert_eq!(
            clock.map(Some(ms(2990)), ms(3000), UPTIME),
            Ok(Some(ms(2990)))
        );
        assert_eq!(clock.map(None, ms(3100), UPTIME), Ok(None));
        assert_eq!(clock.map(None, ms(6099), UPTIME), Ok(None));
        assert_eq!(
            clock.map(None, ms(6100), UPTIME),
            Err(Error::CameraTimestamp)
        );
    }

    #[test]
    fn established_clock_domain_cannot_switch_after_reset() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(
            clock.map(Some(UPTIME + ms(33)), ms(50), UPTIME + ms(50)),
            Ok(Some(ms(33)))
        );
        assert_eq!(clock.map(Some(ms(66)), ms(80), UPTIME + ms(80)), Ok(None));
        assert_eq!(
            clock.map(Some(ms(99)), ms(3080), UPTIME + ms(3080)),
            Err(Error::CameraTimestamp)
        );
    }

    #[test]
    fn ambiguous_initial_clock_domain_is_not_guessed() {
        let mut clock = CaptureClock::new(ms(0));
        assert_eq!(clock.map(Some(ms(100)), ms(200), ms(250)), Ok(None));
    }
}

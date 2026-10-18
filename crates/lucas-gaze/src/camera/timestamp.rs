use crate::{Error, Result};
use std::time::Duration;

const MAX_FRAME_AGE: Duration = Duration::from_secs(1);
const INVALID_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct CaptureClock {
    last: Option<Duration>,
    invalid_since: Option<Duration>,
}

impl CaptureClock {
    pub(super) fn new() -> Self {
        Self {
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
        let candidate = raw
            .filter(|time| !time.is_zero())
            .and_then(|raw| system_now.checked_sub(raw))
            .filter(|age| *age <= MAX_FRAME_AGE)
            .and_then(|age| received.checked_sub(age));
        if let Some(time) = candidate
            && self.last.is_none_or(|last| time > last)
        {
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
        let mut clock = CaptureClock::new();
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
        let mut clock = CaptureClock::new();
        assert_eq!(
            clock.map(Some(UPTIME + ms(230)), ms(250), UPTIME + ms(250)),
            Ok(Some(ms(230)))
        );
    }

    #[test]
    fn stream_relative_samples_after_initialization_delay_are_skipped() {
        let mut clock = CaptureClock::new();
        assert_eq!(clock.map(Some(ms(33)), ms(250), UPTIME + ms(250)), Ok(None));
        assert_eq!(clock.map(Some(ms(66)), ms(550), UPTIME + ms(550)), Ok(None));
        assert_eq!(
            clock.map(Some(UPTIME + ms(600)), ms(650), UPTIME + ms(650)),
            Ok(Some(ms(600)))
        );
    }

    #[test]
    fn zero_and_missing_are_skipped_then_real_samples_are_unchanged() {
        for missing in [None, Some(Duration::ZERO)] {
            let mut clock = CaptureClock::new();
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
    fn only_ages_from_zero_to_one_second_are_accepted() {
        let received = ms(2000);
        let system = UPTIME + received;
        let tick = Duration::from_nanos(100);
        for (raw, expected) in [
            (system, Some(received)),
            (system - ms(1000), Some(ms(1000))),
            (system - ms(1000) - tick, None),
            (system + tick, None),
        ] {
            let mut clock = CaptureClock::new();
            assert_eq!(clock.map(Some(raw), received, system), Ok(expected));
        }
    }

    #[test]
    fn future_samples_are_not_clamped_to_receipt() {
        let mut clock = CaptureClock::new();
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
        let mut clock = CaptureClock::new();
        for (acquired, received, expected) in [
            (200, 220, Some(ms(200))),
            (200, 250, None),
            (199, 260, None),
            (1, 270, None),
            (233, 300, Some(ms(233))),
        ] {
            assert_eq!(
                clock.map(
                    Some(UPTIME + ms(acquired)),
                    ms(received),
                    UPTIME + ms(received)
                ),
                Ok(expected)
            );
        }
    }

    #[test]
    fn implausible_times_are_skipped_without_panicking() {
        for (raw, received, system) in [
            (Some(Duration::MAX), ms(50), UPTIME),
            (Some(UPTIME - ms(1)), ms(50), UPTIME + ms(50)),
            (Some(UPTIME + ms(1)), ms(1002), UPTIME + ms(1002)),
        ] {
            let mut clock = CaptureClock::new();
            assert_eq!(clock.map(raw, received, system), Ok(None));
        }
    }

    #[test]
    fn persistent_missing_or_implausible_times_fail_after_three_seconds() {
        for invalid in [
            None,
            Some(Duration::ZERO),
            Some(Duration::MAX),
            Some(ms(33)),
        ] {
            let mut clock = CaptureClock::new();
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
        let mut clock = CaptureClock::new();
        assert_eq!(clock.map(None, ms(50), UPTIME + ms(50)), Ok(None));
        assert_eq!(
            clock.map(Some(UPTIME + ms(2990)), ms(3000), UPTIME + ms(3000)),
            Ok(Some(ms(2990)))
        );
        assert_eq!(clock.map(None, ms(3100), UPTIME + ms(3100)), Ok(None));
        assert_eq!(clock.map(None, ms(6099), UPTIME + ms(6099)), Ok(None));
        assert_eq!(
            clock.map(None, ms(6100), UPTIME + ms(6100)),
            Err(Error::CameraTimestamp)
        );
    }

    #[test]
    fn stream_relative_times_after_qpc_samples_are_still_skipped() {
        let mut clock = CaptureClock::new();
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
}

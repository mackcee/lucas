//! Timestamp-driven One-Euro filter (Casiez et al., CHI 2012).
use crate::{Error, Result};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct OneEuro {
    min_cutoff: f32,
    beta: f32,
    derivative_cutoff: f32,
    state: Option<(Duration, f32, f32, f32)>,
}
impl OneEuro {
    pub fn new(min_cutoff: f32, beta: f32, derivative_cutoff: f32) -> Result<Self> {
        if !min_cutoff.is_finite()
            || min_cutoff <= 0.0
            || !beta.is_finite()
            || beta < 0.0
            || !derivative_cutoff.is_finite()
            || derivative_cutoff <= 0.0
        {
            return Err(Error::InvalidFilter);
        }
        Ok(Self {
            min_cutoff,
            beta,
            derivative_cutoff,
            state: None,
        })
    }
    pub fn reset(&mut self) {
        self.state = None;
    }
    pub fn filter(&mut self, t: Duration, value: f32) -> Result<f32> {
        if !value.is_finite() {
            return Err(Error::InvalidOutput);
        }
        let Some((previous_t, previous_raw, previous_filtered, previous_derivative)) = self.state
        else {
            self.state = Some((t, value, value, 0.0));
            return Ok(value);
        };
        if t <= previous_t {
            return Err(Error::InvalidTimestamp);
        }
        let dt = (t - previous_t).as_secs_f32();
        // Reacquisition must not smear a new face with old values.
        if dt > 0.25 {
            self.reset();
            return self.filter(t, value);
        }
        let derivative = (value - previous_raw) / dt;
        let derivative = blend(
            alpha(self.derivative_cutoff, dt),
            derivative,
            previous_derivative,
        );
        let cutoff = self.min_cutoff + self.beta * derivative.abs();
        let filtered = blend(alpha(cutoff, dt), value, previous_filtered);
        self.state = Some((t, value, filtered, derivative));
        Ok(filtered)
    }
}
fn alpha(cutoff: f32, dt: f32) -> f32 {
    1.0 / (1.0 + 1.0 / (std::f32::consts::TAU * cutoff * dt))
}
fn blend(a: f32, x: f32, previous: f32) -> f32 {
    a * x + (1.0 - a) * previous
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smooths_jitter_and_resets_after_gap() {
        let mut filter = OneEuro::new(1.0, 0.1, 1.0).unwrap();
        assert_eq!(filter.filter(Duration::ZERO, 1.0).unwrap(), 1.0);
        let smoothed = filter.filter(Duration::from_millis(33), 1.1).unwrap();
        assert!((1.0..1.05).contains(&smoothed));
        assert_eq!(filter.filter(Duration::from_secs(1), 4.0).unwrap(), 4.0);
        assert_eq!(
            filter.filter(Duration::from_secs(1), 4.0),
            Err(Error::InvalidTimestamp)
        );
    }
    #[test]
    fn rejects_nonfinite_and_invalid_parameters() {
        assert!(OneEuro::new(0.0, 1.0, 1.0).is_err());
        assert!(OneEuro::new(1.0, f32::NAN, 1.0).is_err());
        assert!(
            OneEuro::new(1.0, 0.0, 1.0)
                .unwrap()
                .filter(Duration::ZERO, f32::NAN)
                .is_err()
        );
    }
}

use crate::{CoreError, GateInputs, Timestamp, types::elapsed};

/// The half-open interval [from, to) uses the *previous* gate and gaze state.
/// Consumers must report gate transitions (including tracking loss) promptly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapInterval {
    pub from: Timestamp,
    pub to: Timestamp,
    pub eligible: bool,
    pub looking: bool,
    pub gap_start_ms: u64,
    /// Gap after integrating the interval, before a check/reset at `to`.
    pub gap_end_ms: u64,
    pub reset_at_end: bool,
}

impl GapInterval {
    pub fn eligible_ms(self) -> u64 {
        if self.eligible {
            self.to.0 - self.from.0
        } else {
            0
        }
    }
}

/// An eligible-time clock: foreground/tracking/pause losses freeze it, while
/// death, pre-game and end-of-game reset it. Sustained minimap viewing keeps it
/// at zero. Starting/resuming a session is not itself a valid minimap check.
#[derive(Clone, Debug, Default)]
pub struct GapClock {
    last_at: Option<Timestamp>,
    gate: GateInputs,
    looking: bool,
    gap_ms: u64,
    last_check_at: Option<Timestamp>,
}

impl GapClock {
    pub fn gap_ms(&self) -> u64 {
        self.gap_ms
    }
    pub fn eligible(&self) -> bool {
        self.gate.eligible()
    }
    pub fn last_check_at(&self) -> Option<Timestamp> {
        self.last_check_at
    }

    pub fn update(
        &mut self,
        at: Timestamp,
        gate: GateInputs,
        looking: bool,
    ) -> Result<GapInterval, CoreError> {
        let dt = elapsed(self.last_at, at)?;
        let start = self.gap_ms;
        if self.gate.eligible() && !self.looking {
            self.gap_ms = self.gap_ms.saturating_add(dt);
        }
        let interval = GapInterval {
            from: self.last_at.unwrap_or(at),
            to: at,
            eligible: self.gate.eligible(),
            looking: self.looking,
            gap_start_ms: start,
            gap_end_ms: self.gap_ms,
            reset_at_end: gate.resets_gap(),
        };
        if gate.resets_gap() {
            self.gap_ms = 0;
            self.last_check_at = None;
        } else if gate.eligible() && looking {
            self.gap_ms = 0;
            self.last_check_at = Some(at);
        }
        self.gate = gate;
        self.looking = looking && gate.eligible();
        self.last_at = Some(at);
        Ok(interval)
    }
}

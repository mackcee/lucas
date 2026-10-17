use lucas_game::{Eligibility, WindowStatus};
use std::time::Duration;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    #[default]
    LiveClient,
    /// Explicit opt-in: cannot distinguish loading, death, map or game end.
    WindowOnly,
}

#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub game: Eligibility,
    pub window: WindowStatus,
    pub geometry_ready: bool,
    pub aligning: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Waiting,
    Active,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub status: Status,
    pub pulse: bool,
    pub hide: bool,
}

/// A fresh uninterrupted eligible interval is required after every off-switch.
/// No catch-up pulses, wall-clock dates or game-time interpolation.
#[derive(Debug, Default)]
pub struct Timer {
    mode: GateMode,
    paused: bool,
    since: Option<Duration>,
}

impl Timer {
    pub const INTERVAL: Duration = Duration::from_secs(10);

    pub fn mode(&self) -> GateMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: GateMode) {
        if self.mode != mode {
            self.mode = mode;
            self.reset();
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.since = None;
    }

    pub fn tick(&mut self, now: Duration, input: Inputs) -> Decision {
        let active = !self.paused
            && !input.aligning
            && input.geometry_ready
            && input.window.overlay_eligible()
            && (self.mode == GateMode::WindowOnly || input.game.eligible());
        if !active {
            self.reset();
            return Decision {
                status: if self.paused || input.aligning {
                    Status::Paused
                } else {
                    Status::Waiting
                },
                pulse: false,
                hide: true,
            };
        }
        let since = self.since.get_or_insert(now);
        // A rewound injected clock also starts a fresh interval.
        if now < *since {
            *since = now;
        }
        let pulse = now - *since >= Self::INTERVAL;
        if pulse {
            *since = now;
        }
        Decision {
            status: Status::Active,
            pulse,
            hide: false,
        }
    }
}

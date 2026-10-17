//! Read-only League Live Client integration. No LCU, process handles, hooks or input.
//!
//! Call [`LiveClient`]'s blocking, bounded polls on an app-owned worker thread;
//! [`GameState`] consumes timestamped snapshots without a clock or OS dependency.
//! The app combines [`Eligibility`] with gaze and [`WindowStatus`] and owns any
//! explicit window-only fallback. Failed polls never silently enable game eligibility.

#![cfg_attr(not(windows), forbid(unsafe_code))]
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]

mod api;
mod model;
mod settings;
mod state;
mod window;

pub use api::{ClientError, LiveClient, LiveDataSource, Poller, RIOT_ROOT_PEM};
pub use model::{Event, EventData, GameStats, Player, Snapshot};
pub use settings::{SettingsFile, SettingsFormat, SettingsLocator};
pub use state::{Death, Eligibility, GamePhase, GameState, MapKind, StateUpdate};
#[cfg(windows)]
pub use window::Win32WindowProbe;
pub use window::{DisplayMode, FakeWindowProbe, ScreenRect, WindowProbe, WindowStatus};

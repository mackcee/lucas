use crate::{Result, StoreError, ensure_version};
use lucas_core::TRAINING_LEVELS;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseStyle {
    #[default]
    Ripple,
    BreathingBorder,
    CornerBrackets,
    CometSweep,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PulseSettings {
    pub style: PulseStyle,
    pub colour_rgb: [u8; 3],
    pub sound: bool,
}

impl Default for PulseSettings {
    fn default() -> Self {
        Self {
            style: PulseStyle::Ripple,
            colour_rgb: [255, 191, 0],
            sound: false,
        }
    }
}

/// Unknown keys are tolerated; absent fields (including nested fields) use defaults.
/// Malformed input and future versions return errors instead of overwriting data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub setup_completed: bool,
    pub pulse: PulseSettings,
    pub live_client_gate: bool,
    pub window_only_fallback: bool,
    pub density_cap: bool,
    /// Index into lucas-core's manual programme, initially the 10-second ramp.
    pub level_index: usize,
    pub camera_index: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            setup_completed: false,
            pulse: PulseSettings::default(),
            live_client_gate: true,
            window_only_fallback: true,
            density_cap: true,
            level_index: 1,
            camera_index: 0,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure_version("settings", self.schema_version)?;
        if self.level_index >= TRAINING_LEVELS.len() {
            return Err(StoreError::Invalid("unsupported training level index"));
        }
        Ok(())
    }
}

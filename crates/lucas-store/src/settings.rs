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

/// Diagonal size presets; dimensions use the calibration monitor's resolution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenSize {
    #[default]
    Inches24,
    Inches27,
    Inches32,
    Inches34,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalibrationPreferences {
    pub screen_size: ScreenSize,
    pub custom_size_mm: [u32; 2],
    pub viewing_distance_mm: u32,
}

impl Default for CalibrationPreferences {
    fn default() -> Self {
        Self {
            screen_size: ScreenSize::default(),
            custom_size_mm: [531, 299],
            viewing_distance_mm: 650,
        }
    }
}

impl CalibrationPreferences {
    /// Whole millimetres, independent of Windows DPI/scaling. Reject missing
    /// resolution rather than silently assuming a 16:9 calibration monitor.
    pub fn physical_size_mm(&self, resolution: [u32; 2]) -> Result<[u32; 2]> {
        let inches = match self.screen_size {
            ScreenSize::Inches24 => 24.0,
            ScreenSize::Inches27 => 27.0,
            ScreenSize::Inches32 => 32.0,
            ScreenSize::Inches34 => 34.0,
            ScreenSize::Custom => return Ok(self.custom_size_mm),
        };
        if resolution.contains(&0) {
            return Err(StoreError::Invalid(
                "missing calibration monitor resolution",
            ));
        }
        let [width, height] = resolution.map(f64::from);
        let mm_per_pixel = inches * 25.4 / width.hypot(height);
        Ok([
            (width * mm_per_pixel).round() as u32,
            (height * mm_per_pixel).round() as u32,
        ])
    }

    pub fn validate(&self) -> Result<()> {
        if self
            .custom_size_mm
            .into_iter()
            .chain([self.viewing_distance_mm])
            .any(|mm| !(100..=3000).contains(&mm))
        {
            return Err(StoreError::Invalid(
                "screen dimensions and viewing distance must be 100–3000 mm",
            ));
        }
        Ok(())
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
    pub calibration: CalibrationPreferences,
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
            calibration: CalibrationPreferences::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure_version("settings", self.schema_version)?;
        self.calibration.validate()?;
        if self.level_index >= TRAINING_LEVELS.len() {
            return Err(StoreError::Invalid("unsupported training level index"));
        }
        Ok(())
    }
}

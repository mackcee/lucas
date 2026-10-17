use std::fmt;

use serde_json::Value;

use crate::{MinimapSettings, valid_scale};

/// Values actually present in a supplied settings file. Missing is not defaulted.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SettingsPatch {
    pub minimap_scale: Option<f64>,
    pub flipped: Option<bool>,
}

impl SettingsPatch {
    /// Prefer this patch per field, filling missing fields from `fallback`.
    /// Typically: persisted settings `.with_fallback(game_cfg)`.
    pub fn with_fallback(self, fallback: Self) -> Self {
        Self {
            minimap_scale: self.minimap_scale.or(fallback.minimap_scale),
            flipped: self.flipped.or(fallback.flipped),
        }
    }

    /// Fill still-missing fields from an explicit application default.
    /// Manually constructed values are validated by `minimap_rect`.
    pub fn resolve(self, defaults: MinimapSettings) -> MinimapSettings {
        MinimapSettings {
            minimap_scale: self.minimap_scale.unwrap_or(defaults.minimap_scale),
            flipped: self.flipped.unwrap_or(defaults.flipped),
        }
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), SettingsError> {
        if key.eq_ignore_ascii_case("MinimapScale") {
            if self.minimap_scale.is_some() {
                return Err(SettingsError::Duplicate("MinimapScale"));
            }
            let scale = value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|s| valid_scale(*s))
                .ok_or(SettingsError::InvalidValue("MinimapScale"))?;
            self.minimap_scale = Some(scale);
        } else if key.eq_ignore_ascii_case("FlipMiniMap") {
            if self.flipped.is_some() {
                return Err(SettingsError::Duplicate("FlipMiniMap"));
            }
            let flipped = match value.trim() {
                "0" => false,
                "1" => true,
                s if s.eq_ignore_ascii_case("false") => false,
                s if s.eq_ignore_ascii_case("true") => true,
                _ => return Err(SettingsError::InvalidValue("FlipMiniMap")),
            };
            self.flipped = Some(flipped);
        }
        Ok(())
    }
}

/// Relevant malformed values fail explicitly so callers can report or fall back.
/// Unknown files, sections and keys are ignored. Duplicate relevant keys are
/// errors rather than dependent on file ordering.
#[derive(Debug)]
pub enum SettingsError {
    Json(serde_json::Error),
    Structure(&'static str),
    InvalidValue(&'static str),
    Duplicate(&'static str),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid PersistedSettings JSON: {e}"),
            Self::Structure(message) => f.write_str(message),
            Self::InvalidValue(key) => write!(f, "invalid HUD {key} value"),
            Self::Duplicate(key) => write!(f, "duplicate HUD {key} setting"),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

fn relevant(key: &str) -> bool {
    key.eq_ignore_ascii_case("MinimapScale") || key.eq_ignore_ascii_case("FlipMiniMap")
}

fn named(value: &Value, name: &str) -> bool {
    value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|s| s.eq_ignore_ascii_case(name))
}

/// Parses `files[name=game.cfg].sections[name=HUD].settings[{name,value}]`.
///
/// League usually stores values as strings; JSON numbers and booleans are also
/// accepted where appropriate. A UTF-8 BOM is tolerated. Unrelated file entries
/// (e.g. input.ini) cannot override game.cfg. An absent HUD or game.cfg gives an
/// empty patch; broken JSON, required container shapes or relevant values error.
/// No file discovery or fallback I/O occurs here.
pub fn parse_persisted_settings(content: &str) -> Result<SettingsPatch, SettingsError> {
    let root: Value = serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .map_err(SettingsError::Json)?;
    let files = root
        .get("files")
        .and_then(Value::as_array)
        .ok_or(SettingsError::Structure(
            "PersistedSettings must contain a files array",
        ))?;
    let mut patch = SettingsPatch::default();
    for file in files.iter().filter(|v| named(v, "game.cfg")) {
        let sections =
            file.get("sections")
                .and_then(Value::as_array)
                .ok_or(SettingsError::Structure(
                    "game.cfg must contain a sections array",
                ))?;
        for section in sections.iter().filter(|v| named(v, "HUD")) {
            let settings = section.get("settings").and_then(Value::as_array).ok_or(
                SettingsError::Structure("HUD must contain a settings array"),
            )?;
            for setting in settings {
                let Some(key) = setting
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| relevant(s))
                else {
                    continue;
                };
                let value = setting.get("value");
                let text = match value {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    Some(Value::Bool(b)) => b.to_string(),
                    _ => return Err(SettingsError::Structure("HUD setting needs a scalar value")),
                };
                patch.set(key, &text)?;
            }
        }
    }
    Ok(patch)
}

/// Parses only `[HUD]` MinimapScale/FlipMiniMap from supplied game.cfg text.
///
/// Supports LF/CRLF, a UTF-8 BOM, case-insensitive names, whitespace and `;`/`#`
/// line or trailing comments. Other keys/sections are ignored. Missing fields
/// stay `None`; malformed relevant lines/values and duplicates fail explicitly.
pub fn parse_game_cfg(content: &str) -> Result<SettingsPatch, SettingsError> {
    let mut patch = SettingsPatch::default();
    let mut in_hud = false;
    for raw in content.trim_start_matches('\u{feff}').lines() {
        let line = raw.split([';', '#']).next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let section = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .ok_or(SettingsError::Structure(
                    "malformed game.cfg section header",
                ))?;
            in_hud = section.trim().eq_ignore_ascii_case("HUD");
            continue;
        }
        if !in_hud {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            patch.set(key.trim(), value)?;
        } else if line.split_whitespace().next().is_some_and(relevant) {
            return Err(SettingsError::Structure("HUD setting is missing '='"));
        }
    }
    Ok(patch)
}

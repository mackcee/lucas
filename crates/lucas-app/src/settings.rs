use lucas_game::{SettingsFormat, SettingsLocator};
use lucas_geometry::{MinimapSettings, parse_persisted_settings};
use lucas_overlay::PulseStyle as RenderStyle;
use lucas_store::PulseStyle as StoredStyle;
use serde::{Deserialize, Serialize};
use std::io;

pub fn pulse_style(style: StoredStyle) -> RenderStyle {
    match style {
        StoredStyle::Ripple => RenderStyle::Ripple,
        StoredStyle::BreathingBorder => RenderStyle::BreathingBorder,
        StoredStyle::CornerBrackets => RenderStyle::CornerBrackets,
        StoredStyle::CometSweep => RenderStyle::CometSweep,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinimapSource {
    PersistedSettings,
    Default,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinimapSelection {
    pub source: MinimapSource,
    pub minimap_scale: f64,
    pub flipped: bool,
}

impl MinimapSelection {
    pub fn settings(self) -> MinimapSettings {
        MinimapSettings {
            minimap_scale: self.minimap_scale,
            flipped: self.flipped,
        }
    }

    pub fn resolve(locator: &SettingsLocator) -> Result<Self, String> {
        for file in locator
            .candidates()
            .into_iter()
            .filter(|file| file.format == SettingsFormat::PersistedJson)
        {
            let text = match file.read() {
                Ok(text) => text,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("Cannot read {}: {error}", file.path.display())),
            };
            let patch = parse_persisted_settings(&text)
                .map_err(|error| format!("Invalid {}: {error}", file.path.display()))?;
            return Ok(Self {
                source: MinimapSource::PersistedSettings,
                minimap_scale: patch
                    .minimap_scale
                    .ok_or("PersistedSettings.json is missing MinimapScale")?,
                flipped: patch
                    .flipped
                    .ok_or("PersistedSettings.json is missing FlipMiniMap")?,
            });
        }
        let settings = MinimapSettings::default();
        Ok(Self {
            source: MinimapSource::Default,
            minimap_scale: settings.minimap_scale,
            flipped: settings.flipped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn stored_styles_map_to_the_matching_renderer_styles() {
        for rendered in [
            RenderStyle::Ripple,
            RenderStyle::BreathingBorder,
            RenderStyle::CornerBrackets,
            RenderStyle::CometSweep,
        ] {
            let stored = match rendered {
                RenderStyle::Ripple => StoredStyle::Ripple,
                RenderStyle::BreathingBorder => StoredStyle::BreathingBorder,
                RenderStyle::CornerBrackets => StoredStyle::CornerBrackets,
                RenderStyle::CometSweep => StoredStyle::CometSweep,
            };
            assert_eq!(pulse_style(stored), rendered);
        }
    }

    #[test]
    fn reads_persisted_values_and_defaults_only_when_absent() {
        let root = tempfile::tempdir().unwrap();
        let locator = SettingsLocator::new([root.path().to_owned()]);
        let fallback = MinimapSelection::resolve(&locator).unwrap();
        assert_eq!(fallback.source, MinimapSource::Default);
        assert_eq!(fallback.settings(), MinimapSettings::default());
        fs::create_dir(root.path().join("Config")).unwrap();
        let path = root.path().join("Config/PersistedSettings.json");
        fs::write(&path, r#"{"files":[{"name":"game.cfg","sections":[{"name":"HUD","settings":[{"name":"MinimapScale","value":"3"},{"name":"FlipMiniMap","value":"1"}]}]}]}"#).unwrap();
        let resolved = MinimapSelection::resolve(&locator).unwrap();
        assert_eq!(resolved.source, MinimapSource::PersistedSettings);
        assert_eq!(
            resolved.settings(),
            MinimapSettings {
                minimap_scale: 3.0,
                flipped: true
            }
        );
        for invalid in ["not json", r#"{"files":[]}"#] {
            fs::write(&path, invalid).unwrap();
            assert!(MinimapSelection::resolve(&locator).is_err());
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(MinimapSelection::resolve(&locator).is_err());
    }
}

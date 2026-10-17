//! Read a complete HUD profile from one installation only; never silently claim
//! guessed defaults are measured settings. Re-read periodically for HUD changes.
use lucas_game::{SettingsFormat, SettingsLocator};
use lucas_geometry::{MinimapSettings, SettingsPatch, parse_game_cfg, parse_persisted_settings};
use std::io;

pub fn read_hud(locator: &SettingsLocator) -> io::Result<MinimapSettings> {
    let candidates = locator.candidates();
    for root in candidates.chunks(2) {
        let mut combined = SettingsPatch::default();
        for file in root {
            let Ok(text) = file.read() else {
                continue;
            };
            let patch = match file.format {
                SettingsFormat::PersistedJson => parse_persisted_settings(&text),
                SettingsFormat::GameCfg => parse_game_cfg(&text),
            };
            if let Ok(patch) = patch {
                combined = combined.with_fallback(patch);
            }
        }
        if let (Some(minimap_scale), Some(flipped)) = (combined.minimap_scale, combined.flipped) {
            return Ok(MinimapSettings {
                minimap_scale,
                flipped,
            });
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Cannot read League HUD settings. Set LUCAS_LEAGUE_ROOT to the install folder and restart.",
    ))
}

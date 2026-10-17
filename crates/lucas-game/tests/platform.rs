use lucas_game::{
    DisplayMode, FakeWindowProbe, ScreenRect, SettingsFormat, SettingsLocator, WindowProbe,
    WindowStatus,
};
use std::fs;

#[test]
fn locator_preserves_root_priority_and_file_fallback_without_parsing_hud() {
    let temp = tempfile::tempdir().unwrap();
    let install = temp.path().join("Custom Install");
    fs::create_dir_all(install.join("Config")).unwrap();
    let persisted = r#"{"files":[]}"#;
    let cfg = "[HUD]\nMinimapScale=1.5\nFlipMiniMap=1\n";
    fs::write(install.join("Config/PersistedSettings.json"), persisted).unwrap();
    fs::write(install.join("Config/game.cfg"), cfg).unwrap();
    let locator =
        SettingsLocator::new([install.clone(), install.clone(), temp.path().join("Other")]);
    let candidates = locator.candidates();
    assert_eq!(candidates.len(), 4);
    assert_eq!(candidates[0].format, SettingsFormat::PersistedJson);
    assert_eq!(candidates[0].read().unwrap(), persisted);
    assert_eq!(candidates[1].format, SettingsFormat::GameCfg);
    assert_eq!(candidates[1].read().unwrap(), cfg);
    fs::remove_file(&candidates[0].path).unwrap();
    assert!(candidates[0].read().is_err());
    assert_eq!(candidates.iter().find_map(|c| c.read().ok()).unwrap(), cfg);
}

#[test]
fn installation_manifest_adds_only_league_roots_and_preserves_explicit_override() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("RiotClientInstalls.json");
    let install = temp.path().join("Custom League Install");
    fs::create_dir_all(install.join("Config")).unwrap();
    fs::write(install.join("LeagueClient.exe"), []).unwrap();
    let cfg = "[HUD]\nMinimapScale=1.5\n";
    fs::write(install.join("Config/game.cfg"), cfg).unwrap();
    let other = temp.path().join("VALORANT");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("VALORANT.exe"), []).unwrap();
    let install_key = install.to_string_lossy().replace('/', "\\");
    let other_key = other.to_string_lossy().into_owned();
    let missing_key = temp.path().join("Missing").to_string_lossy().into_owned();
    let contents = serde_json::json!({
        "associated_client": {
            install_key: "D:/Riot/RiotClientServices.exe",
            other_key: "D:/Riot/RiotClientServices.exe",
            missing_key: "D:/Riot/RiotClientServices.exe"
        },
        "rc_live": "D:/Riot/RiotClientServices.exe"
    });
    fs::write(&manifest, contents.to_string()).unwrap();
    let explicit = temp.path().join("Explicit");
    let mut locator = SettingsLocator::new([explicit.clone()]);
    locator.add_install_manifest(&manifest).unwrap();
    let paths = locator.candidates();
    assert_eq!(paths.len(), 4);
    assert_eq!(
        paths[0].path,
        explicit.join("Config/PersistedSettings.json")
    );
    assert_eq!(paths[2].path, install.join("Config/PersistedSettings.json"));
    let discovered = paths.iter().find_map(|c| c.read().ok()).unwrap();
    assert_eq!(discovered, cfg);
    println!(
        "EVIDENCE {}",
        serde_json::json!({
            "scenario": "custom installation settings discovery",
            "manifest_format": "associated_client keyed by installation directories",
            "explicit_override_first": paths[0].path == explicit.join("Config/PersistedSettings.json"),
            "discovered_custom_install": paths[2].path == install.join("Config/PersistedSettings.json"),
            "persisted_settings_missing": paths[2].read().is_err(),
            "fallback_file": "Config/game.cfg",
            "read_settings": discovered,
            "scope": "filesystem discovery and reading; HUD parsing belongs to lucas-geometry"
        })
    );
    locator.add_install_manifest(&manifest).unwrap();
    assert_eq!(locator.candidates(), paths);
    fs::write(&manifest, "not json").unwrap();
    assert!(locator.add_install_manifest(&manifest).is_err());
    assert_eq!(locator.candidates(), paths);
}

#[test]
fn fake_window_probe_covers_foreground_minimized_and_exclusive_gates() {
    let mut probe = FakeWindowProbe::default();
    assert!(!probe.sample().unwrap().overlay_eligible());
    probe.status = WindowStatus {
        exists: true,
        foreground: true,
        minimized: false,
        client_rect: Some(ScreenRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        }),
        display_mode: DisplayMode::Borderless,
    };
    assert!(probe.sample().unwrap().overlay_eligible());
    probe.status.foreground = false;
    assert!(!probe.sample().unwrap().overlay_eligible());
    probe.status.foreground = true;
    for mode in [DisplayMode::ExclusiveFullscreen, DisplayMode::Unknown] {
        probe.status.display_mode = mode;
        assert!(!probe.sample().unwrap().overlay_eligible());
    }
    probe.status.display_mode = DisplayMode::Windowed;
    assert!(probe.sample().unwrap().overlay_eligible());
    probe.status.minimized = true;
    assert!(!probe.sample().unwrap().overlay_eligible());
    probe.status.minimized = false;
    probe.status.client_rect = None;
    assert!(!probe.sample().unwrap().overlay_eligible());
}

#[cfg(windows)]
#[test]
fn win32_probe_without_requiring_a_running_game() {
    let status = lucas_game::Win32WindowProbe.sample().unwrap();
    if !status.exists {
        assert!(!status.overlay_eligible());
    }
}

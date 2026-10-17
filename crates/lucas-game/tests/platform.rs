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
    fs::write(
        &manifest,
        r#"{"associated_client":{
        "D:\\Games\\League of Legends\\LeagueClient.exe":"D:/Riot/RiotClientServices.exe",
        "D:/Games/VALORANT/VALORANT.exe":"D:/Riot/RiotClientServices.exe"
    },"rc_live":"D:/Riot/RiotClientServices.exe"}"#,
    )
    .unwrap();
    let explicit = temp.path().join("Explicit");
    let mut locator = SettingsLocator::new([explicit.clone()]);
    locator.add_install_manifest(&manifest).unwrap();
    let paths = locator.candidates();
    assert_eq!(paths.len(), 4);
    assert_eq!(
        paths[0].path,
        explicit.join("Config/PersistedSettings.json")
    );
    assert_eq!(
        paths[2].path,
        std::path::PathBuf::from("D:/Games/League of Legends/Config/PersistedSettings.json")
    );
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

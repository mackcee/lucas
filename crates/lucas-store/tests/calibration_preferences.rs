use lucas_store::{CalibrationPreferences, ScreenSize, Settings, Store};

#[test]
fn diagonal_sizes_follow_monitor_resolution_and_round_to_whole_mm() {
    for (size, resolution, expected) in [
        (ScreenSize::Inches24, [1920, 1080], [531, 299]),
        (ScreenSize::Inches27, [3840, 2160], [598, 336]),
        (ScreenSize::Inches27, [1920, 1080], [598, 336]),
        (ScreenSize::Inches32, [2560, 1440], [708, 398]),
        (ScreenSize::Inches34, [3440, 1440], [797, 333]),
        (ScreenSize::Inches27, [1080, 1920], [336, 598]),
    ] {
        let preferences = CalibrationPreferences {
            screen_size: size,
            ..Default::default()
        };
        assert_eq!(preferences.physical_size_mm(resolution).unwrap(), expected);
    }
    assert!(
        CalibrationPreferences::default()
            .physical_size_mm([0, 1080])
            .is_err()
    );
    assert!(
        CalibrationPreferences::default()
            .physical_size_mm([1920, 0])
            .is_err()
    );
}

#[test]
fn old_and_partial_settings_keep_the_existing_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    for text in ["camera_index = 2\n", "[calibration]\n"] {
        std::fs::write(dir.path().join("settings.toml"), text).unwrap();
        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.calibration, CalibrationPreferences::default());
        assert_eq!(
            loaded.calibration.physical_size_mm([1920, 1080]).unwrap(),
            [531, 299]
        );
        assert_eq!(loaded.calibration.viewing_distance_mm, 650);
    }
    std::fs::write(
        dir.path().join("settings.toml"),
        "[calibration]\nscreen_size = \"inches27\"\n",
    )
    .unwrap();
    let loaded = store.load_settings().unwrap();
    assert_eq!(loaded.calibration.screen_size, ScreenSize::Inches27);
    assert_eq!(loaded.calibration.viewing_distance_mm, 650);
}

#[test]
fn screen_choices_custom_dimensions_and_distance_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let settings = Settings {
        camera_index: 4,
        level_index: 3,
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    for size in [
        ScreenSize::Inches24,
        ScreenSize::Inches27,
        ScreenSize::Inches32,
        ScreenSize::Inches34,
        ScreenSize::Custom,
    ] {
        let preferences = CalibrationPreferences {
            screen_size: size,
            custom_size_mm: [597, 336],
            viewing_distance_mm: 725,
        };
        store.save_calibration_preferences(preferences).unwrap();
        let loaded = Store::open_in(dir.path()).unwrap().load_settings().unwrap();
        assert_eq!(loaded.calibration, preferences);
        assert_eq!(loaded.camera_index, settings.camera_index);
        assert_eq!(loaded.level_index, settings.level_index);
        assert!(!loaded.setup_completed);
        if size == ScreenSize::Custom {
            assert_eq!(
                loaded.calibration.physical_size_mm([3440, 1440]).unwrap(),
                [597, 336]
            );
        }
    }
    assert!(store.load_calibration().unwrap().is_none());
}

#[test]
fn invalid_preferences_fail_without_overwriting_saved_settings() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    for preferences in [
        CalibrationPreferences {
            custom_size_mm: [0, 299],
            ..Default::default()
        },
        CalibrationPreferences {
            viewing_distance_mm: 3001,
            ..Default::default()
        },
    ] {
        assert!(store.save_calibration_preferences(preferences).is_err());
        assert_eq!(store.load_settings().unwrap(), Settings::default());
    }
    let invalid = "[calibration]\nscreen_size = \"unknown\"\n";
    std::fs::write(dir.path().join("settings.toml"), invalid).unwrap();
    assert!(
        store
            .save_calibration_preferences(CalibrationPreferences::default())
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("settings.toml")).unwrap(),
        invalid
    );
}

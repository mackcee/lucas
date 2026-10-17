use lucas_geometry::{
    Alignment, MinimapSettings, Rect, SettingsError, SettingsPatch, minimap_rect, parse_game_cfg,
    parse_persisted_settings,
};

fn persisted(settings: &str) -> String {
    format!(
        r#"{{"files":[{{"name":"game.cfg","sections":[{{"name":"HUD","settings":[{settings}]}}]}}]}}"#
    )
}

#[test]
fn synthetic_files_prefer_persisted_and_ignore_other_files_sections_and_global_scale() {
    let preferred =
        parse_persisted_settings(include_str!("fixtures/PersistedSettings.json")).unwrap();
    let fallback = parse_game_cfg(include_str!("fixtures/game.cfg")).unwrap();
    assert_eq!(
        preferred,
        SettingsPatch {
            minimap_scale: Some(1.5),
            flipped: Some(true)
        }
    );
    assert_eq!(
        fallback,
        SettingsPatch {
            minimap_scale: Some(2.0),
            flipped: Some(false)
        }
    );
    let settings = preferred
        .with_fallback(fallback)
        .resolve(MinimapSettings::default());
    assert_eq!(
        settings,
        MinimapSettings {
            minimap_scale: 1.5,
            flipped: true
        }
    );
    // Config Width/Height do not influence the physical client rectangle.
    let map = minimap_rect(
        Rect::new(0.0, 0.0, 3840.0, 2160.0).unwrap(),
        settings,
        Alignment::default(),
    )
    .unwrap();
    assert!((map.width() - 604.152).abs() < 1e-8);
}

#[test]
fn missing_fields_are_preserved_and_fallback_is_per_field() {
    let preferred =
        parse_persisted_settings(&persisted(r#"{"name":"MinimapScale","value":"0"}"#)).unwrap();
    let fallback = parse_game_cfg("[HUD]\nMinimapScale=2\nFlipMiniMap=1").unwrap();
    assert_eq!(preferred.flipped, None);
    assert_eq!(
        preferred
            .with_fallback(fallback)
            .resolve(MinimapSettings::default()),
        MinimapSettings {
            minimap_scale: 0.0,
            flipped: true
        }
    );
    for empty in [
        r#"{"files":[]}"#,
        r#"{"files":[{"name":"input.ini"}]}"#,
        r#"{"files":[{"name":"game.cfg","sections":[]}]}"#,
        &persisted(""),
    ] {
        assert_eq!(
            parse_persisted_settings(empty).unwrap(),
            SettingsPatch::default()
        );
    }
    assert_eq!(
        parse_game_cfg("[General]\nMinimapScale=3").unwrap(),
        SettingsPatch::default()
    );
    assert_eq!(
        parse_game_cfg("")
            .unwrap()
            .resolve(MinimapSettings::default()),
        MinimapSettings::default()
    );
}

#[test]
fn cfg_tolerates_bom_crlf_comments_whitespace_and_case() {
    let result = parse_game_cfg("\u{feff}; comment\r\n [hUd] ; section\r\n minimapscale = 2.5 # size\r\n FLIPMINIMAP = true ; corner\r\nUnknown=anything\r\n[Other]\r\nMinimapScale=bad\r\n").unwrap();
    assert_eq!(
        result,
        SettingsPatch {
            minimap_scale: Some(2.5),
            flipped: Some(true)
        }
    );
    assert_eq!(
        parse_game_cfg("[HUD]\nFlipMiniMap=FALSE").unwrap().flipped,
        Some(false)
    );
}

#[test]
fn json_accepts_numeric_and_boolean_scalars_and_bom() {
    let text =
        persisted(r#"{"name":"MinimapScale","value":3},{"name":"FlipMiniMap","value":true}"#);
    assert_eq!(
        parse_persisted_settings(&format!("\u{feff}{text}")).unwrap(),
        SettingsPatch {
            minimap_scale: Some(3.0),
            flipped: Some(true)
        }
    );
    for (value, expected) in [
        ("0", false),
        ("1", true),
        ("false", false),
        ("\"TRUE\"", true),
    ] {
        assert_eq!(
            parse_persisted_settings(&persisted(&format!(
                r#"{{"name":"FlipMiniMap","value":{value}}}"#
            )))
            .unwrap()
            .flipped,
            Some(expected)
        );
    }
}

#[test]
fn invalid_values_are_not_silently_defaulted_or_clamped() {
    for value in ["-1", "3.001", "100", "NaN", "inf", "1e999", "", "bogus"] {
        assert!(matches!(
            parse_game_cfg(&format!("[HUD]\nMinimapScale={value}")),
            Err(SettingsError::InvalidValue("MinimapScale"))
        ));
        assert!(matches!(
            parse_persisted_settings(&persisted(&format!(
                r#"{{"name":"MinimapScale","value":"{value}"}}"#
            ))),
            Err(SettingsError::InvalidValue("MinimapScale"))
        ));
    }
    for value in ["2", "-1", "yes", "", "1.5"] {
        assert!(matches!(
            parse_game_cfg(&format!("[HUD]\nFlipMiniMap={value}")),
            Err(SettingsError::InvalidValue("FlipMiniMap"))
        ));
    }
    for value in ["null", "[]", "{}"] {
        assert!(matches!(
            parse_persisted_settings(&persisted(&format!(
                r#"{{"name":"MinimapScale","value":{value}}}"#
            ))),
            Err(SettingsError::Structure(_))
        ));
    }
    assert!(parse_persisted_settings(&persisted(r#"{"name":"FlipMiniMap"}"#)).is_err());
}

#[test]
fn malformed_structure_and_relevant_lines_fail() {
    for text in [
        "",
        "{",
        "[]",
        "{}",
        r#"{"files":{}}"#,
        r#"{"files":[{"name":"game.cfg"}]}"#,
        r#"{"files":[{"name":"game.cfg","sections":[{"name":"HUD"}]}]}"#,
    ] {
        assert!(parse_persisted_settings(text).is_err(), "{text}");
    }
    for text in [
        "[HUD",
        "[HUD] junk",
        "[HUD]\nMinimapScale",
        "[HUD]\nFlipMiniMap 1",
    ] {
        assert!(parse_game_cfg(text).is_err(), "{text}");
    }
}

#[test]
fn duplicate_relevant_settings_fail_even_across_hud_sections() {
    for (key, value) in [("MinimapScale", "1"), ("FlipMiniMap", "0")] {
        assert!(matches!(
            parse_game_cfg(&format!("[HUD]\n{key}={value}\n[HUD]\n{key}={value}")),
            Err(SettingsError::Duplicate(_))
        ));
        let entry = format!(r#"{{"name":"{key}","value":"{value}"}}"#);
        assert!(matches!(
            parse_persisted_settings(&persisted(&format!("{entry},{entry}"))),
            Err(SettingsError::Duplicate(_))
        ));
    }
}

#[test]
fn errors_expose_json_source_without_suppressing_failure() {
    use std::error::Error;
    let json_error = parse_persisted_settings("{").unwrap_err();
    assert!(json_error.source().is_some());
    assert!(
        json_error
            .to_string()
            .starts_with("invalid PersistedSettings JSON:")
    );
    let value_error = parse_game_cfg("[HUD]\nMinimapScale=bad").unwrap_err();
    assert!(value_error.source().is_none());
    assert_eq!(value_error.to_string(), "invalid HUD MinimapScale value");
}

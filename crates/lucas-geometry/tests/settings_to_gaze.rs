//! Consumer-level checks: supplied League settings -> physical map -> gaze hit region.
//! Fixtures are synthetic; these outputs do not establish live-game HUD alignment.
use lucas_geometry::{
    Alignment, MinimapSettings, Rect, ViewingGeometry, gaze_zone, minimap_rect, parse_game_cfg,
    parse_persisted_settings,
};
use serde_json::json;

fn edges(rect: Rect) -> [f64; 4] {
    [rect.left(), rect.top(), rect.right(), rect.bottom()]
}

#[test]
fn supplied_settings_drive_map_placement_and_gaze_hits() {
    let persisted =
        parse_persisted_settings(include_str!("fixtures/PersistedSettings.json")).unwrap();
    let cfg = parse_game_cfg(include_str!("fixtures/game.cfg")).unwrap();
    let partial = parse_persisted_settings(
        r#"{"files":[{"name":"game.cfg","sections":[{"name":"HUD","settings":[{"name":"MinimapScale","value":"0"}]}]}]}"#,
    )
    .unwrap();
    let flipped_cfg = parse_game_cfg("[HUD]\nMinimapScale=2\nFlipMiniMap=1").unwrap();
    let invalid = parse_persisted_settings(
        r#"{"files":[{"name":"game.cfg","sections":[{"name":"HUD","settings":[{"name":"MinimapScale","value":"100"}]}]}]}"#,
    )
    .unwrap_err();
    assert_eq!(invalid.to_string(), "invalid HUD MinimapScale value");

    let scenarios = [
        (
            "persisted wins; flipped 4K on negative-origin monitor with alignment",
            persisted
                .with_fallback(cfg)
                .resolve(MinimapSettings::default()),
            Rect::new(-3840.0, -200.0, 0.0, 1960.0).unwrap(),
            Alignment {
                offset_x_px: -7.0,
                offset_y_px: 3.0,
                scale_multiplier: 1.1,
            },
            [-3832.024, 1283.5288, -3167.4568, 1948.096],
            [-3840.0, 1219.5288, -3103.4568, 1960.0],
        ),
        (
            "persisted zero retained; missing flip filled from cfg; shifted 1080p client",
            partial
                .with_fallback(flipped_cfg)
                .resolve(MinimapSettings::default()),
            Rect::new(200.0, 100.0, 2120.0, 1180.0).unwrap(),
            Alignment::default(),
            [207.488, 994.240, 385.796, 1172.548],
            [200.0, 962.240, 417.796, 1180.0],
        ),
        (
            "invalid persisted scale reported; caller explicitly chooses cfg",
            cfg.resolve(MinimapSettings::default()),
            Rect::new(0.0, 0.0, 1920.0, 1080.0).unwrap(),
            Alignment::default(),
            [1569.180, 729.216, 1912.512, 1072.548],
            [1537.180, 697.216, 1920.0, 1080.0],
        ),
    ];

    for (name, settings, client, alignment, expected_map, expected_zone) in scenarios {
        let map = minimap_rect(client, settings, alignment).unwrap();
        let zone = gaze_zone(
            map,
            ViewingGeometry {
                bounds: client,
                width_mm: 600.0,
                height_mm: 337.5,
                distance_mm: 650.0,
            },
            (1.0_f64 / 65.0).atan().to_degrees(),
        )
        .unwrap()
        .unwrap();
        for (actual, expected) in edges(map)
            .into_iter()
            .zip(expected_map)
            .chain(edges(zone).into_iter().zip(expected_zone))
        {
            assert!(
                (actual - expected).abs() < 1e-8,
                "{name}: {actual} != {expected}"
            );
        }
        let center = [
            (map.left() + map.right()) / 2.0,
            (map.top() + map.bottom()) / 2.0,
        ];
        let samples = [
            ("map center", center, true),
            (
                "just above map within gaze tolerance",
                [center[0], map.top() - 16.0],
                true,
            ),
            (
                "gameplay center",
                [
                    (client.left() + client.right()) / 2.0,
                    (client.top() + client.bottom()) / 2.0,
                ],
                false,
            ),
            ("outside monitor", [client.left() - 1.0, center[1]], false),
            ("excluded right boundary", [zone.right(), center[1]], false),
        ];
        let hits: Vec<_> = samples
            .into_iter()
            .map(|(label, point, expected)| {
                let actual = zone.contains(point[0], point[1]);
                assert_eq!(actual, expected, "{name}: {label}");
                json!({"sample": label, "point_px": point, "inside_gaze_zone": actual})
            })
            .collect();
        println!(
            "GEOMETRY_EVIDENCE {}",
            json!({
                "scenario": name,
                "provenance": "synthetic settings and physical client coordinates; not a live League capture",
                "settings": {"minimap_scale": settings.minimap_scale, "flipped": settings.flipped},
                "client_edges_px": edges(client),
                "map_edges_px": edges(map),
                "gaze_zone_edges_px": edges(zone),
                "gaze_samples": hits,
                "invalid_persisted_error": invalid.to_string(),
            })
        );
    }
}

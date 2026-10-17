use lucas_geometry::{Alignment, MinimapSettings, Rect, ViewingGeometry, gaze_zone, minimap_rect};

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

fn screen() -> Rect {
    Rect::new(0.0, 0.0, 3840.0, 2160.0).unwrap()
}

fn viewing() -> ViewingGeometry {
    // Near a 27-inch 16:9 monitor, dimensions rounded for an exact test pitch.
    ViewingGeometry {
        bounds: screen(),
        width_mm: 600.0,
        height_mm: 337.5,
        distance_mm: 650.0,
    }
}

#[test]
fn seed_rectangle_fixtures() {
    for (index, line) in include_str!("fixtures/minimap.csv")
        .lines()
        .enumerate()
        .skip(1)
    {
        let columns: Vec<_> = line.split(',').collect();
        assert_eq!(columns.len(), 10, "fixture line {}", index + 1);
        let n = |i: usize| columns[i].parse::<f64>().unwrap();
        let client = Rect::new(n(0), n(1), n(0) + n(2), n(1) + n(3)).unwrap();
        let map = minimap_rect(
            client,
            MinimapSettings {
                minimap_scale: n(4),
                flipped: columns[5].parse().unwrap(),
            },
            Alignment::default(),
        )
        .unwrap();
        for (actual, expected) in [map.left(), map.top(), map.right(), map.bottom()]
            .into_iter()
            .zip((6..10).map(n))
        {
            assert!(
                (actual - expected).abs() < 1e-8,
                "fixture line {}: {actual} != {expected}",
                index + 1
            );
        }
        close(map.width(), map.height());
    }
}

#[test]
fn alignment_scales_about_outer_bottom_corner_then_translates() {
    for flipped in [false, true] {
        let settings = MinimapSettings {
            minimap_scale: 1.0,
            flipped,
        };
        let base = minimap_rect(screen(), settings, Alignment::default()).unwrap();
        for multiplier in [0.9, 1.1] {
            let adjusted = minimap_rect(
                screen(),
                settings,
                Alignment {
                    offset_x_px: -7.0,
                    offset_y_px: 3.0,
                    scale_multiplier: multiplier,
                },
            )
            .unwrap();
            close(adjusted.width(), base.width() * multiplier);
            close(adjusted.height(), base.height() * multiplier);
            close(adjusted.bottom(), base.bottom() + 3.0);
            if flipped {
                close(adjusted.left(), base.left() - 7.0);
            } else {
                close(adjusted.right(), base.right() - 7.0);
            }
        }
    }
}

#[test]
fn physical_4k_is_twice_1080p_regardless_of_windows_ui_scale() {
    let small = minimap_rect(
        Rect::new(0.0, 0.0, 1920.0, 1080.0).unwrap(),
        MinimapSettings::default(),
        Alignment::default(),
    )
    .unwrap();
    let large = minimap_rect(screen(), MinimapSettings::default(), Alignment::default()).unwrap();
    close(large.left(), small.left() * 2.0);
    close(large.top(), small.top() * 2.0);
    close(large.width(), small.width() * 2.0);
}

#[test]
fn inflation_converts_degrees_to_pixels_and_clips_both_corners() {
    for flipped in [false, true] {
        let map = minimap_rect(
            screen(),
            MinimapSettings {
                flipped,
                ..Default::default()
            },
            Alignment::default(),
        )
        .unwrap();
        // tan(atan(1/65)) * 650 mm = 10 mm = 64 pixels at this pitch.
        let margin = (1.0_f64 / 65.0).atan().to_degrees();
        let zone = gaze_zone(map, viewing(), margin).unwrap().unwrap();
        close(zone.top(), map.top() - 64.0);
        close(zone.bottom(), 2160.0);
        if flipped {
            close(zone.left(), 0.0);
            close(zone.right(), map.right() + 64.0);
        } else {
            close(zone.left(), map.left() - 64.0);
            close(zone.right(), 3840.0);
        }
        assert!(zone.contains(map.left(), map.top()));
        assert!(!zone.contains(-1.0, 2160.0));
        assert!(!zone.contains(zone.right(), zone.bottom()));
        assert!(!zone.contains(f64::NAN, zone.top()));
    }
}

#[test]
fn monitor_pitch_is_per_axis_and_origin_can_be_negative() {
    let monitor = Rect::new(-2000.0, -1000.0, 0.0, 0.0).unwrap();
    let map = Rect::new(-500.0, -400.0, -200.0, -100.0).unwrap();
    let view = ViewingGeometry {
        bounds: monitor,
        width_mm: 500.0,
        height_mm: 500.0,
        distance_mm: 500.0,
    };
    let zone = gaze_zone(map, view, 0.02_f64.atan().to_degrees())
        .unwrap()
        .unwrap();
    close(zone.left(), -540.0);
    close(zone.top(), -420.0);
    close(zone.right(), -160.0);
    close(zone.bottom(), -80.0);
}

#[test]
fn zero_margin_and_offscreen_intersections() {
    let map = minimap_rect(screen(), MinimapSettings::default(), Alignment::default()).unwrap();
    assert_eq!(gaze_zone(map, viewing(), 0.0).unwrap(), Some(map));
    let outside = Rect::new(-100.0, -100.0, 0.0, 0.0).unwrap();
    assert_eq!(gaze_zone(outside, viewing(), 0.0).unwrap(), None);
    let partial = Rect::new(-10.0, -10.0, 10.0, 10.0).unwrap();
    assert_eq!(
        gaze_zone(partial, viewing(), 0.0).unwrap(),
        Some(Rect::new(0.0, 0.0, 10.0, 10.0).unwrap())
    );
    assert_eq!(gaze_zone(map, viewing(), 89.0).unwrap(), Some(screen()));
}

#[test]
fn invalid_rectangles_settings_and_alignments_fail() {
    for edges in [
        [0.0, 0.0, 0.0, 10.0],
        [10.0, 0.0, 0.0, 10.0],
        [0.0, 0.0, 10.0, -1.0],
        [f64::NAN, 0.0, 10.0, 10.0],
        [0.0, 0.0, f64::INFINITY, 10.0],
        [-f64::MAX, 0.0, f64::MAX, 1.0],
    ] {
        assert!(Rect::new(edges[0], edges[1], edges[2], edges[3]).is_err());
    }
    for scale in [-0.01, 3.01, f64::NAN, f64::INFINITY] {
        assert!(
            minimap_rect(
                screen(),
                MinimapSettings {
                    minimap_scale: scale,
                    flipped: false
                },
                Alignment::default()
            )
            .is_err()
        );
    }
    for scale_multiplier in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(
            minimap_rect(
                screen(),
                MinimapSettings::default(),
                Alignment {
                    scale_multiplier,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for alignment in [
            Alignment {
                offset_x_px: invalid,
                ..Default::default()
            },
            Alignment {
                offset_y_px: invalid,
                ..Default::default()
            },
        ] {
            assert!(minimap_rect(screen(), MinimapSettings::default(), alignment).is_err());
        }
    }
}

#[test]
fn invalid_viewing_parameters_and_angles_fail() {
    for margin in [-0.1, 90.0, 180.0, f64::NAN, f64::INFINITY] {
        assert!(gaze_zone(screen(), viewing(), margin).is_err());
    }
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        for view in [
            ViewingGeometry {
                width_mm: bad,
                ..viewing()
            },
            ViewingGeometry {
                height_mm: bad,
                ..viewing()
            },
            ViewingGeometry {
                distance_mm: bad,
                ..viewing()
            },
        ] {
            assert!(gaze_zone(screen(), view, 2.0).is_err());
        }
    }
    assert!(
        gaze_zone(
            screen(),
            ViewingGeometry {
                distance_mm: f64::MAX,
                ..viewing()
            },
            89.0
        )
        .is_err()
    );
}

#[test]
fn seed_is_monotone_mirrored_and_inside_common_clients() {
    for (width, height) in [
        (1600.0, 1200.0),
        (1920.0, 1080.0),
        (1920.0, 1200.0),
        (2560.0, 1440.0),
        (3440.0, 1440.0),
        (3840.0, 2160.0),
    ] {
        let client = Rect::new(-width, 100.0, 0.0, height + 100.0).unwrap();
        let mut previous = 0.0;
        for step in 0..=30 {
            let settings = MinimapSettings {
                minimap_scale: f64::from(step) / 10.0,
                flipped: false,
            };
            let right = minimap_rect(client, settings, Alignment::default()).unwrap();
            let left = minimap_rect(
                client,
                MinimapSettings {
                    flipped: true,
                    ..settings
                },
                Alignment::default(),
            )
            .unwrap();
            assert!(right.width() > previous);
            previous = right.width();
            assert_eq!(right.intersection(client), Some(right));
            assert_eq!(left.intersection(client), Some(left));
            close(left.left() - client.left(), client.right() - right.right());
            close(left.width(), right.width());
            close(left.top(), right.top());
        }
    }
}

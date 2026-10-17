use lucas_overlay::{
    Colour, Pulse, PulseStyle, Rect, Salience,
    render::{self, Surface},
};
use std::{path::PathBuf, time::Duration};

fn target() -> Rect {
    Rect::new(-260.25, 600.75, -60.25, 800.75).unwrap()
}
fn surface() -> Surface {
    Surface::for_target(target()).unwrap()
}
const STYLES: [PulseStyle; 4] = [
    PulseStyle::Ripple,
    PulseStyle::BreathingBorder,
    PulseStyle::CornerBrackets,
    PulseStyle::CometSweep,
];
const SALIENCES: [Salience; 4] = [
    Salience::Minimal,
    Salience::Soft,
    Salience::Medium,
    Salience::Strong,
];

fn cases() -> Vec<(String, Pulse, Duration)> {
    let mut cases = Vec::new();
    for salience in SALIENCES {
        for millis in [180, 540, 960] {
            cases.push((
                format!("ripple-{salience:?}-{millis}.png").to_lowercase(),
                Pulse {
                    salience,
                    ..Pulse::default()
                },
                Duration::from_millis(millis),
            ));
        }
    }
    for style in STYLES.into_iter().skip(1) {
        for (name, colour) in [
            ("amber", Colour::AMBER),
            ("cyan", Colour::CYAN),
            ("white", Colour::WHITE),
        ] {
            cases.push((
                format!("{style:?}-{name}.png").to_lowercase(),
                Pulse {
                    style,
                    colour,
                    ..Pulse::default()
                },
                Duration::from_millis(360),
            ));
        }
    }
    cases
}

#[test]
fn golden_images() {
    for (name, pulse, time) in cases() {
        let image = render::frame(surface(), pulse, time).unwrap();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/goldens")
            .join(&name);
        let expected = tiny_skia::Pixmap::load_png(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            (image.width(), image.height()),
            (expected.width(), expected.height()),
            "{name}"
        );
        // PNG stores straight alpha, so compare encoded/decoded pixels, not a
        // premultiplied buffer with its lossy unpremultiply/repremultiply roundtrip.
        let actual = tiny_skia::Pixmap::decode_png(&image.encode_png().unwrap()).unwrap();
        assert!(actual.data() == expected.data(), "golden mismatch: {name}");
    }
}

/// Write candidates OUTSIDE the source tree; review before explicitly copying.
#[test]
#[ignore = "explicit golden candidate generation, requires LUCAS_GOLDEN_DIR"]
fn generate_golden_candidates() {
    let directory = PathBuf::from(
        std::env::var_os("LUCAS_GOLDEN_DIR").expect("set LUCAS_GOLDEN_DIR to a scratch directory"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    for (name, pulse, time) in cases() {
        render::frame(surface(), pulse, time)
            .unwrap()
            .save_png(directory.join(name))
            .unwrap();
    }
}

#[test]
fn transparent_content_and_edges_at_every_phase_for_every_style_and_salience() {
    let surface = surface();
    let margin = surface.margin();
    for style in STYLES {
        for salience in SALIENCES {
            for millis in (0..=1200).step_by(60) {
                let image = render::frame(
                    surface,
                    Pulse {
                        style,
                        salience,
                        ..Pulse::default()
                    },
                    Duration::from_millis(millis),
                )
                .unwrap();
                for y in 0..surface.height() {
                    for x in 0..surface.width() {
                        let pixel = image.pixel(x, y).unwrap();
                        assert!(
                            pixel.red() <= pixel.alpha()
                                && pixel.green() <= pixel.alpha()
                                && pixel.blue() <= pixel.alpha()
                        );
                        let in_map = x >= margin
                            && x < surface.width() - margin
                            && y >= margin
                            && y < surface.height() - margin;
                        let on_edge = x == 0
                            || y == 0
                            || x + 1 == surface.width()
                            || y + 1 == surface.height();
                        if in_map || on_edge {
                            assert_eq!(
                                pixel.alpha(),
                                0,
                                "{style:?}/{salience:?}/{millis} at {x},{y}"
                            );
                        }
                    }
                }
                if millis == 0 || millis == 1200 {
                    assert!(image.data().iter().all(|b| *b == 0));
                }
            }
        }
    }
}

#[test]
fn geometry_rounds_outward_handles_negative_desktops_and_bounds_memory() {
    let s = surface();
    assert_eq!(s.left() + s.margin() as i32, -261);
    assert_eq!(s.top() + s.margin() as i32, 600);
    assert_eq!(s.width() - s.margin() * 2, 201);
    assert_eq!(s.height() - s.margin() * 2, 201);
    for rect in [
        Rect::new(0.0, 0.0, 1e30, 1.0).unwrap(),
        Rect::new(-1e30, 0.0, 1.0, 1.0).unwrap(),
        Rect::new(i32::MAX as f64 - 2.0, 0.0, i32::MAX as f64, 1.0).unwrap(),
        Rect::new(i32::MIN as f64, 0.0, i32::MIN as f64 + 2.0, 1.0).unwrap(),
    ] {
        assert!(Surface::for_target(rect).is_err());
    }
    assert!(Surface::for_target(Rect::new(0.01, 0.01, 0.02, 0.02).unwrap()).is_ok());
}

#[test]
fn defaults_softening_motion_fade_and_bgra_contract() {
    let default = Pulse::default();
    assert_eq!(default.style, PulseStyle::Ripple);
    assert_eq!(default.colour, Colour::AMBER);
    assert_eq!(default.ripple_count(), 2);
    let soft = Pulse {
        salience: Salience::Soft,
        ..default
    };
    assert_eq!(soft.ripple_count(), 1);
    let sample =
        |pulse, millis| render::frame(surface(), pulse, Duration::from_millis(millis)).unwrap();
    let energy =
        |image: &tiny_skia::Pixmap| image.pixels().iter().map(|p| p.alpha() as u64).sum::<u64>();
    assert!(energy(&sample(soft, 180)) < energy(&sample(default, 180)));
    assert!(energy(&sample(soft, 960)) < energy(&sample(soft, 180)));
    let peak_x = |image: &tiny_skia::Pixmap| {
        (0..surface().margin())
            .max_by_key(|x| image.pixel(*x, surface().height() / 2).unwrap().alpha())
            .unwrap()
    };
    assert!(
        peak_x(&sample(soft, 720)) < peak_x(&sample(soft, 180)),
        "ring expands outward"
    );
    assert!(sample(soft, u64::MAX).data().iter().all(|b| *b == 0));
    let image = sample(default, 180);
    let mut bgra = vec![0; image.data().len()];
    render::copy_bgra(&image, &mut bgra);
    for (src, dst) in image.data().chunks_exact(4).zip(bgra.chunks_exact(4)) {
        assert_eq!(dst, &[src[2], src[1], src[0], src[3]]);
    }
}

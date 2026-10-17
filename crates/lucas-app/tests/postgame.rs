use lucas_app::postgame::{PostGameCard, demo_summary, next_criterion_text, wheels_text};
use lucas_core::TRAINING_LEVELS;

#[test]
fn demo_is_a_valid_summary_with_blind_seen_and_unknown() {
    let summary = demo_summary();
    summary.validate().unwrap();
    assert_eq!(
        (
            summary.metrics.blind_deaths,
            summary.metrics.seen_deaths,
            summary.metrics.unknown_deaths
        ),
        (1, 1, 1)
    );
    assert!(summary.metrics.compliance.unwrap() > 0.7);
    assert!(summary.metrics.pulses > 0);
}

#[test]
fn every_core_level_has_truthful_wheels_and_guidance() {
    for level in TRAINING_LEVELS {
        assert!(wheels_text(level).starts_with("Wheels: "));
        assert!(!next_criterion_text(level).is_empty());
    }
    assert_eq!(wheels_text(TRAINING_LEVELS[0]), "Wheels: off");
    assert_eq!(
        wheels_text(TRAINING_LEVELS[6]),
        "Wheels: 8 s · medium · 100%"
    );
    assert_eq!(
        wheels_text(TRAINING_LEVELS[8]),
        "Wheels: 15 s · minimal · 50%"
    );
}

#[test]
fn card_lays_out_at_normal_and_minimum_window_sizes() {
    for width in [480.0, 860.0] {
        let context = eframe::egui::Context::default();
        context.memory_mut(|memory| memory.set_everything_is_visible(true));
        let mut card = PostGameCard::demo();
        let mut output = context.run_ui(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(width, 880.0),
                )),
                ..Default::default()
            },
            |ui| {
                card.show(ui);
                assert!(
                    ui.min_rect().width() <= width + 1.0,
                    "card overflowed the viewport"
                );
            },
        );
        // This headless layout check deliberately has no texture renderer.
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| shape.clip_rect.is_finite())
        );
    }
}

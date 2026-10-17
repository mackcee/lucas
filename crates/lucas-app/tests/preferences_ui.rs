//! Exercise the shared selector through pointer events, then reopen saved choices.
use eframe::egui::{self, Event, PointerButton, Pos2, RawInput, Rect, Vec2};
use lucas_app::preferences::{level_label, level_picker, selectable_levels};
use lucas_store::{Settings, Store};

fn render(context: &egui::Context, index: &mut usize, events: Vec<Event>) -> egui::FullOutput {
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(560.0, 480.0))),
            events,
            ..Default::default()
        },
        |ui| level_picker(ui, index),
    );
    output.textures_delta.clear();
    output
}

fn position(output: &egui::FullOutput, label: &str) -> Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing rendered control: {label}"))
}

fn click(context: &egui::Context, index: &mut usize, label: &str) {
    let output = render(context, index, vec![]);
    let pos = position(&output, label);
    for pressed in [true, false] {
        render(
            context,
            index,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}

#[test]
fn shared_selector_clicks_each_ramp_fade_and_off_choice_and_persists_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open_in(directory.path()).unwrap();
    let mut settings = store.load_settings().unwrap();
    let context = egui::Context::default();
    let mut selections = Vec::new();
    render(&context, &mut settings.level_index, vec![]);
    for (expected, level) in selectable_levels() {
        let current = level_label(lucas_core::TRAINING_LEVELS[settings.level_index]);
        click(&context, &mut settings.level_index, &current);
        let opened = render(&context, &mut settings.level_index, vec![]);
        assert!(
            !opened.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::epaint::Shape::Text(text) if text.galley.text() == "Baseline (no pulses)"
            ))
        );
        let label = level_label(level);
        click(&context, &mut settings.level_index, &label);
        assert_eq!(settings.level_index, expected, "clicked {label}");
        let output = render(&context, &mut settings.level_index, vec![]);
        position(
            &output,
            &format!(
                "Goal: a minimap check every {} s. Changes apply next game.",
                level.standard_ms as f64 / 1000.0
            ),
        );
        store.save_settings(&settings).unwrap();
        let reopened = Store::open_in(directory.path())
            .unwrap()
            .load_settings()
            .unwrap();
        assert_eq!(reopened, settings);
        assert!(!reopened.setup_completed);
        selections.push(serde_json::json!({
            "clicked_label": label,
            "persisted_level": lucas_core::TRAINING_LEVELS[reopened.level_index],
            "setup_completed": reopened.setup_completed,
        }));
    }
    assert_ne!(settings.level_index, Settings::default().level_index);
    if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let directory = std::path::Path::new(&path);
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("selector-interactions.json"),
            serde_json::to_vec_pretty(&selections).unwrap(),
        )
        .unwrap();
    }
}

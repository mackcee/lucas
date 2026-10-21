//! Headless pointer interaction with the exact picker used by all calibration entry points.
use eframe::egui::{self, Event, PointerButton, Pos2, RawInput, Rect, Vec2};
use lucas_app::preferences::calibration_picker;
use lucas_store::{CalibrationPreferences, ScreenSize, Store};

fn render(
    context: &egui::Context,
    preferences: &mut CalibrationPreferences,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(560.0, 480.0))),
            events,
            ..Default::default()
        },
        |ui| calibration_picker(ui, preferences, [3840, 2160]),
    );
    output.textures_delta.clear();
    output
}
fn position(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.text() == label => {
            Some(text.pos + text.galley.size() / 2.0)
        }
        _ => None,
    })
}
fn click(context: &egui::Context, preferences: &mut CalibrationPreferences, label: &str) {
    let output = render(context, preferences, vec![]);
    let pos = position(&output, label).unwrap_or_else(|| panic!("missing control: {label}"));
    for pressed in [true, false] {
        render(
            context,
            preferences,
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
fn picker_selects_presets_reveals_custom_and_reopens_saved_choice() {
    let context = egui::Context::default();
    let mut preferences = CalibrationPreferences::default();
    render(&context, &mut preferences, vec![]);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let mut stale_tray_settings = store.load_settings().unwrap();
    let mut current = "24 in";
    for (label, size) in [
        ("27 in", ScreenSize::Inches27),
        ("32 in", ScreenSize::Inches32),
        ("34 in", ScreenSize::Inches34),
        ("24 in", ScreenSize::Inches24),
        ("Custom", ScreenSize::Custom),
    ] {
        click(&context, &mut preferences, current);
        click(&context, &mut preferences, label);
        assert_eq!(preferences.screen_size, size);
        let output = render(&context, &mut preferences, vec![]);
        assert_eq!(
            position(&output, "Screen width (mm)").is_some(),
            size == ScreenSize::Custom
        );
        assert_eq!(
            position(&output, "Height (mm)").is_some(),
            size == ScreenSize::Custom
        );
        assert!(position(&output, "Assumed distance (mm)").is_some());
        if size == ScreenSize::Inches27 {
            assert!(
                position(
                    &output,
                    "598 × 336 mm · based on the main monitor's 3840 × 2160 resolution"
                )
                .is_some()
            );
        }
        if size == ScreenSize::Inches27 {
            store.save_calibration_preferences(preferences).unwrap();
            // A running tray still holds the defaults loaded before calibration.
            stale_tray_settings.level_index = 3;
            stale_tray_settings.live_client_gate = false;
            store.save_settings(&stale_tray_settings).unwrap();
            let mut reopened = Store::open_in(dir.path())
                .unwrap()
                .load_settings()
                .unwrap()
                .calibration;
            assert_eq!(reopened, preferences);
            let output = render(&egui::Context::default(), &mut reopened, vec![]);
            assert!(position(&output, "27 in").is_some());
            export_settings(dir.path(), "reopened-27-inch");
        }
        current = label;
    }
    preferences.custom_size_mm = [597, 336];
    preferences.viewing_distance_mm = 720;
    Store::open_in(dir.path())
        .unwrap()
        .save_calibration_preferences(preferences)
        .unwrap();
    let mut reopened = Store::open_in(dir.path())
        .unwrap()
        .load_settings()
        .unwrap()
        .calibration;
    assert_eq!(reopened, preferences);
    let output = render(&egui::Context::default(), &mut reopened, vec![]);
    assert!(position(&output, "Custom").is_some());
    assert!(position(&output, "Screen width (mm)").is_some());
    export_settings(dir.path(), "reopened-custom");
}

#[test]
fn picker_labels_have_renderable_glyphs() {
    let context = egui::Context::default();
    let mut preferences = CalibrationPreferences::default();
    let output = render(&context, &mut preferences, vec![]);
    for shape in &output.shapes {
        if let egui::epaint::Shape::Text(text) = &shape.shape {
            for section in &text.galley.job.sections {
                let label =
                    &text.galley.job.text[section.byte_range.start.0..section.byte_range.end.0];
                assert!(
                    context.fonts_mut(|fonts| fonts.has_glyphs(&section.format.font_id, label)),
                    "picker text renders a missing-glyph box: {label:?}"
                );
            }
        }
    }
}

// Export the actual persisted settings contract for optional native rendering.
fn export_settings(source: &std::path::Path, name: &str) {
    if let Some(root) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let destination = std::path::PathBuf::from(root).join(name);
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::copy(
            source.join("settings.toml"),
            destination.join("settings.toml"),
        )
        .unwrap();
    }
}

//! Click the production camera-rate control and reopen its persisted settings.
use eframe::egui::{self, Event, PointerButton, Pos2, RawInput, Rect, Vec2};
use lucas_app::preferences::camera_frame_rate_picker;
use lucas_store::{CameraFrameRate, Store};

fn render(
    context: &egui::Context,
    rate: &mut CameraFrameRate,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 240.0))),
            events,
            ..Default::default()
        },
        |ui| camera_frame_rate_picker(ui, rate),
    );
    // This interaction test does not have a renderer consuming texture uploads.
    output.textures_delta.clear();
    output
}

fn click(context: &egui::Context, rate: &mut CameraFrameRate, label: &str) {
    let output = render(context, rate, vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing rendered control: {label}"));
    for pressed in [true, false] {
        render(
            context,
            rate,
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
fn camera_rate_clicks_persist_and_reopen_without_changing_other_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open_in(directory.path()).unwrap();
    let mut settings = store.load_settings().unwrap();
    settings.camera_index = 2;
    settings.level_index = 3;
    let context = egui::Context::default();
    render(&context, &mut settings.camera_frame_rate, vec![]);
    assert_eq!(settings.camera_frame_rate, CameraFrameRate::Auto);
    let mut interactions = Vec::new();
    for (expected, name) in [
        (CameraFrameRate::Fps30, "30"),
        (CameraFrameRate::Auto, "auto"),
    ] {
        let current = settings.camera_frame_rate.label();
        click(&context, &mut settings.camera_frame_rate, current);
        click(&context, &mut settings.camera_frame_rate, expected.label());
        assert_eq!(settings.camera_frame_rate, expected);
        store.save_settings(&settings).unwrap();
        let reopened = Store::open_in(directory.path())
            .unwrap()
            .load_settings()
            .unwrap();
        assert_eq!(reopened, settings);
        assert_eq!(reopened.camera_index, 2);
        assert_eq!(reopened.level_index, 3);
        assert!(!reopened.setup_completed);
        interactions.push(serde_json::json!({
            "clicked_label": expected.label(),
            "reopened_settings": reopened,
        }));
        if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
            let evidence = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&evidence).unwrap();
            std::fs::copy(
                directory.path().join("settings.toml"),
                evidence.join(format!("camera-rate-{name}.settings.toml")),
            )
            .unwrap();
        }
    }
    if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        std::fs::write(
            std::path::Path::new(&path).join("camera-rate-interactions.json"),
            serde_json::to_vec_pretty(&interactions).unwrap(),
        )
        .unwrap();
    }
}

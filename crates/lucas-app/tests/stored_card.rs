use eframe::egui::{self, Event, PointerButton, Pos2, RawInput, Rect, Vec2};
use lucas_app::postgame::{PostGameCard, demo_summary};
use lucas_app::timer::alignment_store::{AlignmentStore, Key};
use lucas_core::{
    GateInputs, GazeSample, SeededRng, Session, SessionConfig, TRAINING_LEVELS, Timestamp,
};
use lucas_store::{Calibration, GameSummary, ScreenGeometry, Settings, Store};
use std::path::Path;

fn seed_store(path: &Path) -> GameSummary {
    let mut store = Store::open_in(path).unwrap();
    let summary = demo_summary();
    store.save_settings(&Settings::default()).unwrap();
    let mut alignments = AlignmentStore::default();
    alignments
        .set(
            Key {
                width: 1920,
                height: 1080,
                minimap_scale: 1.0,
                flipped: false,
            },
            lucas_geometry::Alignment {
                offset_x_px: -4.0,
                offset_y_px: -6.0,
                scale_multiplier: 1.1,
            },
        )
        .unwrap();
    alignments.save(path).unwrap();
    std::fs::copy(path.join("alignment.toml"), path.join("alignment.toml.tmp")).unwrap();
    store
        .save_calibration(&Calibration {
            schema_version: 1,
            model: vec![42; 1024],
            screen: ScreenGeometry {
                left: 0,
                top: 0,
                width: 1920,
                height: 1080,
                scale_factor: 1.0,
            },
            created_at_unix_ms: 1_799_000_000_000,
            validation_error_degrees: Some(2.1),
        })
        .unwrap();
    for (index, interval) in [12_000, 10_000, 8_000].into_iter().enumerate() {
        let mut session = Session::new(
            format!("synthetic-probe-{index}"),
            SessionConfig::default(),
            TRAINING_LEVELS[0],
            SeededRng::new(11),
        )
        .unwrap();
        for ms in (0..=120_000).step_by(50) {
            session
                .observe(
                    GazeSample {
                        at: Timestamp(ms),
                        p_minimap: f64::from(ms % interval < 400),
                        quality: 1.0,
                    },
                    GateInputs {
                        active_game: true,
                        alive: true,
                        game_time_ms: ms,
                        foreground: true,
                        face_tracked: true,
                        paused: false,
                    },
                )
                .unwrap();
        }
        let record = session.finish(Timestamp(120_000)).unwrap();
        let mut context = summary.context.clone();
        context.length_ms = 120_000;
        context.ended_at_unix_ms -= (3 - index as u64) * 1_000_000;
        context.probe_game = true;
        context.tracking.total_frames = 2401;
        context.tracking.valid_frames = 2401;
        let probe = GameSummary::from_session(&record, context, |at| Some(at.0)).unwrap();
        store.save_summary(&probe).unwrap();
    }
    store.save_summary(&summary).unwrap();
    summary
}

fn render(
    context: &egui::Context,
    card: &mut PostGameCard,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(860.0, 2400.0))),
            events,
            ..Default::default()
        },
        |ui| card.show(ui),
    );
    output.textures_delta.clear();
    output
}

fn text_position(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.text() == label => {
            Some(text.pos + text.galley.size() / 2.0)
        }
        _ => None,
    })
}

fn click(context: &egui::Context, card: &mut PostGameCard, label: &str) -> egui::FullOutput {
    let output = render(context, card, vec![]);
    let pos = text_position(&output, label)
        .unwrap_or_else(|| panic!("missing rendered control: {label}"));
    render(
        context,
        card,
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    render(
        context,
        card,
        vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    )
}

#[test]
fn persisted_card_requires_confirmation_and_clears_data_through_ui() {
    let directory = tempfile::tempdir().unwrap();
    let summary = seed_store(directory.path());
    let store = Store::open_in(directory.path()).unwrap();
    assert_eq!(store.latest_summary().unwrap(), Some(summary.clone()));
    assert_eq!(store.load_settings().unwrap(), Settings::default());
    assert_eq!(store.load_calibration().unwrap().unwrap().model.len(), 1024);
    assert_eq!(store.probe_trend().unwrap().len(), 3);
    assert_eq!(store.events(&summary.id).unwrap(), summary.events);
    assert_ne!(
        AlignmentStore::load(directory.path()).unwrap(),
        AlignmentStore::default()
    );

    // Optional reproducible, synthetic fixture for inspecting the native CLI UI.
    // No user directory is ever used implicitly.
    if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let fixture = Path::new(&path).join("synthetic-store");
        assert!(
            !fixture.exists(),
            "refusing to overwrite an evidence fixture"
        );
        seed_store(&fixture);
    }

    std::fs::write(directory.path().join("unrelated.txt"), "keep").unwrap();
    let mut card = PostGameCard::from_store(store, &summary.id).unwrap();
    let context = egui::Context::default();
    render(&context, &mut card, vec![]);
    let output = render(&context, &mut card, vec![]);
    for label in [
        "Lux · Support · 28:00 · Assisted game",
        "1 / 3 blind · 1 seen · 1 unknown",
        "Wheels: 8 s · medium · 100%",
        "Open data folder",
        "Delete all data…",
    ] {
        assert!(
            text_position(&output, label).is_some(),
            "missing rendered output: {label}"
        );
    }

    click(&context, &mut card, "Delete all data…");
    assert!(directory.path().join("games.sqlite3").exists());
    click(&context, &mut card, "Cancel");
    let output = render(&context, &mut card, vec![]);
    assert!(text_position(&output, "Permanently delete all data").is_none());
    assert!(directory.path().join("settings.toml").exists());
    assert!(directory.path().join("alignment.toml").exists());
    assert!(directory.path().join("alignment.toml.tmp").exists());
    click(&context, &mut card, "Delete all data…");
    click(&context, &mut card, "Permanently delete all data");
    let output = render(&context, &mut card, vec![]);
    assert!(
        text_position(
            &output,
            "Local data deleted. Settings and calibration will start fresh next time."
        )
        .is_some()
    );
    assert!(text_position(&output, "1 / 3 blind · 1 seen · 1 unknown").is_none());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(
        AlignmentStore::load(directory.path()).unwrap(),
        AlignmentStore::default()
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("unrelated.txt")).unwrap(),
        "keep"
    );
}

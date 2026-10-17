use lucas_core::{
    GateInputs, GazeSample, SeededRng, Session, SessionConfig, TRAINING_LEVELS, Timestamp,
};
use lucas_store::{GameContext, GameSummary, ProbePoint, TrackingQuality};

/// Synthetic input passes through the same core metrics and storage builder as
/// real sessions. Nothing is saved and no webcam or game API is opened.
pub fn demo_summary() -> GameSummary {
    let mut session = Session::new(
        "demo",
        SessionConfig::default(),
        TRAINING_LEVELS[6],
        SeededRng::new(11),
    )
    .expect("valid built-in level");
    let mut tracking = TrackingQuality {
        calibration_error_degrees: Some(2.1),
        ..Default::default()
    };
    for ms in (100_000..=1_680_000).step_by(50) {
        let tracked = !(1_190_000..1_201_000).contains(&ms);
        tracking.total_frames += 1;
        tracking.valid_frames += u64::from(tracked);
        let looking = ms % 6_000 < 400 && !(310_000..322_000).contains(&ms);
        session
            .observe(
                GazeSample {
                    at: Timestamp(ms),
                    p_minimap: f64::from(looking),
                    quality: f64::from(tracked),
                },
                GateInputs {
                    active_game: true,
                    alive: true,
                    game_time_ms: ms,
                    foreground: true,
                    face_tracked: tracked,
                    paused: false,
                },
            )
            .expect("ordered demo frames");
    }
    for ms in [320_000, 600_500, 1_200_000] {
        session
            .death(Timestamp(ms))
            .expect("demo death inside observation history");
    }
    let record = session
        .finish(Timestamp(1_680_000))
        .expect("ordered demo end");
    GameSummary::from_session(
        &record,
        GameContext {
            ended_at_unix_ms: 1_800_000_000_000,
            role: Some("Support".into()),
            champion: Some("Lux".into()),
            length_ms: 1_680_000,
            probe_game: false,
            deaths_available: true,
            tracking,
        },
        |at| Some(at.0),
    )
    .expect("consistent demo summary")
}

pub(super) fn demo_probes() -> Vec<ProbePoint> {
    [0.68, 0.72, 0.77]
        .into_iter()
        .enumerate()
        .map(|(index, compliance)| ProbePoint {
            game_id: format!("demo-probe-{index}"),
            ended_at_unix_ms: 1_799_000_000_000 + index as u64 * 100_000,
            compliance: Some(compliance),
            standard_ms: 5_000,
            standard_changed: false,
            valid_fraction: Some(0.96),
        })
        .collect()
}

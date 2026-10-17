//! Executable M1.1 acceptance traces through the public app/simulator boundary.
//! Run with --nocapture to expose storage payloads for reviewer evidence.
use lucas_core::*;

// Unit draw 0.25: a reproducible -5% jitter and a successful fade opportunity.
struct QuarterRng;
impl RandomSource for QuarterRng {
    fn next_u64(&mut self) -> u64 {
        1 << 62
    }
}

fn gate(at: u64) -> GateInputs {
    GateInputs {
        active_game: true,
        alive: true,
        game_time_ms: 91_000 + at,
        foreground: true,
        face_tracked: true,
        paused: false,
    }
}

fn sample(at: u64, looking: bool) -> GazeSample {
    GazeSample {
        at: Timestamp(at),
        p_minimap: if looking { 1.0 } else { 0.0 },
        quality: 1.0,
    }
}

fn assert_payload_equivalent(actual: &serde_json::Value, expected: &serde_json::Value) {
    use serde_json::Value;
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() && b.is_f64() => {
            // JSON decimal parsing can differ by one ULP for derived ratios.
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() <= 2.0 * f64::EPSILON * b.abs().max(1.0));
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_payload_equivalent(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (key, b) in b {
                assert_payload_equivalent(a.get(key).unwrap(), b);
            }
        }
        _ => assert_eq!(actual, expected),
    }
}

fn emit_storage_payload(record: SessionRecord) {
    let json = serde_json::to_string(&record).unwrap();
    let decoded: SessionRecord = serde_json::from_str(&json).unwrap();
    assert_payload_equivalent(
        &serde_json::to_value(decoded).unwrap(),
        &serde_json::to_value(record).unwrap(),
    );
    println!("SESSION_RECORD={json}");
}

#[test]
fn replay_each_programme_level_from_gaze_to_pulse_check_and_summary() {
    let expected_delays = [
        None,
        Some(9_500),
        Some(7_600),
        Some(5_700),
        Some(4_750),
        Some(5_700),
        Some(7_600),
        Some(9_500),
        Some(14_250),
        None,
    ];
    for (index, (level, expected_delay)) in
        TRAINING_LEVELS.into_iter().zip(expected_delays).enumerate()
    {
        let mut session = Session::new(
            format!("programme-level-{index}"),
            SessionConfig::default(),
            level,
            QuarterRng,
        )
        .unwrap();
        let check_start = expected_delay.unwrap_or(20_000) + 500;
        let end = check_start + 500;
        let mut pulses = Vec::new();
        let mut checks = Vec::new();
        for at in (0..=end).step_by(50) {
            let looking = (check_start..check_start + 300).contains(&at);
            let update = session.observe(sample(at, looking), gate(at)).unwrap();
            for event in update.events {
                match event.kind {
                    EventKind::Pulse(pulse) => {
                        assert!(update.eligible);
                        assert_ne!(update.gaze, GazeState::Minimap);
                        pulses.push(pulse);
                    }
                    EventKind::Check(check) => {
                        assert_eq!(update.gaze, GazeState::Minimap);
                        assert_eq!(update.gap_ms, 0);
                        checks.push(check);
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(pulses.len(), usize::from(expected_delay.is_some()));
        if let Some(delay) = expected_delay {
            assert_eq!(pulses[0].at, Timestamp(delay));
            assert_eq!(pulses[0].effective_delay_ms, delay);
            assert_eq!(pulses[0].salience, level.salience);
        }
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].at, Timestamp(check_start + 100));
        assert_eq!(checks[0].prompted, expected_delay.is_some());
        assert_eq!(checks[0].pulse_latency_ms, expected_delay.map(|_| 600));
        assert_eq!(
            session.death(Timestamp(end)).unwrap().classification,
            DeathClassification::Seen
        );
        let record = session.finish(Timestamp(end)).unwrap();
        assert_eq!(record.metrics.pulses, pulses.len() as u64);
        assert_eq!(record.metrics.checks, 1);
        assert_eq!(record.metrics.mean_dwell_ms, Some(300.0));
        assert_eq!(record.metrics.seen_deaths, 1);
        assert_eq!(
            record
                .events
                .iter()
                .filter(|e| matches!(e.kind, EventKind::GateChanged(_)))
                .count(),
            1
        );
        emit_storage_payload(record);
    }
}

#[test]
fn replay_death_evidence_and_delayed_delivery_into_storage_payloads() {
    for (name, end, death_at, paused, expected) in [
        (
            "short-history",
            5_000,
            5_000,
            false,
            DeathClassification::Unknown,
        ),
        (
            "exact-eight-second-gap",
            8_300,
            8_300,
            false,
            DeathClassification::Blind,
        ),
        (
            "paused-minimap-view",
            10_000,
            10_000,
            true,
            DeathClassification::Unknown,
        ),
        (
            "delayed-death-after-new-check",
            12_000,
            8_300,
            false,
            DeathClassification::Blind,
        ),
    ] {
        let mut session = Session::new(
            name,
            SessionConfig::default(),
            TRAINING_LEVELS[1],
            QuarterRng,
        )
        .unwrap();
        for at in (0..=end).step_by(50) {
            let looking =
                paused || (name != "short-history" && (at < 300 || (11_000..11_300).contains(&at)));
            session
                .observe(sample(at, looking), GateInputs { paused, ..gate(at) })
                .unwrap();
        }
        let death = session.death(Timestamp(death_at)).unwrap();
        assert_eq!(death.classification, expected);
        if expected == DeathClassification::Blind {
            assert_eq!(death.gap_ms, Some(8_000));
            assert_eq!(death.tracked_fraction, 1.0);
        }
        if paused {
            assert_eq!(death.tracked_fraction, 0.0);
            assert_eq!(session.metrics().pulses, 0);
            assert_eq!(session.metrics().checks, 0);
        }
        assert_eq!(session.death(Timestamp(death_at)).unwrap(), death);
        let record = session.finish(Timestamp(end)).unwrap();
        assert_eq!(
            record
                .events
                .iter()
                .filter(|e| matches!(e.kind, EventKind::Death(_)))
                .count(),
            1
        );
        emit_storage_payload(record);
    }
}

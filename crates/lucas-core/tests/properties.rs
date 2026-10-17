use lucas_core::*;
use proptest::prelude::*;

fn gate() -> GateInputs {
    GateInputs {
        active_game: true,
        alive: true,
        game_time_ms: 91_000,
        foreground: true,
        face_tracked: true,
        paused: false,
    }
}
fn observation(at: u64, looking: bool) -> MetricsObservation {
    MetricsObservation {
        at: Timestamp(at),
        gate: gate(),
        tracking: true,
        looking,
        check_started: looking,
        effective_delay_ms: Some(10_000),
    }
}
struct MidpointRng;
impl RandomSource for MidpointRng {
    fn next_u64(&mut self) -> u64 {
        1 << 63
    }
}

proptest! {
    // No regression artifacts in the repository; failures print reproducible seeds.
    #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn detector_never_checks_on_unknown_or_before_dwell(
        inputs in prop::collection::vec((1u64..300, 0u8..3), 1..300)
    ) {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        let mut at = 0;
        let mut candidate = None;
        let mut count = 0;
        let mut confirmed = false;
        for (dt, category) in inputs {
            at += dt;
            if dt > 250 || category != 2 {
                candidate = None;
                count = 0;
                confirmed = false;
            }
            if category == 2 {
                candidate.get_or_insert(at);
                count += 1;
            }
            let result = detector.observe(GazeSample { at: Timestamp(at), p_minimap: if category == 2 { 1.0 } else { 0.0 }, quality: if category == 0 { 0.0 } else { 1.0 } }).unwrap();
            if category == 0 { prop_assert_eq!(result.state, GazeState::Unknown); }
            if result.check_started {
                prop_assert_eq!(category, 2);
                prop_assert!(!confirmed);
                prop_assert!(count >= 3);
                prop_assert!(at - candidate.unwrap() >= 100);
                confirmed = true;
            }
            if let Some(glance) = result.completed_glance {
                prop_assert!(glance.started_at <= glance.confirmed_at);
                prop_assert!(glance.confirmed_at <= glance.ended_at);
                prop_assert_eq!(glance.dwell_ms, glance.ended_at.0 - glance.started_at.0);
            }
        }
    }

    #[test]
    fn gap_clock_matches_reference_state_machine(
        inputs in prop::collection::vec((0u64..10_000, any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()), 1..200)
    ) {
        let mut clock = GapClock::default();
        let mut at = 0;
        let mut reference_gap = 0;
        let mut previous_eligible = false;
        let mut previous_looking = false;
        let mut initialized = false;
        for (dt, alive, foreground, tracked, paused, looking) in inputs {
            at += dt;
            if initialized && previous_eligible && !previous_looking { reference_gap += dt; }
            let g = GateInputs { alive, foreground, face_tracked: tracked, paused, ..gate() };
            if !alive || (g.eligible() && looking) { reference_gap = 0; }
            clock.update(Timestamp(at), g, looking).unwrap();
            prop_assert_eq!(clock.gap_ms(), reference_gap);
            prop_assert_eq!(clock.eligible(), g.eligible());
            previous_eligible = g.eligible();
            previous_looking = looking && previous_eligible;
            initialized = true;
        }
    }

    #[test]
    fn staircase_matches_weighted_up_down_and_never_wraps(
        operations in prop::collection::vec(0u8..3, 1..500)
    ) {
        let mut policy = PulsePolicy::new(TRAINING_LEVELS[1], true).unwrap();
        let mut expected = 10_000u64;
        for (index, op) in operations.into_iter().enumerate() {
            match op {
                0 => {
                    policy.reset_gap();
                    let pulse = policy.tick(Timestamp(index as u64), u64::MAX, true, &mut MidpointRng).unwrap().unwrap();
                    prop_assert_eq!(pulse.effective_delay_ms, expected.max(10_000));
                    expected += 750;
                }
                1 => { policy.on_check(true); expected = expected.saturating_sub(250); }
                _ => policy.on_check(false),
            }
            prop_assert_eq!(policy.staircase_ms(), expected);
        }
    }

    #[test]
    fn jitter_is_bounded_and_seed_reproducible(seed in any::<u64>(), base in 100u64..100_000) {
        let level = TrainingLevel { pulse_delay_ms: Some(base), ..TRAINING_LEVELS[1] };
        let mut a = PulsePolicy::new(level, false).unwrap();
        let mut b = PulsePolicy::new(level, false).unwrap();
        let mut rng_a = SeededRng::new(seed);
        let mut rng_b = SeededRng::new(seed);
        for _ in 0..20 {
            let delay = a.prepare_gap(&mut rng_a).unwrap();
            prop_assert!((delay as f64) >= base as f64 * 0.9 - 0.5);
            prop_assert!((delay as f64) <= base as f64 * 1.1 + 0.5);
            prop_assert_eq!(Some(delay), b.prepare_gap(&mut rng_b));
            a.reset_gap(); b.reset_gap();
        }
    }

    #[test]
    fn metrics_match_closed_form_for_arbitrary_gap_sequences(
        standard in 1u64..20_000,
        gaps in prop::collection::vec(1u64..50_000, 1..100)
    ) {
        let mut m = MetricsAccumulator::new(standard, MetricsConfig::default()).unwrap();
        let mut at = 0;
        let mut compliant = 0;
        let mut blind_spells = 0;
        m.observe(observation(at, false)).unwrap();
        for gap in &gaps {
            at += gap;
            compliant += gap.min(&standard);
            blind_spells += u64::from(*gap > 15_000);
            m.observe(observation(at, true)).unwrap();
            m.observe(observation(at, false)).unwrap();
        }
        let result = m.snapshot();
        prop_assert_eq!(result.eligible_ms, at);
        prop_assert_eq!(result.compliant_ms, compliant);
        prop_assert_eq!(result.checks, gaps.len() as u64);
        prop_assert_eq!(result.blind_spells, blind_spells);
        prop_assert_eq!(result.gaps.max_ms, gaps.iter().max().copied());
        prop_assert!((0.0..=1.0).contains(&result.compliance.unwrap()));
        prop_assert!(result.gaps.median_ms <= result.gaps.p80_ms);
        prop_assert!(result.gaps.p80_ms <= result.gaps.p90_ms);
        prop_assert!(result.gaps.p90_ms.unwrap() <= result.gaps.max_ms.unwrap() as f64);
        prop_assert_eq!(result.unprompted_ratio, Some(1.0));
        prop_assert!(serde_json::to_string(&result).is_ok());
    }

    #[test]
    fn compliance_is_independent_of_tick_density(total in 1u64..100_000, step in 1u64..1_000) {
        let mut coarse = MetricsAccumulator::new(5_000, MetricsConfig::default()).unwrap();
        let mut fine = coarse.clone();
        coarse.observe(observation(0, false)).unwrap();
        fine.observe(observation(0, false)).unwrap();
        coarse.observe(observation(total, false)).unwrap();
        let mut at = step;
        while at < total { fine.observe(observation(at, false)).unwrap(); at += step; }
        fine.observe(observation(total, false)).unwrap();
        prop_assert_eq!(coarse.snapshot(), fine.snapshot());
    }

    #[test]
    fn death_coverage_is_time_weighted_not_sample_count(tracked in 0u64..8_001, extra_frames in 1u64..100) {
        let mut m = MetricsAccumulator::new(5_000, MetricsConfig::default()).unwrap();
        m.observe(observation(0, false)).unwrap();
        for n in 0..extra_frames {
            m.observe(observation(tracked * n / extra_frames, false)).unwrap();
        }
        m.observe(MetricsObservation { tracking: false, ..observation(tracked, false) }).unwrap();
        m.observe(MetricsObservation { tracking: false, ..observation(8_000, false) }).unwrap();
        let death = m.classify_death(Timestamp(8_000)).unwrap();
        prop_assert_eq!(death.tracked_fraction, tracked as f64 / 8_000.0);
        prop_assert_eq!(death.classification, if tracked < 4_000 { DeathClassification::Unknown } else { DeathClassification::Blind });
    }

    #[test]
    fn sessions_never_emit_pulses_while_ineligible(
        seed in any::<u64>(),
        samples in prop::collection::vec((1u64..1_000, any::<bool>(), any::<bool>(), any::<bool>()), 1..300)
    ) {
        let mut s = Session::new("property", SessionConfig::default(), TRAINING_LEVELS[4], SeededRng::new(seed)).unwrap();
        let mut at = 0;
        for (dt, foreground, tracked, looking) in samples {
            at += dt;
            let update = s.observe(GazeSample { at: Timestamp(at), p_minimap: if looking { 1.0 } else { 0.0 }, quality: 1.0 }, GateInputs { foreground, face_tracked: tracked, ..gate() }).unwrap();
            for event in &update.events {
                if matches!(event.kind, EventKind::Pulse(_)) {
                    prop_assert!(update.eligible);
                    prop_assert_ne!(update.gaze, GazeState::Minimap);
                }
            }
            let metrics = s.metrics();
            prop_assert!(metrics.compliant_ms <= metrics.eligible_ms);
            prop_assert!(metrics.eligible_ms <= at);
        }
        prop_assert!(serde_json::to_string(&s.finish(Timestamp(at)).unwrap()).is_ok());
    }
}

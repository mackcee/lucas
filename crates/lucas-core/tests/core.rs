use lucas_core::*;

fn sample(at: u64, p: f64) -> GazeSample {
    GazeSample {
        at: Timestamp(at),
        p_minimap: p,
        quality: 1.0,
    }
}
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
fn suppressed_gates() -> [GateInputs; 6] {
    [
        GateInputs {
            active_game: false,
            ..gate()
        },
        GateInputs {
            alive: false,
            ..gate()
        },
        GateInputs {
            game_time_ms: 90_000,
            ..gate()
        },
        GateInputs {
            foreground: false,
            ..gate()
        },
        GateInputs {
            face_tracked: false,
            ..gate()
        },
        GateInputs {
            paused: true,
            ..gate()
        },
    ]
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
fn metrics() -> MetricsAccumulator {
    MetricsAccumulator::new(5_000, MetricsConfig::default()).unwrap()
}
fn pulse(at: u64) -> Pulse {
    Pulse {
        at: Timestamp(at),
        salience: Salience::Strong,
        effective_delay_ms: 10_000,
    }
}
struct FixedRng(u64);
impl RandomSource for FixedRng {
    fn next_u64(&mut self) -> u64 {
        self.0
    }
}

mod detector {
    use super::*;
    #[test]
    fn dwell_requires_both_time_and_three_distinct_frames() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        assert!(!detector.observe(sample(0, 0.9)).unwrap().check_started);
        assert!(!detector.observe(sample(100, 0.9)).unwrap().check_started);
        assert_eq!(
            detector.observe(sample(100, 0.9)),
            Err(CoreError::DuplicateSample)
        );
        assert!(detector.observe(sample(101, 0.9)).unwrap().check_started);
        assert!(!detector.observe(sample(102, 0.9)).unwrap().check_started);
    }
    #[test]
    fn hysteresis_holds_at_exit_threshold_then_closes_one_glance() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        for at in [0, 50, 100] {
            detector.observe(sample(at, 0.8)).unwrap();
        }
        assert_eq!(
            detector.observe(sample(150, 0.6)).unwrap().state,
            GazeState::Minimap
        );
        let result = detector.observe(sample(200, 0.59)).unwrap();
        assert_eq!(result.state, GazeState::Elsewhere);
        assert_eq!(
            result.completed_glance.unwrap(),
            Glance {
                started_at: Timestamp(0),
                confirmed_at: Timestamp(100),
                ended_at: Timestamp(200),
                dwell_ms: 200
            }
        );
        assert!(
            detector
                .observe(sample(250, 0.7))
                .unwrap()
                .completed_glance
                .is_none()
        );
    }
    #[test]
    fn candidate_requires_consecutive_high_probability_samples() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        for (at, p) in [(0, 1.0), (50, 1.0), (100, 0.7), (150, 1.0), (200, 1.0)] {
            assert!(!detector.observe(sample(at, p)).unwrap().check_started);
        }
        assert!(detector.observe(sample(250, 1.0)).unwrap().check_started);
    }
    #[test]
    fn unknown_interrupts_candidate_and_ends_dwell_without_inventing_away_time() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        for at in [0, 50, 100, 150] {
            detector.observe(sample(at, 1.0)).unwrap();
        }
        let unknown = detector
            .observe(GazeSample {
                quality: 0.0,
                ..sample(200, 1.0)
            })
            .unwrap();
        assert_eq!(unknown.state, GazeState::Unknown);
        assert_eq!(unknown.completed_glance.unwrap().dwell_ms, 150);
        assert!(!detector.observe(sample(250, 1.0)).unwrap().check_started);
        assert!(!detector.observe(sample(300, 1.0)).unwrap().check_started);
        assert!(detector.observe(sample(350, 1.0)).unwrap().check_started);
    }
    #[test]
    fn a_stream_gap_cannot_complete_a_dwell() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        detector.observe(sample(0, 1.0)).unwrap();
        detector.observe(sample(50, 1.0)).unwrap();
        assert!(!detector.observe(sample(5_000, 1.0)).unwrap().check_started);
        assert!(!detector.observe(sample(5_050, 1.0)).unwrap().check_started);
        assert!(detector.observe(sample(5_100, 1.0)).unwrap().check_started);
    }
    #[test]
    fn malformed_samples_are_unknown_and_bad_configs_are_rejected() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        for (i, value) in [f64::NAN, f64::INFINITY, -0.1, 1.1].into_iter().enumerate() {
            assert_eq!(
                detector
                    .observe(sample(i as u64 * 100, value))
                    .unwrap()
                    .state,
                GazeState::Unknown
            );
            assert_eq!(
                detector
                    .observe(GazeSample {
                        quality: value,
                        ..sample(i as u64 * 100 + 50, 1.0)
                    })
                    .unwrap()
                    .state,
                GazeState::Unknown
            );
        }
        assert!(
            GlanceDetector::new(DetectorConfig {
                exit_probability: 0.9,
                ..DetectorConfig::default()
            })
            .is_err()
        );
        assert!(
            GlanceDetector::new(DetectorConfig {
                minimum_quality: 0.0,
                ..DetectorConfig::default()
            })
            .is_err()
        );
    }
    #[test]
    fn out_of_order_does_not_corrupt_a_candidate() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        detector.observe(sample(100, 1.0)).unwrap();
        detector.observe(sample(150, 1.0)).unwrap();
        assert_eq!(
            detector.observe(sample(99, 0.0)),
            Err(CoreError::OutOfOrder)
        );
        assert!(detector.observe(sample(200, 1.0)).unwrap().check_started);
    }
    #[test]
    fn finishing_a_glance_is_idempotent() {
        let mut detector = GlanceDetector::new(DetectorConfig::default()).unwrap();
        for at in [0, 50, 100] {
            detector.observe(sample(at, 1.0)).unwrap();
        }
        assert_eq!(detector.finish_session().unwrap().dwell_ms, 100);
        assert!(detector.finish_session().is_none());
    }
}

mod gap_clock {
    use super::*;
    #[test]
    fn gates_start_strictly_after_ninety_seconds() {
        for g in [
            GateInputs {
                game_time_ms: 90_000,
                ..gate()
            },
            GateInputs {
                active_game: false,
                ..gate()
            },
            GateInputs {
                alive: false,
                ..gate()
            },
        ] {
            let mut clock = GapClock::default();
            clock.update(Timestamp(0), g, false).unwrap();
            clock.update(Timestamp(10_000), g, false).unwrap();
            assert_eq!(clock.gap_ms(), 0);
            assert!(!clock.eligible());
        }
        assert!(
            GateInputs {
                game_time_ms: 90_001,
                ..gate()
            }
            .eligible()
        );
    }
    #[test]
    fn freezes_for_foreground_face_and_pause_then_resumes() {
        for g in [
            GateInputs {
                foreground: false,
                ..gate()
            },
            GateInputs {
                face_tracked: false,
                ..gate()
            },
            GateInputs {
                paused: true,
                ..gate()
            },
        ] {
            let mut clock = GapClock::default();
            clock.update(Timestamp(0), gate(), false).unwrap();
            clock.update(Timestamp(2_000), g, false).unwrap();
            clock.update(Timestamp(12_000), gate(), false).unwrap();
            assert_eq!(clock.gap_ms(), 2_000);
            clock.update(Timestamp(13_000), gate(), false).unwrap();
            assert_eq!(clock.gap_ms(), 3_000);
            assert_eq!(clock.last_check_at(), None);
        }
    }
    #[test]
    fn death_resets_and_respawn_does_not_count_dead_time() {
        let mut clock = GapClock::default();
        clock.update(Timestamp(0), gate(), false).unwrap();
        clock
            .update(
                Timestamp(9_000),
                GateInputs {
                    alive: false,
                    ..gate()
                },
                false,
            )
            .unwrap();
        assert_eq!(clock.gap_ms(), 0);
        clock.update(Timestamp(40_000), gate(), false).unwrap();
        assert_eq!(clock.gap_ms(), 0);
        clock.update(Timestamp(41_000), gate(), false).unwrap();
        assert_eq!(clock.gap_ms(), 1_000);
    }
    #[test]
    fn sustained_minimap_view_keeps_gap_zero() {
        let mut clock = GapClock::default();
        clock.update(Timestamp(0), gate(), false).unwrap();
        clock.update(Timestamp(4_000), gate(), true).unwrap();
        clock.update(Timestamp(24_000), gate(), true).unwrap();
        assert_eq!(clock.gap_ms(), 0);
        assert_eq!(clock.last_check_at(), Some(Timestamp(24_000)));
        clock.update(Timestamp(24_100), gate(), false).unwrap();
        clock.update(Timestamp(25_100), gate(), false).unwrap();
        assert_eq!(clock.gap_ms(), 1_000);
    }
    #[test]
    fn invalid_time_does_not_change_clock_and_large_time_does_not_overflow() {
        let mut clock = GapClock::default();
        clock.update(Timestamp(1), gate(), false).unwrap();
        assert_eq!(
            clock.update(Timestamp(0), gate(), true),
            Err(CoreError::OutOfOrder)
        );
        clock.update(Timestamp(u64::MAX), gate(), false).unwrap();
        assert_eq!(clock.gap_ms(), u64::MAX - 1);
    }
}

mod policy {
    use super::*;
    #[test]
    fn programme_ramps_standard_then_fades_only_help() {
        let ramps: Vec<_> = TRAINING_LEVELS
            .iter()
            .filter(|l| l.phase == Phase::Ramp)
            .collect();
        assert_eq!(
            ramps.iter().map(|l| l.standard_ms).collect::<Vec<_>>(),
            [10_000, 8_000, 6_000, 5_000]
        );
        assert!(
            ramps
                .iter()
                .all(|l| l.pulse_delay_ms == Some(l.standard_ms))
        );
        let fades: Vec<_> = TRAINING_LEVELS
            .iter()
            .filter(|l| matches!(l.phase, Phase::Fade | Phase::Graduated))
            .collect();
        assert!(fades.iter().all(|l| l.standard_ms == 5_000));
        assert_eq!(
            fades.iter().map(|l| l.pulse_delay_ms).collect::<Vec<_>>(),
            [Some(6_000), Some(8_000), Some(10_000), Some(15_000), None]
        );
        assert!(TRAINING_LEVELS.iter().all(|l| l.validate().is_ok()));
    }
    #[test]
    fn custom_levels_are_rejected_without_changing_policy_or_session() {
        let initial = TRAINING_LEVELS[1];
        let mut policy = PulsePolicy::new(initial, false).unwrap();
        let mut session = Session::new(
            "levels",
            SessionConfig::default(),
            initial,
            FixedRng(1 << 63),
        )
        .unwrap();
        session.observe(sample(0, 0.0), gate()).unwrap();
        let before = session.events().to_vec();
        for invalid in [
            TrainingLevel {
                standard_ms: 20_000,
                ..TRAINING_LEVELS[5]
            },
            TrainingLevel {
                phase: Phase::Fade,
                ..TRAINING_LEVELS[4]
            },
            TrainingLevel {
                pulse_delay_ms: Some(7_000),
                ..initial
            },
            TrainingLevel {
                salience: Salience::Soft,
                ..initial
            },
            TrainingLevel {
                pulse_probability: 0.5,
                ..initial
            },
            TrainingLevel {
                pulse_probability: f64::NAN,
                ..initial
            },
        ] {
            assert!(invalid.validate().is_err());
            assert!(PulsePolicy::new(invalid, false).is_err());
            assert!(
                Session::new("invalid", SessionConfig::default(), invalid, FixedRng(0),).is_err()
            );
            assert!(policy.set_level(invalid).is_err());
            assert!(session.set_level(invalid).is_err());
            assert_eq!(policy.level(), initial);
            assert_eq!(session.level(), initial);
            assert_eq!(session.events(), before);
        }
        for level in TRAINING_LEVELS {
            policy.set_level(level).unwrap();
            session.set_level(level).unwrap();
            assert_eq!(policy.level(), level);
            assert_eq!(session.level(), level);
        }
    }
    #[test]
    fn jitter_is_sampled_once_and_cap_moves_by_exact_steps() {
        let mut policy = PulsePolicy::new(TRAINING_LEVELS[1], true).unwrap();
        let mut rng = FixedRng(1 << 63); // unit = 0.5; nominal jitter.
        assert_eq!(policy.prepare_gap(&mut rng), Some(10_000));
        assert_eq!(policy.prepare_gap(&mut FixedRng(0)), Some(10_000));
        assert!(
            policy
                .tick(Timestamp(9_999), 9_999, true, &mut rng)
                .unwrap()
                .is_none()
        );
        assert!(
            policy
                .tick(Timestamp(10_000), 10_000, true, &mut rng)
                .unwrap()
                .is_some()
        );
        assert_eq!(policy.staircase_ms(), 10_750);
        assert!(
            policy
                .tick(Timestamp(20_000), 20_000, true, &mut rng)
                .unwrap()
                .is_none()
        );
        policy.on_check(false);
        assert_eq!(policy.prepare_gap(&mut rng), Some(10_750));
        for _ in 0..3 {
            policy.on_check(true);
        }
        assert_eq!(policy.staircase_ms(), 10_000);
        for _ in 0..100 {
            policy.on_check(true);
        }
        assert_eq!(policy.staircase_ms(), 0);
        assert_eq!(policy.prepare_gap(&mut rng), Some(10_000));
    }
    #[test]
    fn probability_skip_is_not_retried_on_every_frame() {
        let mut policy = PulsePolicy::new(TRAINING_LEVELS[8], true).unwrap();
        let mut rng = FixedRng(u64::MAX);
        assert!(
            policy
                .tick(Timestamp(20_000), 20_000, true, &mut rng)
                .unwrap()
                .is_none()
        );
        for at in 20_001..21_000 {
            assert!(
                policy
                    .tick(Timestamp(at), at, true, &mut FixedRng(0))
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(policy.staircase_ms(), 15_000);
        policy.on_check(true);
        assert!(
            policy
                .tick(Timestamp(50_000), 20_000, true, &mut FixedRng(0))
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn ineligible_tick_does_not_consume_opportunity_and_off_never_pulses() {
        let mut policy = PulsePolicy::new(TRAINING_LEVELS[1], false).unwrap();
        let mut rng = FixedRng(0);
        assert!(
            policy
                .tick(Timestamp(20_000), 20_000, false, &mut rng)
                .unwrap()
                .is_none()
        );
        assert!(
            policy
                .tick(Timestamp(20_001), 20_000, true, &mut rng)
                .unwrap()
                .is_some()
        );
        assert_eq!(policy.staircase_ms(), 10_000);
        policy.set_level(TRAINING_LEVELS[9]).unwrap();
        assert_eq!(policy.prepare_gap(&mut rng), None);
        assert!(
            policy
                .tick(Timestamp(100_000), 100_000, true, &mut rng)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn invalid_level_and_time_leave_policy_usable() {
        let mut policy = PulsePolicy::new(TRAINING_LEVELS[1], true).unwrap();
        let mut rng = FixedRng(0);
        policy.tick(Timestamp(100), 100, true, &mut rng).unwrap();
        assert_eq!(
            policy.tick(Timestamp(99), 20_000, true, &mut rng),
            Err(CoreError::OutOfOrder)
        );
        assert!(
            policy
                .set_level(TrainingLevel {
                    pulse_probability: f64::NAN,
                    ..TRAINING_LEVELS[1]
                })
                .is_err()
        );
        assert_eq!(policy.level(), TRAINING_LEVELS[1]);
        assert!(
            policy
                .tick(Timestamp(10_000), 10_000, true, &mut rng)
                .unwrap()
                .is_some()
        );
    }
}

mod metric_tests {
    use super::*;
    #[test]
    fn primary_metrics_match_hand_calculation_including_terminal_gap() {
        let mut m = metrics();
        m.observe(observation(0, false)).unwrap();
        m.observe(observation(4_000, true)).unwrap(); // gap 4 s
        m.observe(observation(4_000, false)).unwrap();
        m.observe(observation(14_000, true)).unwrap(); // gap 10 s
        m.observe(observation(14_000, false)).unwrap();
        m.observe(observation(34_000, false)).unwrap(); // tail 20 s
        m.record_glance(Glance {
            started_at: Timestamp(0),
            confirmed_at: Timestamp(100),
            ended_at: Timestamp(200),
            dwell_ms: 200,
        });
        m.record_glance(Glance {
            started_at: Timestamp(1_000),
            confirmed_at: Timestamp(1_100),
            ended_at: Timestamp(1_400),
            dwell_ms: 400,
        });
        let result = m.snapshot();
        assert_eq!(result.eligible_ms, 34_000);
        assert_eq!(result.compliant_ms, 14_000);
        assert_eq!(result.compliance, Some(14.0 / 34.0));
        assert_eq!(result.checks, 2);
        assert!((result.checks_per_minute.unwrap() - 120.0 / 34.0).abs() < 1e-10);
        assert_eq!(
            result.gaps,
            GapStatistics {
                median_ms: Some(10_000.0),
                p80_ms: Some(16_000.0),
                p90_ms: Some(18_000.0),
                max_ms: Some(20_000)
            }
        );
        assert_eq!(result.blind_spells, 1);
        assert_eq!(result.mean_dwell_ms, Some(300.0));
        assert_eq!(result.unprompted_ratio, Some(1.0));
    }
    #[test]
    fn empty_ratios_are_none_and_json_safe() {
        let result = metrics().snapshot();
        assert_eq!(result, GameMetrics::default());
        assert!(serde_json::to_string(&result).is_ok());
    }
    #[test]
    fn unknown_and_dead_intervals_do_not_pollute_denominators_or_tail() {
        let mut m = metrics();
        m.observe(observation(0, false)).unwrap();
        m.observe(MetricsObservation {
            tracking: false,
            ..observation(4_000, false)
        })
        .unwrap();
        m.observe(observation(24_000, false)).unwrap();
        m.observe(MetricsObservation {
            gate: GateInputs {
                alive: false,
                ..gate()
            },
            ..observation(30_000, false)
        })
        .unwrap();
        m.observe(observation(50_000, false)).unwrap();
        let result = m.snapshot();
        assert_eq!(result.eligible_ms, 10_000);
        assert_eq!(result.compliant_ms, 5_000);
        assert_eq!(result.gaps.max_ms, Some(10_000));
        assert_eq!(result.blind_spells, 0);
    }
    #[test]
    fn blind_spell_is_strictly_greater_than_threshold_and_counted_once() {
        let mut m = metrics();
        m.observe(observation(0, false)).unwrap();
        m.observe(observation(15_000, false)).unwrap();
        assert_eq!(m.snapshot().blind_spells, 0);
        m.observe(observation(15_001, false)).unwrap();
        m.observe(observation(30_000, false)).unwrap();
        assert_eq!(m.snapshot().blind_spells, 1);
        m.observe(observation(30_000, true)).unwrap();
        m.observe(observation(31_000, false)).unwrap();
        m.observe(observation(46_001, false)).unwrap();
        assert_eq!(m.snapshot().blind_spells, 2);
    }
    #[test]
    fn assistance_windows_latency_and_anticipation_are_distinct() {
        let mut m = metrics();
        m.observe(observation(0, false)).unwrap();
        let first = m.observe(observation(9_500, true)).unwrap().unwrap();
        assert!(first.anticipatory && !first.prompted);
        m.observe(observation(9_500, false)).unwrap();
        m.observe(observation(19_500, false)).unwrap();
        m.record_pulse(pulse(19_500)).unwrap();
        let second = m.observe(observation(22_500, true)).unwrap().unwrap();
        assert!(second.prompted && !second.anticipatory); // inclusive 3 s
        assert_eq!(second.pulse_latency_ms, Some(3_000));
        m.observe(observation(22_500, false)).unwrap();
        m.observe(observation(32_500, false)).unwrap();
        m.record_pulse(pulse(32_500)).unwrap();
        let third = m.observe(observation(35_501, true)).unwrap().unwrap();
        assert!(!third.prompted && !third.anticipatory);
        assert_eq!(third.pulse_latency_ms, Some(3_001));
        let result = m.snapshot();
        assert_eq!(result.pulses, 2);
        assert_eq!(result.unprompted_ratio, Some(2.0 / 3.0));
        assert_eq!(result.anticipation_share, Some(0.5));
        assert_eq!(result.median_pulse_to_glance_ms, Some(3_000.5));
    }
    #[test]
    fn sustained_view_is_compliant_and_a_death_is_seen() {
        let mut m = metrics();
        m.observe(observation(0, true)).unwrap();
        m.observe(MetricsObservation {
            check_started: false,
            ..observation(30_000, true)
        })
        .unwrap();
        assert_eq!(m.snapshot().compliance, Some(1.0));
        assert_eq!(m.snapshot().checks, 1);
        assert_eq!(
            m.classify_death(Timestamp(30_000)).unwrap().classification,
            DeathClassification::Seen
        );
    }
    #[test]
    fn deaths_use_event_time_threshold_and_duration_weighted_coverage() {
        let mut m = metrics();
        m.observe(observation(0, false)).unwrap();
        m.observe(observation(1_000, true)).unwrap();
        m.observe(observation(1_000, false)).unwrap();
        m.observe(observation(8_999, false)).unwrap();
        assert_eq!(
            m.classify_death(Timestamp(8_999)).unwrap().classification,
            DeathClassification::Seen
        );
        m.observe(observation(9_000, false)).unwrap();
        let blind = m.record_death(Timestamp(9_000)).unwrap();
        assert_eq!(blind.classification, DeathClassification::Blind);
        assert_eq!(blind.gap_ms, Some(8_000));
        m.observe(MetricsObservation {
            tracking: false,
            ..observation(10_000, false)
        })
        .unwrap();
        m.observe(observation(16_000, true)).unwrap();
        m.observe(observation(17_000, false)).unwrap();
        // A newly seen check cannot hide six seconds of tracking loss.
        assert_eq!(
            m.record_death(Timestamp(17_000)).unwrap().classification,
            DeathClassification::Unknown
        );
        // Delayed death is unaffected by later checks or later tracking loss.
        assert_eq!(m.classify_death(Timestamp(9_000)).unwrap(), blind);
        assert_eq!(m.snapshot().blind_deaths, 1);
        assert_eq!(m.snapshot().unknown_deaths, 1);
        assert!(m.classify_death(Timestamp(18_000)).is_err());
    }
    #[test]
    fn suppressed_intervals_are_not_usable_death_evidence() {
        for gate in suppressed_gates() {
            let mut m = metrics();
            m.observe(MetricsObservation {
                gate,
                ..observation(0, true)
            })
            .unwrap();
            m.observe(observation(10_000, false)).unwrap();
            let death = m.classify_death(Timestamp(10_000)).unwrap();
            assert_eq!(death.classification, DeathClassification::Unknown);
            assert_eq!(death.tracked_fraction, 0.0);
            assert_eq!(m.snapshot().checks, 0);
            m.observe(observation(13_999, false)).unwrap();
            assert_eq!(
                m.classify_death(Timestamp(13_999)).unwrap().classification,
                DeathClassification::Unknown
            );
            m.observe(observation(18_000, false)).unwrap();
            assert_eq!(
                m.classify_death(Timestamp(18_000)).unwrap().classification,
                DeathClassification::Blind
            );
            assert_eq!(m.classify_death(Timestamp(10_000)).unwrap(), death);
        }
    }
    #[test]
    fn half_tracked_is_not_mostly_lost_and_threshold_is_configurable() {
        let mut m = MetricsAccumulator::new(
            5_000,
            MetricsConfig {
                blind_death_ms: 4_000,
                ..MetricsConfig::default()
            },
        )
        .unwrap();
        m.observe(observation(0, false)).unwrap();
        m.observe(MetricsObservation {
            tracking: false,
            ..observation(2_000, false)
        })
        .unwrap();
        m.observe(observation(4_000, false)).unwrap();
        let result = m.classify_death(Timestamp(4_000)).unwrap();
        assert_eq!(result.tracked_fraction, 0.5);
        assert_eq!(result.classification, DeathClassification::Blind);
    }
    #[test]
    fn absent_history_is_unknown_not_a_fabricated_blind_death() {
        for start in [0, 100_000] {
            let mut m = metrics();
            m.observe(observation(start, false)).unwrap();
            for duration in [1_000, 4_000, 5_000, 7_999] {
                m.observe(observation(start + duration, false)).unwrap();
                let death = m.classify_death(Timestamp(start + duration)).unwrap();
                assert_eq!(death.classification, DeathClassification::Unknown);
                assert_eq!(death.gap_ms, None);
            }
            m.observe(observation(start + 8_000, false)).unwrap();
            assert_eq!(
                m.classify_death(Timestamp(start + 8_000))
                    .unwrap()
                    .classification,
                DeathClassification::Blind
            );
            assert_eq!(
                m.classify_death(Timestamp(start + 5_000))
                    .unwrap()
                    .classification,
                DeathClassification::Unknown
            );
            if start > 0 {
                assert!(m.classify_death(Timestamp(start - 1)).is_err());
            }
        }
    }
    #[test]
    fn changing_standard_scores_subsequent_time_without_rewriting_history() {
        let mut m = MetricsAccumulator::new(10_000, MetricsConfig::default()).unwrap();
        m.observe(observation(0, false)).unwrap();
        m.observe(observation(8_000, false)).unwrap();
        m.set_standard(5_000).unwrap();
        m.observe(observation(10_000, false)).unwrap();
        assert_eq!(m.snapshot().compliant_ms, 8_000);
        assert_eq!(m.snapshot().compliance, Some(0.8));
    }
    #[test]
    fn backward_observation_does_not_mutate_metrics() {
        let mut m = metrics();
        m.observe(observation(10_000, false)).unwrap();
        let before = m.snapshot();
        assert_eq!(
            m.observe(observation(9_000, true)),
            Err(CoreError::OutOfOrder)
        );
        assert_eq!(m.snapshot(), before);
    }
}

mod session_tests {
    use super::*;
    fn session() -> Session<FixedRng> {
        Session::new(
            "test",
            SessionConfig::default(),
            TRAINING_LEVELS[1],
            FixedRng(1 << 63),
        )
        .unwrap()
    }
    #[test]
    fn complete_trace_pulses_checks_and_round_trips_storage_payload() {
        let mut s = session();
        for at in (0..=11_000).step_by(50) {
            s.observe(
                sample(
                    at,
                    if (10_500..10_800).contains(&at) {
                        1.0
                    } else {
                        0.0
                    },
                ),
                gate(),
            )
            .unwrap();
        }
        let death = s.death(Timestamp(11_000)).unwrap();
        assert_eq!(death.classification, DeathClassification::Seen);
        assert_eq!(s.death(Timestamp(11_000)).unwrap(), death);
        let record = s.finish(Timestamp(11_000)).unwrap();
        assert_eq!(record.metrics.pulses, 1);
        assert_eq!(record.metrics.checks, 1);
        assert_eq!(record.metrics.unprompted_ratio, Some(0.0));
        assert_eq!(record.metrics.median_pulse_to_glance_ms, Some(600.0));
        assert_eq!(record.metrics.mean_dwell_ms, Some(300.0));
        assert_eq!(record.metrics.seen_deaths, 1);
        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(
            serde_json::from_str::<SessionRecord>(&json).unwrap(),
            record
        );
        assert!(!json.contains("p_minimap"));
    }
    #[test]
    fn suppressed_minimap_samples_do_not_produce_blind_deaths() {
        for gate in suppressed_gates() {
            let mut s = session();
            for at in (0..=10_000).step_by(50) {
                s.observe(sample(at, 1.0), gate).unwrap();
            }
            let death = s.death(Timestamp(10_000)).unwrap();
            assert_eq!(death.classification, DeathClassification::Unknown);
            assert_eq!(death.tracked_fraction, 0.0);
            assert_eq!(s.metrics().checks, 0);
        }
    }
    #[test]
    fn gate_events_ignore_clock_progress_but_preserve_state_transitions() {
        let mut s = session();
        for at in (0..=2_000).step_by(50) {
            s.observe(
                sample(at, 0.0),
                GateInputs {
                    game_time_ms: 89_000 + at,
                    ..gate()
                },
            )
            .unwrap();
        }
        let changes: Vec<_> = s
            .events()
            .iter()
            .filter_map(|event| {
                if let EventKind::GateChanged(gate) = event.kind {
                    Some((event.at, gate.game_time_ms))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            changes,
            [(Timestamp(0), 89_000), (Timestamp(1_050), 90_050)]
        );
        let mut at = 2_000;
        for changed_gate in suppressed_gates() {
            for gate in [changed_gate, gate()] {
                at += 50;
                let update = s.observe(sample(at, 0.0), gate).unwrap();
                assert_eq!(
                    update
                        .events
                        .iter()
                        .filter(|event| { matches!(event.kind, EventKind::GateChanged(_)) })
                        .count(),
                    1
                );
            }
        }
    }
    #[test]
    fn silent_camera_expiry_freezes_gap_and_marks_death_unknown() {
        let mut s = session();
        s.observe(sample(0, 0.0), gate()).unwrap();
        let update = s.observe(sample(20_000, 0.0), gate()).unwrap();
        assert_eq!(update.gap_ms, 250);
        assert_eq!(s.metrics().eligible_ms, 250);
        assert_eq!(s.metrics().pulses, 0);
        assert_eq!(
            s.death(Timestamp(20_000)).unwrap().classification,
            DeathClassification::Unknown
        );
        assert!(update.events.iter().any(|e| e.at == Timestamp(250)
            && matches!(e.kind, EventKind::GazeChanged(GazeState::Unknown))));
    }
    #[test]
    fn game_gate_loss_prevents_glances_pulses_and_resets_after_death() {
        let mut s = session();
        for at in (0..=20_000).step_by(50) {
            let g = if at >= 9_000 {
                GateInputs {
                    alive: false,
                    ..gate()
                }
            } else {
                gate()
            };
            s.observe(sample(at, if at >= 9_000 { 1.0 } else { 0.0 }), g)
                .unwrap();
        }
        let update = s.observe(sample(20_050, 0.0), gate()).unwrap();
        assert_eq!(update.gap_ms, 0);
        assert_eq!(s.metrics().pulses, 0);
        assert_eq!(s.metrics().checks, 0);
        assert_eq!(s.metrics().eligible_ms, 9_000);
    }
    #[test]
    fn level_setter_is_manual_and_preserves_changes_in_event_log() {
        let mut s = session();
        s.set_level(TRAINING_LEVELS[3]).unwrap();
        s.observe(sample(0, 0.0), gate()).unwrap();
        s.set_level(TRAINING_LEVELS[9]).unwrap();
        for at in (50..=20_000).step_by(50) {
            s.observe(sample(at, 0.0), gate()).unwrap();
        }
        assert_eq!(s.level(), TRAINING_LEVELS[9]);
        let record = s.finish(Timestamp(20_000)).unwrap();
        assert_eq!(record.initial_level, TRAINING_LEVELS[3]);
        assert_eq!(record.metrics.pulses, 0);
        assert!(
            record
                .events
                .iter()
                .any(|e| matches!(e.kind, EventKind::LevelChanged(_)))
        );
    }
    #[test]
    fn finishing_long_after_camera_stops_does_not_fabricate_eligible_time() {
        let mut s = session();
        s.observe(sample(0, 0.0), gate()).unwrap();
        let record = s.finish(Timestamp(60_000)).unwrap();
        assert_eq!(record.metrics.eligible_ms, 250);
        assert_eq!(record.metrics.gaps.max_ms, Some(250));
        assert_eq!(record.metrics.pulses, 0);
    }
    #[test]
    fn finish_closes_open_glance_and_duplicate_samples_are_transactional() {
        let mut s = session();
        for at in [0, 50, 100, 150] {
            s.observe(sample(at, 1.0), gate()).unwrap();
        }
        let before = s.events().to_vec();
        assert_eq!(
            s.observe(sample(150, 0.0), gate()),
            Err(CoreError::DuplicateSample)
        );
        assert_eq!(
            s.observe(sample(149, 0.0), gate()),
            Err(CoreError::OutOfOrder)
        );
        assert_eq!(s.events(), before);
        let record = s.finish(Timestamp(150)).unwrap();
        assert_eq!(record.metrics.mean_dwell_ms, Some(150.0));
    }
    #[test]
    fn empty_session_is_serializable_and_has_no_eligible_time() {
        let record = session().finish(Timestamp(0)).unwrap();
        assert_eq!(record.metrics, GameMetrics::default());
        assert!(serde_json::to_string(&record).is_ok());
    }
}

//! Design plan §13 P4: approximate load, not bit-for-bit prototype parity.
use crate::sim::{Config, run};

fn band(label: &str, actual: f64, low: f64, high: f64) {
    assert!(
        (low..=high).contains(&actual),
        "{label}: {actual:.4} outside [{low}, {high}]"
    );
}

#[test]
fn pulse_load_and_unassisted_blind_time_stay_in_reference_bands() {
    // Six full games retain staircase settling time while staying fast in CI.
    // Reference (20 x 30 min, gamma k=2, 80% response, S=10):
    // mean:             4     6     8     10     15     20
    // uncapped pulses: 18.7  49.8  69.6   85.2  104.7  110.3
    // capped pulses:    --   44.6  55.8   54.3   46.5   44.2
    // solo blind:      .023 .093  .181   .268   .443     --
    // Allow sampling, dwell, detector confirmation and prototype differences.
    let config = Config {
        games: 6,
        caps: vec![false, true],
        ..Config::default()
    };
    let rows = run(&config).unwrap();
    let references = [
        (18.7, None, 0.023),
        (49.8, Some(44.6), 0.093),
        (69.6, Some(55.8), 0.181),
        (85.2, Some(54.3), 0.268),
        (104.7, Some(46.5), 0.443),
        // Gamma renewal limit at mean=20, S=10: exp(-1)*(1+1/2).
        (110.3, Some(44.2), 0.552),
    ];
    for (pair, (uncapped_ref, capped_ref, blind_ref)) in rows.chunks_exact(2).zip(references) {
        let uncapped = &pair[0];
        let capped = &pair[1];
        let label = format!("mean {}s", uncapped.mean);
        band(
            &format!("{label} uncapped pulses"),
            uncapped.pulses_per_game,
            uncapped_ref * 0.75,
            uncapped_ref * 1.25,
        );
        if let Some(reference) = capped_ref {
            band(
                &format!("{label} capped pulses"),
                capped.pulses_per_game,
                reference * 0.75,
                reference * 1.25,
            );
        }
        band(
            &format!("{label} solo blind"),
            uncapped.unassisted_blind_share,
            (blind_ref - 0.04_f64).max(0.0),
            blind_ref + 0.04,
        );
        // Approximate gamma(k=2) median is .83917 * mean (dwell excluded).
        band(
            &format!("{label} solo median"),
            uncapped.unassisted_median_gap_seconds.unwrap(),
            uncapped.mean * 0.72,
            uncapped.mean * 0.98,
        );
        assert_eq!(
            uncapped.unassisted_blind_share,
            capped.unassisted_blind_share
        );
        assert!(
            uncapped
                .prompted_gap_share
                .is_some_and(|s| (0.0..=1.0).contains(&s))
        );
        if uncapped.mean >= 10.0 {
            band(
                &format!("{label} cap reduction"),
                capped.pulses_per_game / uncapped.pulses_per_game,
                0.30,
                0.75,
            );
            assert!(capped.prompted_gap_share < uncapped.prompted_gap_share);
        }
    }
}

#[test]
fn approved_fade_levels_reduce_load_and_keep_five_second_solo_standard() {
    // P4's older 15s/.75 and 20s/.5 programme is NOT the approved core ladder.
    // Test the actual manual programme instead of adding custom policy knobs.
    let config = Config {
        games: 3,
        means: vec![10.0],
        levels: vec![4, 7, 8, 9],
        caps: vec![false],
        ..Config::default()
    };
    let rows = run(&config).unwrap();
    for pair in rows.windows(2) {
        assert!(pair[0].pulses_per_game > pair[1].pulses_per_game);
        assert_eq!(
            pair[0].unassisted_blind_share,
            pair[1].unassisted_blind_share
        );
        assert_eq!(
            pair[0].unassisted_median_gap_seconds,
            pair[1].unassisted_median_gap_seconds
        );
    }
    band("S=5 solo blind", rows[0].unassisted_blind_share, 0.49, 0.60);
    assert_eq!(rows[3].pulses_per_game, 0.0);
}

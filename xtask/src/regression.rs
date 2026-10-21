//! Repeating-reminder load: broad seeded simulation bands, not player evidence.
use crate::sim::{Config, run};

fn band(label: &str, actual: f64, low: f64, high: f64) {
    assert!(
        (low..=high).contains(&actual),
        "{label}: {actual:.4} outside [{low}, {high}]"
    );
}

#[test]
fn repeating_ramp_load_and_unassisted_blind_time_stay_in_reference_bands() {
    // Six full games keep CI fast. Repeating policy reference measured with
    // 20 x 30 min, seed 42, gamma k=2, 80% response, S=10. The old one-shot
    // prototype's pulse anchors/cap reduction no longer describe Ramp.
    // Keep broad sampling bands; solo blind anchors remain analytical.
    let config = Config {
        games: 6,
        caps: vec![false, true],
        ..Config::default()
    };
    let rows = run(&config).unwrap();
    let references = [
        (15.8, 0.023),
        (46.6, 0.093),
        (70.3, 0.181),
        (89.8, 0.268),
        (118.5, 0.443),
        // Gamma renewal limit at mean=20, S=10: exp(-1)*(1+1/2).
        (134.2, 0.552),
    ];
    for (pair, (uncapped_ref, blind_ref)) in rows.chunks_exact(2).zip(references) {
        let uncapped = &pair[0];
        let capped = &pair[1];
        let label = format!("mean {}s", uncapped.mean);
        band(
            &format!("{label} uncapped pulses"),
            uncapped.pulses_per_game,
            uncapped_ref * 0.75,
            uncapped_ref * 1.25,
        );
        assert_eq!(uncapped.pulses_per_game, capped.pulses_per_game);
        assert_eq!(uncapped.prompted_gap_share, capped.prompted_gap_share);
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
    }
}

#[test]
fn fade_staircase_still_reduces_load_with_repeating_reminders() {
    let config = Config {
        games: 6,
        means: vec![15.0, 20.0],
        levels: vec![7],
        caps: vec![false, true],
        ..Config::default()
    };
    for pair in run(&config).unwrap().chunks_exact(2) {
        let (uncapped, capped) = (&pair[0], &pair[1]);
        band(
            "Fade cap reduction",
            capped.pulses_per_game / uncapped.pulses_per_game,
            0.30,
            0.75,
        );
        assert!(capped.prompted_gap_share < uncapped.prompted_gap_share);
        assert_eq!(
            uncapped.unassisted_blind_share,
            capped.unassisted_blind_share
        );
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

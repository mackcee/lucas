use lucas_core::Timestamp;
use std::time::Duration;

/// Retain the entire game's ~1Hz clock anchors, not just the game crate's recent
/// 120 polls. Interpolation stops across outages; a stopped game clock stays flat.
#[derive(Default)]
pub(super) struct GameClock {
    anchors: Vec<(u64, u64, bool)>,
    interrupted: bool,
}
impl GameClock {
    pub fn push(&mut self, at: Duration, game_seconds: f64) {
        let pair = (
            at.as_millis() as u64,
            (game_seconds * 1000.0).round() as u64,
            !self.interrupted,
        );
        if self
            .anchors
            .last()
            .is_none_or(|last| pair.0 > last.0 && pair.1 >= last.1)
        {
            self.anchors.push(pair);
            self.interrupted = false;
        }
    }
    pub fn unavailable(&mut self) {
        self.interrupted = true;
    }
    pub fn length_ms(&self) -> u64 {
        self.anchors.last().map_or(0, |p| p.1)
    }
    pub fn game_at(&self, at: Timestamp) -> Option<u64> {
        let first = *self.anchors.first()?;
        if at.0 < first.0 {
            return None;
        }
        for pair in self.anchors.windows(2) {
            let [(a, x, _), (b, y, continuous)] = [pair[0], pair[1]];
            if at.0 <= b {
                if at.0 == b {
                    return Some(y);
                }
                return Some(if continuous && b - a <= 3000 {
                    interpolate(at.0, a, b, x, y)
                } else {
                    x
                });
            }
        }
        // Final pulses/checks are conservatively clamped to the latest poll.
        Some(self.length_ms())
    }
    pub fn wall_at(&self, game_ms: u64) -> Option<Timestamp> {
        for &(wall, game, _) in &self.anchors {
            if game == game_ms {
                return Some(Timestamp(wall));
            }
        }
        self.anchors.windows(2).find_map(|pair| {
            let [(a, x, _), (b, y, continuous)] = [pair[0], pair[1]];
            (continuous && x < game_ms && game_ms < y && b - a <= 3000)
                .then(|| Timestamp(interpolate(game_ms, x, y, a, b)))
        })
    }
}
fn interpolate(value: u64, left: u64, right: u64, start: u64, end: u64) -> u64 {
    start + ((value - left) as f64 / (right - left) as f64 * (end - start) as f64).round() as u64
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_poll_breaks_short_intervals_without_losing_history() {
        let mut clock = GameClock::default();
        clock.push(Duration::from_secs(99), 199.0);
        clock.push(Duration::from_secs(100), 200.0);
        clock.unavailable();
        clock.unavailable();
        clock.push(Duration::from_secs(102), 201.0);
        clock.push(Duration::from_secs(103), 202.0);
        assert_eq!(clock.wall_at(199_500), Some(Timestamp(99_500)));
        assert_eq!(clock.wall_at(200_000), Some(Timestamp(100_000)));
        assert_eq!(clock.wall_at(200_250), None);
        assert_eq!(clock.game_at(Timestamp(100_500)), Some(200_000));
        assert_eq!(clock.wall_at(201_000), Some(Timestamp(102_000)));
        assert_eq!(clock.wall_at(201_500), Some(Timestamp(102_500)));
        assert_eq!(clock.game_at(Timestamp(102_500)), Some(201_500));
    }

    #[test]
    fn pause_outage_and_long_game_history() {
        let mut clock = GameClock::default();
        for i in 0..1800 {
            clock.push(Duration::from_secs(i), i as f64);
        }
        assert_eq!(clock.game_at(Timestamp(500)), Some(500));
        assert_eq!(clock.wall_at(500), Some(Timestamp(500)));
        clock.push(Duration::from_secs(1800), 1799.0);
        assert_eq!(clock.game_at(Timestamp(1799500)), Some(1799000));
        clock.push(Duration::from_secs(1810), 1805.0);
        assert_eq!(clock.wall_at(1800000), None);
        assert_eq!(clock.game_at(Timestamp(1805000)), Some(1799000));
    }
}

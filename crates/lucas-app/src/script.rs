//! Deterministic target schedule. No camera, clock, window or file access.
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const COUNTDOWN: Duration = Duration::from_secs(2);
pub const HOLD: Duration = Duration::from_secs(4);
pub const DEFAULT_SECONDS: u64 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Minimap,
    Elsewhere,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// Unique occurrence, not a reusable position index.
    pub id: u32,
    /// Normalized monitor coordinates, top-left origin. No desktop offset.
    pub position: [f64; 2],
    pub kind: TargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Countdown,
    Hold,
    /// Window not focused: never treat these frames as labelled fixation data.
    Unfocused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub target: Target,
    pub phase: Phase,
    pub remaining: Duration,
}

pub struct Script {
    duration: Duration,
    minimap: [f64; 2],
}

impl Script {
    /// 1..=60 minutes, in whole six-second trials. Default: 100 trials / 10 minutes.
    pub fn new(seconds: u64, monitor: [u32; 2]) -> Result<Self, &'static str> {
        if !(60..=3600).contains(&seconds) || !seconds.is_multiple_of(6) {
            return Err("duration must be 60..=3600 seconds and a multiple of 6");
        }
        let bounds = Rect::new(0.0, 0.0, monitor[0] as f64, monitor[1] as f64)
            .map_err(|_| "invalid monitor resolution")?;
        let map = minimap_rect(bounds, MinimapSettings::default(), Alignment::default())
            .map_err(|_| "cannot locate minimap")?;
        Ok(Self {
            duration: Duration::from_secs(seconds),
            minimap: [
                (map.left() + map.width() / 2.0) / bounds.width(),
                (map.top() + map.height() / 2.0) / bounds.height(),
            ],
        })
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn trials(&self) -> u32 {
        (self.duration.as_secs() / 6) as u32
    }

    pub fn at(&self, elapsed: Duration) -> Option<Step> {
        if elapsed >= self.duration {
            return None;
        }
        let cycle = COUNTDOWN + HOLD;
        let id = (elapsed.as_nanos() / cycle.as_nanos()) as u32;
        let within = elapsed - cycle * id;
        let (phase, remaining) = if within < COUNTDOWN {
            (Phase::Countdown, COUNTDOWN - within)
        } else {
            (Phase::Hold, cycle - within)
        };
        // All nine broad calibration positions, plus challenging negatives near
        // the minimap. The exact lower-right corner is positive, not a negative.
        let map_x = self.minimap[0];
        let map_y = self.minimap[1];
        let elsewhere = [
            [0.5, 0.5],
            [0.08, 0.08],
            [0.5, 0.08],
            [0.92, 0.08],
            [0.08, 0.5],
            [0.92, 0.5],
            [0.08, 0.92],
            [0.5, 0.92],
            [map_x, (map_y - 0.22).max(0.1)],
            [(map_x - 0.22).max(0.1), map_y],
        ];
        let (kind, position) = if id.is_multiple_of(2) {
            (
                TargetKind::Elsewhere,
                elsewhere[(id / 2) as usize % elsewhere.len()],
            )
        } else {
            (TargetKind::Minimap, self.minimap)
        };
        Some(Step {
            target: Target { id, position, kind },
            phase,
            remaining,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_boundaries_alternation_and_completion() {
        let s = Script::new(600, [1920, 1080]).unwrap();
        assert_eq!(s.trials(), 100);
        assert_eq!(s.at(Duration::ZERO).unwrap().phase, Phase::Countdown);
        assert_eq!(
            s.at(Duration::from_millis(1999)).unwrap().phase,
            Phase::Countdown
        );
        assert_eq!(s.at(COUNTDOWN).unwrap().phase, Phase::Hold);
        for id in 0..100 {
            let step = s.at(Duration::from_secs(6 * id + 2)).unwrap();
            assert_eq!(step.target.id, id as u32);
            assert_eq!(
                step.target.kind,
                if id % 2 == 0 {
                    TargetKind::Elsewhere
                } else {
                    TargetKind::Minimap
                }
            );
        }
        assert!(s.at(Duration::from_secs(600)).is_none());
        assert!(s.at(Duration::MAX).is_none());
    }
    #[test]
    fn physical_geometry_normalizes_at_any_dpi_and_resolution() {
        for resolution in [[1920, 1080], [3840, 2160], [3440, 1440]] {
            let s = Script::new(60, resolution).unwrap();
            let target = s.at(Duration::from_secs(8)).unwrap().target;
            let map = minimap_rect(
                Rect::new(0.0, 0.0, resolution[0] as f64, resolution[1] as f64).unwrap(),
                MinimapSettings::default(),
                Alignment::default(),
            )
            .unwrap();
            assert!(map.contains(
                target.position[0] * resolution[0] as f64,
                target.position[1] * resolution[1] as f64
            ));
        }
    }
    #[test]
    fn validates_duration_and_covers_screen_without_negative_in_map() {
        for seconds in [0, 59, 61, 3606, u64::MAX] {
            assert!(Script::new(seconds, [1920, 1080]).is_err());
        }
        assert!(Script::new(60, [0, 1080]).is_err());
        let s = Script::new(120, [1920, 1080]).unwrap();
        let mut positions = Vec::new();
        for id in (0..20).step_by(2) {
            let target = s.at(Duration::from_secs(id * 6 + 2)).unwrap().target;
            assert!(target.position.iter().all(|v| (0.0..=1.0).contains(v)));
            assert!(!positions.contains(&target.position));
            positions.push(target.position);
        }
        assert_eq!(positions.len(), 10);
    }
}

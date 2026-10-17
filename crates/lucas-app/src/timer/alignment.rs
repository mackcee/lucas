use super::alignment_store::Key;
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjustment {
    Left,
    Right,
    Up,
    Down,
    Grow,
    Shrink,
}

/// Snapshot the profile at entry. A HUD/resolution change cancels rather than
/// saving an adjustment under a different profile. Monitor movement is allowed.
#[derive(Debug, Clone, Copy)]
pub struct AlignmentSession {
    pub key: Key,
    pub draft: Alignment,
}

impl AlignmentSession {
    pub fn new(key: Key, saved: Alignment) -> Self {
        Self { key, draft: saved }
    }

    pub fn adjust(&mut self, client: Rect, settings: MinimapSettings, action: Adjustment) {
        if Key::new(client, settings) != self.key {
            return;
        }
        let Ok(seed) = minimap_rect(client, settings, Alignment::default()) else {
            return;
        };
        let mut candidate = self.draft;
        match action {
            Adjustment::Left => candidate.offset_x_px -= 1.0,
            Adjustment::Right => candidate.offset_x_px += 1.0,
            Adjustment::Up => candidate.offset_y_px -= 1.0,
            Adjustment::Down => candidate.offset_y_px += 1.0,
            Adjustment::Grow => candidate.scale_multiplier += 1.0 / seed.width(),
            Adjustment::Shrink => candidate.scale_multiplier -= 1.0 / seed.width(),
        }
        if !(0.25..=4.0).contains(&candidate.scale_multiplier) {
            return;
        }
        if let Ok(rect) = minimap_rect(client, settings, candidate)
            && rect.left() >= client.left()
            && rect.top() >= client.top()
            && rect.right() <= client.right()
            && rect.bottom() <= client.bottom()
        {
            self.draft = candidate;
        }
    }
}

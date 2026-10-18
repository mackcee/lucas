use super::alignment_store::Key;
use lucas_geometry::{Alignment, MinimapSettings, Rect, minimap_rect};

/// Only the alignment border accepts the control panel as foreground. Ordinary
/// coaching still requires League itself, and all other window gates remain.
pub fn preview_eligible(mut window: lucas_game::WindowStatus, panel_foreground: bool) -> bool {
    window.foreground |= panel_foreground;
    window.overlay_eligible()
}

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
        // The seed has fractional edges (e.g. a 14.976 px right inset at
        // 3840x2160). Allow the final one-pixel nudge across the client edge;
        // otherwise whole-pixel steps can never reach that edge. No wider
        // off-screen travel is needed, and the geometry formula is unchanged.
        if let Ok(rect) = minimap_rect(client, settings, candidate)
            && rect.left() > client.left() - 1.0
            && rect.top() > client.top() - 1.0
            && rect.right() < client.right() + 1.0
            && rect.bottom() < client.bottom() + 1.0
        {
            self.draft = candidate;
        }
    }
}

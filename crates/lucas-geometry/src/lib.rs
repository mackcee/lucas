//! Pure minimap geometry and parsers for League's supplied settings content.
//!
//! All coordinates are **physical pixels**, in one desktop coordinate space. On
//! Windows the caller must be per-monitor-v2 DPI aware and convert the game's
//! client origin to screen coordinates. Do not divide 4K coordinates by a 150%
//! desktop scale factor. Negative origins (secondary monitors) are supported.
//! This crate does no window queries, disk I/O, drawing, or gaze classification.
//!
//! The HUD formula is a community-calibrated **seed**, not pixel-exact ground
//! truth. See the crate README for its constants, alignment and fixture provenance.
//!
//! ```
//! use lucas_geometry::{
//!     Alignment, MinimapSettings, Rect, ViewingGeometry, gaze_zone, minimap_rect,
//!     parse_game_cfg,
//! };
//!
//! let settings = parse_game_cfg("[HUD]\nMinimapScale=1.5\nFlipMiniMap=0")?
//!     .resolve(MinimapSettings::default());
//! let client = Rect::new(0.0, 0.0, 3840.0, 2160.0)?;
//! let map = minimap_rect(client, settings, Alignment::default())?;
//! let zone = gaze_zone(map, ViewingGeometry {
//!     bounds: client, width_mm: 597.7, height_mm: 336.2, distance_mm: 650.0,
//! }, 2.0)?.expect("map overlaps the monitor");
//! assert!(zone.contains(map.left(), map.top()));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

mod settings;
pub use settings::{SettingsError, SettingsPatch, parse_game_cfg, parse_persisted_settings};

use std::fmt;

/// Invalid or unrepresentable geometry; no NaN/infinite rectangles are returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryError(pub &'static str);

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for GeometryError {}

/// A nonempty axis-aligned rectangle in physical desktop pixels.
///
/// Edges are half-open: left/top included, right/bottom excluded. Fractional
/// pixels are retained; a renderer can floor left/top and ceil right/bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Rect {
    /// Constructs a rectangle from edges, not a position and size.
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Result<Self, GeometryError> {
        if ![left, top, right, bottom, right - left, bottom - top]
            .into_iter()
            .all(f64::is_finite)
            || right <= left
            || bottom <= top
        {
            return Err(GeometryError(
                "rectangle must have finite edges and positive finite size",
            ));
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    pub fn left(self) -> f64 {
        self.left
    }
    pub fn top(self) -> f64 {
        self.top
    }
    pub fn right(self) -> f64 {
        self.right
    }
    pub fn bottom(self) -> f64 {
        self.bottom
    }
    pub fn width(self) -> f64 {
        self.right - self.left
    }
    pub fn height(self) -> f64 {
        self.bottom - self.top
    }

    /// Nonfinite points are never contained.
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        )
        .ok()
    }
}

/// League HUD settings. `minimap_scale` is the stored 0..=3 value, not slider percent.
/// Geometry validates it even when settings are constructed without a parser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimapSettings {
    pub minimap_scale: f64,
    pub flipped: bool,
}

impl Default for MinimapSettings {
    /// Explicit application fallback, not a claim about every League installation.
    fn default() -> Self {
        Self {
            minimap_scale: 1.0,
            flipped: false,
        }
    }
}

pub(crate) fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && (0.0..=3.0).contains(&scale)
}

/// User correction applied after the seed formula.
///
/// Scaling preserves the bottom and the selected outer edge (right normally,
/// left when flipped). Translation then moves right/down for positive values.
/// Offsets are physical pixels, never logical UI pixels. Store corrections per
/// client resolution, MinimapScale and flip state; profile selection is external.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    pub offset_x_px: f64,
    pub offset_y_px: f64,
    pub scale_multiplier: f64,
}

impl Default for Alignment {
    fn default() -> Self {
        Self {
            offset_x_px: 0.0,
            offset_y_px: 0.0,
            scale_multiplier: 1.0,
        }
    }
}

/// Computes the square minimap seed and applies alignment without clipping it.
///
/// Side = client height * (0.1651 + 0.0764 * MinimapScale).
/// Horizontal inset = 0.0039 * client width; bottom inset = 0.0069 * height.
/// Flipping mirrors the seed horizontally. GlobalScale is deliberately unused.
/// Common 4:3, 16:10, 16:9 and ultrawide layouts use height-based HUD scaling;
/// the seed still needs real-game alignment validation after game patches.
pub fn minimap_rect(
    client: Rect,
    settings: MinimapSettings,
    alignment: Alignment,
) -> Result<Rect, GeometryError> {
    if !valid_scale(settings.minimap_scale) {
        return Err(GeometryError("MinimapScale must be finite and in 0..=3"));
    }
    if !alignment.offset_x_px.is_finite()
        || !alignment.offset_y_px.is_finite()
        || !alignment.scale_multiplier.is_finite()
        || alignment.scale_multiplier <= 0.0
    {
        return Err(GeometryError(
            "alignment needs finite offsets and a positive finite scale",
        ));
    }
    let side =
        client.height() * (0.1651 + 0.0764 * settings.minimap_scale) * alignment.scale_multiplier;
    let inset = client.width() * 0.0039;
    let bottom = client.bottom() - client.height() * 0.0069 + alignment.offset_y_px;
    let left = if settings.flipped {
        client.left() + inset + alignment.offset_x_px
    } else {
        client.right() - inset - side + alignment.offset_x_px
    };
    Rect::new(left, bottom - side, left + side, bottom)
}

/// Physical monitor dimensions and eye-to-screen distance, all in millimetres.
///
/// `bounds` covers the full monitor in physical desktop pixels, not just the
/// game client or the desktop work area. Supply the monitor hosting the game;
/// spanning monitors with different pixel pitches needs separate handling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewingGeometry {
    pub bounds: Rect,
    pub width_mm: f64,
    pub height_mm: f64,
    pub distance_mm: f64,
}

/// Inflates the map by a per-edge visual-angle margin, then clips to the monitor.
///
/// The local planar approximation is `distance * tan(margin_degrees)` mm per
/// edge, converted independently by each axis's pixel pitch. The margin is a
/// **radius**, not a full angular diameter; callers may pass e.g. twice their
/// validated angular error. This is error tolerance, not an off-axis gaze model.
/// Margin must be finite in [0, 90) degrees. Zero only clips; no overlap yields
/// `Ok(None)`. Off-screen points are never counted by the resulting rectangle.
pub fn gaze_zone(
    minimap: Rect,
    viewing: ViewingGeometry,
    margin_degrees: f64,
) -> Result<Option<Rect>, GeometryError> {
    if ![viewing.width_mm, viewing.height_mm, viewing.distance_mm]
        .into_iter()
        .all(|n| n.is_finite() && n > 0.0)
    {
        return Err(GeometryError(
            "screen dimensions and viewing distance must be positive and finite",
        ));
    }
    if !margin_degrees.is_finite() || !(0.0..90.0).contains(&margin_degrees) {
        return Err(GeometryError(
            "visual-angle margin must be finite and in [0, 90) degrees",
        ));
    }
    let margin_mm = viewing.distance_mm * margin_degrees.to_radians().tan();
    let x = margin_mm * (viewing.bounds.width() / viewing.width_mm);
    let y = margin_mm * (viewing.bounds.height() / viewing.height_mm);
    let inflated = Rect::new(
        minimap.left() - x,
        minimap.top() - y,
        minimap.right() + x,
        minimap.bottom() + y,
    )?;
    Ok(inflated.intersection(viewing.bounds))
}

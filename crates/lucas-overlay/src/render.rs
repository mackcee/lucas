//! Pure tiny-skia renderer. Buffers are premultiplied RGBA, transparent inside
//! the entire outward-rounded minimap rectangle and at the surface boundary.

use crate::{OverlayError, Pulse, PulseStyle, Rect, Result, Salience};
use std::time::Duration;
use tiny_skia::{BlendMode, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Validated, bounded raster geometry. The margin contains the full expanding
/// ripple and its glow; desktop clipping is left to the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    margin: u32,
}

impl Surface {
    pub fn for_target(rect: Rect) -> Result<Self> {
        let w = rect.right().ceil() - rect.left().floor();
        let h = rect.bottom().ceil() - rect.top().floor();
        let margin = (w.max(h) * 0.2).ceil().max(24.0) + 12.0;
        let left = rect.left().floor() - margin;
        let top = rect.top().floor() - margin;
        let width = w + margin * 2.0;
        let height = h + margin * 2.0;
        if width > 4096.0
            || height > 4096.0
            || left < i32::MIN as f64
            || top < i32::MIN as f64
            || left + width > i32::MAX as f64
            || top + height > i32::MAX as f64
        {
            return Err(OverlayError::InvalidRect);
        }
        Ok(Self {
            left: left as i32,
            top: top as i32,
            width: width as u32,
            height: height as u32,
            margin: margin as u32,
        })
    }

    pub fn left(self) -> i32 {
        self.left
    }
    pub fn top(self) -> i32 {
        self.top
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
    pub fn margin(self) -> u32 {
        self.margin
    }
}

/// Render one animation frame; time >= duration produces a fully clear frame.
pub fn frame(surface: Surface, pulse: Pulse, elapsed: Duration) -> Result<Pixmap> {
    let mut image = Pixmap::new(surface.width, surface.height).ok_or(OverlayError::InvalidRect)?;
    if elapsed.is_zero() || elapsed >= Pulse::DURATION {
        return Ok(image);
    }
    let t = elapsed.as_secs_f32() / Pulse::DURATION.as_secs_f32();
    let (opacity, width) = match pulse.salience {
        Salience::Minimal => (0.24, 1.0),
        Salience::Soft => (0.42, 1.5),
        Salience::Medium => (0.90, 4.0),
        Salience::Strong => (1.0, 6.0),
    };
    let margin = surface.margin as f32;
    let map = tiny_skia::Rect::from_ltrb(
        margin,
        margin,
        surface.width as f32 - margin,
        surface.height as f32 - margin,
    )
    .expect("validated surface has a positive map size");
    // Geometry is in physical pixels, not DIPs. Scale the stronger strokes
    // with minimap size so a 4K map does not get the same hairline as a small
    // desktop. Keep Soft/Minimal unchanged and the existing surface footprint.
    let prominent = matches!(pulse.salience, Salience::Strong | Salience::Medium);
    let width = if prominent {
        width * (map.width().min(map.height()) / 200.0).clamp(1.0, 2.5)
    } else {
        width
    };
    let base = 3.0 + width / 2.0;
    let travel = (map.width().min(map.height()) * 0.2).min(margin - base - width / 2.0 - 6.0);
    match pulse.style {
        PulseStyle::Ripple => {
            let count = pulse.ripple_count();
            for i in 0..count {
                let phase = if count == 1 {
                    t
                } else {
                    (t - i as f32 * 0.28) / 0.72
                };
                if !(0.0..1.0).contains(&phase) {
                    continue;
                }
                let expansion = base + travel * phase;
                // Keep the two waves visible longer, rather than spending most
                // of the 1.2-second cue as an almost-transparent outline.
                let fade = if prominent {
                    1.0 - phase
                } else {
                    (1.0 - phase).powi(2)
                };
                let alpha = opacity * (phase / 0.08).min(1.0) * fade;
                let path = PathBuilder::from_rect(expand(map, expansion));
                glow(&mut image, &path, pulse, width, alpha);
            }
        }
        PulseStyle::BreathingBorder | PulseStyle::CornerBrackets => {
            // Two smooth breaths, or one for the quieter levels. No strobing.
            let phase = (t * pulse.ripple_count() as f32).fract();
            let alpha = opacity * 16.0 * phase.powi(2) * (1.0 - phase).powi(2);
            let rect = expand(map, base);
            let path = if pulse.style == PulseStyle::CornerBrackets {
                brackets(rect)
            } else {
                PathBuilder::from_rect(rect)
            };
            glow(&mut image, &path, pulse, width, alpha);
        }
        PulseStyle::CometSweep => {
            let rect = expand(map, base);
            let envelope = (t / 0.1).min(1.0) * ((1.0 - t) / 0.2).min(1.0);
            // A short tapering tail traverses the rectangular perimeter once.
            for segment in 0..20 {
                let tail = segment as f32 / 20.0;
                let start = (t - tail * 0.18).rem_euclid(1.0);
                let end = (start - 0.009).rem_euclid(1.0);
                // Sample each short segment so corners never cut across the map.
                let mut path = PathBuilder::new();
                let (x, y) = perimeter(rect, end);
                path.move_to(x, y);
                for step in 1..=4 {
                    let (x, y) = perimeter(rect, (end + step as f32 * 0.009 / 4.0).fract());
                    path.line_to(x, y);
                }
                glow(
                    &mut image,
                    &path.finish().unwrap(),
                    pulse,
                    width,
                    opacity * envelope * (1.0 - tail).powi(2),
                );
            }
        }
    }
    // Strict exclusion, including pixels intersecting fractional target edges.
    image.fill_rect(
        map,
        &Paint {
            blend_mode: BlendMode::Clear,
            anti_alias: false,
            ..Paint::default()
        },
        Transform::identity(),
        None,
    );
    Ok(image)
}

fn expand(rect: tiny_skia::Rect, by: f32) -> tiny_skia::Rect {
    tiny_skia::Rect::from_ltrb(
        rect.left() - by,
        rect.top() - by,
        rect.right() + by,
        rect.bottom() + by,
    )
    .unwrap()
}

fn glow(image: &mut Pixmap, path: &tiny_skia::Path, pulse: Pulse, width: f32, alpha: f32) {
    for (extra, strength) in [(10.0, 0.04), (6.0, 0.09), (3.0, 0.2), (0.0, 1.0)] {
        let mut paint = Paint::default();
        paint.set_color_rgba8(
            pulse.colour.0,
            pulse.colour.1,
            pulse.colour.2,
            (alpha * strength * 255.0).round() as u8,
        );
        image.stroke_path(
            path,
            &paint,
            &Stroke {
                width: width + extra,
                line_join: LineJoin::Round,
                line_cap: LineCap::Round,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
}

fn brackets(r: tiny_skia::Rect) -> tiny_skia::Path {
    let length = r.width().min(r.height()) * 0.18;
    let mut p = PathBuilder::new();
    for (x, y, dx, dy) in [
        (r.left(), r.top(), 1.0, 1.0),
        (r.right(), r.top(), -1.0, 1.0),
        (r.right(), r.bottom(), -1.0, -1.0),
        (r.left(), r.bottom(), 1.0, -1.0),
    ] {
        p.move_to(x + dx * length, y);
        p.line_to(x, y);
        p.line_to(x, y + dy * length);
    }
    p.finish().unwrap()
}

fn perimeter(r: tiny_skia::Rect, phase: f32) -> (f32, f32) {
    let distance = phase * 2.0 * (r.width() + r.height());
    if distance < r.width() {
        (r.left() + distance, r.top())
    } else if distance < r.width() + r.height() {
        (r.right(), r.top() + distance - r.width())
    } else if distance < 2.0 * r.width() + r.height() {
        (r.right() - (distance - r.width() - r.height()), r.bottom())
    } else {
        (
            r.left(),
            r.bottom() - (distance - 2.0 * r.width() - r.height()),
        )
    }
}

/// UpdateLayeredWindow uses premultiplied BGRA. Swizzling must not unpremultiply.
pub fn copy_bgra(image: &Pixmap, destination: &mut [u8]) {
    assert_eq!(image.data().len(), destination.len());
    for (rgba, bgra) in image
        .data()
        .chunks_exact(4)
        .zip(destination.chunks_exact_mut(4))
    {
        bgra.copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
    }
}

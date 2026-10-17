use lucas_overlay::PulseStyle as RenderStyle;
use lucas_store::PulseStyle as StoredStyle;

pub fn pulse_style(style: StoredStyle) -> RenderStyle {
    match style {
        StoredStyle::Ripple => RenderStyle::Ripple,
        StoredStyle::BreathingBorder => RenderStyle::BreathingBorder,
        StoredStyle::CornerBrackets => RenderStyle::CornerBrackets,
        StoredStyle::CometSweep => RenderStyle::CometSweep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_styles_map_to_the_matching_renderer_styles() {
        for rendered in [
            RenderStyle::Ripple,
            RenderStyle::BreathingBorder,
            RenderStyle::CornerBrackets,
            RenderStyle::CometSweep,
        ] {
            let stored = match rendered {
                RenderStyle::Ripple => StoredStyle::Ripple,
                RenderStyle::BreathingBorder => StoredStyle::BreathingBorder,
                RenderStyle::CornerBrackets => StoredStyle::CornerBrackets,
                RenderStyle::CometSweep => StoredStyle::CometSweep,
            };
            assert_eq!(pulse_style(stored), rendered);
        }
    }
}

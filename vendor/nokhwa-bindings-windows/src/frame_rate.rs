// LUCAS patch; licensed under Apache-2.0 like the vendored bindings.

// MF_MT_FRAME_RATE packs the numerator into the high 32 bits and the
// denominator into the low 32 bits. Match compatible_format_list: only a
// denominator of 1 is supported; 0 marks an unsupported frame rate.
pub(crate) fn decode_frame_rate(ratio: u64) -> u32 {
    let numerator = (ratio >> 32) as u32;
    let denominator = ratio as u32;
    if denominator == 1 { numerator } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::decode_frame_rate;

    #[test]
    fn c270_30_fps_is_not_the_denominator() {
        assert_eq!(decode_frame_rate((30_u64 << 32) | 1), 30);
    }

    #[test]
    fn integer_rates_use_the_numerator() {
        for fps in [0_u32, 1, 15, 30, 60, u32::MAX] {
            assert_eq!(decode_frame_rate((u64::from(fps) << 32) | 1), fps);
        }
    }

    #[test]
    fn unsupported_denominators_match_format_enumeration() {
        for (numerator, denominator) in [(30_u64, 0_u64), (30_000, 1001), (60, 2)] {
            assert_eq!(decode_frame_rate((numerator << 32) | denominator), 0);
        }
    }
}

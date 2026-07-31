//! IEEE 754 binary16 conversions (RFC 8949 §3.1, major type 7).
//!
//! Rust's `f16` is still unstable and the `half` crate is off limits, so this is
//! done by hand on the bit patterns. `core` has no `powi`, which rules out the
//! arithmetic shortcuts — everything here is shifts and masks.
//!
//! Rounding is round-to-nearest-even, matching what upstream gets from its
//! `(_Float16)x` cast on hardware that has one.

/// binary16 -> binary32. Always exact: every half value fits in an f32.
pub fn decode(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = (h >> 10) & 0x1f;
    let mant = (h & 0x3ff) as u32;

    match exp {
        // Zero, or subnormal with value mant * 2^-24.
        0 if mant == 0 => f32::from_bits(sign),
        0 => {
            // Renormalise: shift the mantissa up until the implicit bit appears,
            // then drop it and pay for each shift out of the exponent.
            let mut m = mant;
            let mut shifts = 0;
            while m & 0x400 == 0 {
                m <<= 1;
                shifts += 1;
            }
            f32::from_bits(sign | ((113 - shifts) << 23) | ((m & 0x3ff) << 13))
        }
        // Infinity and NaN. Shifting the payload keeps quiet NaNs quiet.
        0x1f => f32::from_bits(sign | 0x7f80_0000 | (mant << 13)),
        // 127 - 15 = 112 rebiases the exponent.
        _ => f32::from_bits(sign | ((exp as u32 + 112) << 23) | (mant << 13)),
    }
}

/// binary32 -> binary16, round-to-nearest-even. Lossy, and deliberately so.
pub fn encode(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let mant = bits & 0x7f_ffff;
    let unbiased = ((bits >> 23) & 0xff) as i32 - 127;

    // Infinity and NaN. A NaN must stay a NaN: truncating its payload to zero
    // would silently turn it into an infinity, so force a bit on.
    if unbiased == 128 {
        return sign
            | 0x7c00
            | if mant != 0 {
                0x200 | (mant >> 13) as u16
            } else {
                0
            };
    }

    if unbiased > 15 {
        return sign | 0x7c00; // overflows the exponent range
    }

    if unbiased >= -14 {
        let h = sign | (((unbiased + 15) as u16) << 10) | (mant >> 13) as u16;
        return h + round_up(mant, 13);
    }

    // Subnormal half, or small enough to flush to zero. The value we want is
    // mant16 = round(x / 2^-24), which is the full 24-bit significand shifted
    // right by however far the exponent falls short.
    let shift = (-unbiased - 1) as u32;
    if shift > 24 {
        return sign; // below half of the smallest subnormal
    }
    let full = mant | 0x80_0000;
    sign | ((full >> shift) as u16 + round_up(full, shift))
}

/// The carry to add after a right shift of `bits`, under round-to-nearest-even.
///
/// Ties go to even, which is the whole reason this is not just `>> n`.
fn round_up(value: u32, shift: u32) -> u16 {
    let half = 1u32 << (shift - 1);
    let rest = value & ((1u32 << shift) - 1);
    let odd = (value >> shift) & 1;
    u16::from(rest > half || (rest == half && odd == 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_exactly() {
        // Every half bit pattern must survive decode -> encode unchanged, except
        // NaN payloads, which are compared as bits after normalising the sign.
        for h in 0u16..=0xffff {
            let exp = (h >> 10) & 0x1f;
            let is_nan = exp == 0x1f && (h & 0x3ff) != 0;
            if is_nan {
                continue;
            }
            assert_eq!(encode(decode(h)), h, "half {h:#06x} did not round-trip");
        }
    }

    #[test]
    fn known_values() {
        assert_eq!(decode(0x0000), 0.0);
        assert_eq!(decode(0x8000), -0.0);
        assert_eq!(decode(0x3c00), 1.0);
        assert_eq!(decode(0xc000), -2.0);
        assert_eq!(decode(0x7bff), 65504.0); // largest finite half
        assert_eq!(decode(0x0400), 6.1035156e-5); // smallest normal
        assert_eq!(decode(0x0001), 5.9604645e-8); // smallest subnormal
        assert!(decode(0x7c00).is_infinite() && decode(0x7c00) > 0.0);
        assert!(decode(0xfc00).is_infinite() && decode(0xfc00) < 0.0);
        assert!(decode(0x7e00).is_nan());
    }

    #[test]
    fn rounds_to_nearest_even() {
        // Exactly halfway between 2048 and 2050 (half has 11 bits of precision,
        // so 2049 is not representable). Ties go to the even neighbour.
        assert_eq!(encode(2049.0), encode(2048.0));
        assert_eq!(encode(2051.0), encode(2052.0));
        // Half of the smallest subnormal is a tie against zero, which is even.
        assert_eq!(encode(2.9802322e-8), 0);
        // Just over that tie rounds up to the smallest subnormal.
        assert_eq!(encode(3.5e-8), 1);
    }

    #[test]
    fn saturates_and_flushes() {
        assert_eq!(encode(70000.0), 0x7c00); // overflow -> +inf
        assert_eq!(encode(-70000.0), 0xfc00);
        assert_eq!(encode(1e-10), 0x0000); // underflow -> +0
        assert_eq!(encode(-1e-10), 0x8000); // sign survives
        assert!(decode(encode(f32::NAN)).is_nan());
    }
}

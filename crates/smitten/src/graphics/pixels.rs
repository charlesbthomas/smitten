//! Reading and writing one pixel in each [`PixelFormat`].
//!
//! Channels a format lacks read back as 0 for green and blue and 1 for
//! alpha. Normalized channels are clamped to 0 to 1 when written;
//! floating-point ones are not.

use smitten_core::PixelFormat;

use crate::Color;

/// The color stored in `bytes`, which hold one pixel in `format`.
pub(crate) fn decode(format: PixelFormat, bytes: &[u8]) -> Color {
    use PixelFormat::*;
    let u8n = |index: usize| f32::from(bytes[index]) / 255.0;
    let u16n = |index: usize| f32::from(u16_at(bytes, index)) / 65535.0;
    let f16 = |index: usize| f16_to_f32(u16_at(bytes, index));
    let f32 =
        |index: usize| f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    let [r, g, b, a] = match format {
        R8 => [u8n(0), 0.0, 0.0, 1.0],
        Rg8 => [u8n(0), u8n(1), 0.0, 1.0],
        Rgba8 | Srgba8 => [u8n(0), u8n(1), u8n(2), u8n(3)],
        R16 => [u16n(0), 0.0, 0.0, 1.0],
        Rg16 => [u16n(0), u16n(1), 0.0, 1.0],
        Rgba16 => [u16n(0), u16n(1), u16n(2), u16n(3)],
        R16f => [f16(0), 0.0, 0.0, 1.0],
        Rg16f => [f16(0), f16(1), 0.0, 1.0],
        Rgba16f => [f16(0), f16(1), f16(2), f16(3)],
        R32f => [f32(0), 0.0, 0.0, 1.0],
        Rg32f => [f32(0), f32(1), 0.0, 1.0],
        Rgba32f => [f32(0), f32(1), f32(2), f32(3)],
        Rgb10a2 => {
            let packed = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let channel = |shift: u32, bits: u32| {
                ((packed >> shift) & ((1 << bits) - 1)) as f32 / ((1 << bits) - 1) as f32
            };
            [
                channel(0, 10),
                channel(10, 10),
                channel(20, 10),
                channel(30, 2),
            ]
        }
        Rg11b10f => {
            let packed = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            [
                f16_to_f32((((packed & 0x7ff) as u16) << 4) & 0x7fff),
                f16_to_f32(((((packed >> 11) & 0x7ff) as u16) << 4) & 0x7fff),
                f16_to_f32((((packed >> 22) & 0x3ff) as u16) << 5),
                1.0,
            ]
        }
        Depth16 | Depth24 | Depth32f | Depth24Stencil8 | Depth32fStencil8 => {
            unreachable!("depth formats hold no colors")
        }
    };
    Color::rgba(r, g, b, a)
}

/// Writes `color` into `bytes`, which hold one pixel in `format`.
pub(crate) fn encode(format: PixelFormat, color: Color, bytes: &mut [u8]) {
    use PixelFormat::*;
    let channels = [color.r, color.g, color.b, color.a];
    let count = format.components() as usize;
    match format {
        R8 | Rg8 | Rgba8 | Srgba8 => {
            for (byte, value) in bytes.iter_mut().zip(&channels[..count]) {
                *byte = unorm(*value, 255) as u8;
            }
        }
        R16 | Rg16 | Rgba16 => {
            for (index, value) in channels[..count].iter().enumerate() {
                let value = unorm(*value, 65535) as u16;
                bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
        R16f | Rg16f | Rgba16f => {
            for (index, value) in channels[..count].iter().enumerate() {
                let value = f32_to_f16(*value);
                bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
        R32f | Rg32f | Rgba32f => {
            for (index, value) in channels[..count].iter().enumerate() {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        Rgb10a2 => {
            let packed = unorm(color.r, 1023)
                | unorm(color.g, 1023) << 10
                | unorm(color.b, 1023) << 20
                | unorm(color.a, 3) << 30;
            bytes[..4].copy_from_slice(&packed.to_le_bytes());
        }
        Rg11b10f => {
            let packed = u32::from(f32_to_uf11(color.r))
                | u32::from(f32_to_uf11(color.g)) << 11
                | u32::from(f32_to_uf10(color.b)) << 22;
            bytes[..4].copy_from_slice(&packed.to_le_bytes());
        }
        Depth16 | Depth24 | Depth32f | Depth24Stencil8 | Depth32fStencil8 => {
            unreachable!("depth formats hold no colors")
        }
    }
}

fn u16_at(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([bytes[index * 2], bytes[index * 2 + 1]])
}

/// `value`, clamped to 0 to 1, as a whole number from 0 to `max`.
fn unorm(value: f32, max: u32) -> u32 {
    // NaN clamps to NaN, which `as` turns into 0.
    (value.clamp(0.0, 1.0) * max as f32).round() as u32
}

/// The nearest half-precision float to `value`, rounding ties to even.
pub(crate) fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        // Infinity stays infinity, and NaN stays NaN.
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    let (kept, shift) = if half_exponent <= 0 {
        // Too small for a normal half: a subnormal, or zero.
        if half_exponent < -10 {
            return sign;
        }
        (mantissa | 0x80_0000, (14 - half_exponent) as u32)
    } else {
        ((half_exponent as u32) << 23 | mantissa, 13)
    };
    let truncated = kept >> shift;
    let rest = kept & ((1 << shift) - 1);
    let halfway = 1 << (shift - 1);
    let rounded = if rest > halfway || (rest == halfway && truncated & 1 == 1) {
        // A carry into the exponent is still right, up to infinity.
        truncated + 1
    } else {
        truncated
    };
    sign | rounded as u16
}

/// The value of a half-precision float.
pub(crate) fn f16_to_f32(half: u16) -> f32 {
    let sign = u32::from(half & 0x8000) << 16;
    let exponent = u32::from((half >> 10) & 0x1f);
    let mantissa = u32::from(half & 0x3ff);
    let bits = match exponent {
        0 => {
            // Zero or a subnormal: mantissa times 2^-24.
            let magnitude = mantissa as f32 / 16_777_216.0;
            return if sign != 0 { -magnitude } else { magnitude };
        }
        0x1f => sign | 0x7f80_0000 | mantissa << 13,
        _ => sign | (exponent + 112) << 23 | mantissa << 13,
    };
    f32::from_bits(bits)
}

/// A non-negative half with its lowest `dropped` mantissa bits rounded
/// away, which is how the 11- and 10-bit floats of `rg11b10f` are made.
fn unsigned_small_float(value: f32, dropped: u32) -> u16 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    let half = f32_to_f16(value);
    if half >= 0x7c00 {
        return 0x7c00 >> dropped;
    }
    ((half + (1 << (dropped - 1))) >> dropped).min(0x7c00 >> dropped)
}

fn f32_to_uf11(value: f32) -> u16 {
    unsigned_small_float(value, 4)
}

fn f32_to_uf10(value: f32) -> u16 {
    unsigned_small_float(value, 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_trip_and_round_to_nearest() {
        for value in [0.0, 1.0, -2.5, 0.5, 65504.0, 6.1035156e-5, 5.9604645e-8] {
            assert_eq!(f16_to_f32(f32_to_f16(value)), value, "{value}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(-0.0), 0x8000);
        assert_eq!(f32_to_f16(100_000.0), 0x7c00);
        assert_eq!(f16_to_f32(f32_to_f16(f32::INFINITY)), f32::INFINITY);
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
        // 1 + 2^-11 is halfway between two halves and rounds to the even one.
        assert_eq!(f32_to_f16(1.0 + 1.0 / 2048.0), 0x3c00);
        assert_eq!(f32_to_f16(1.0 + 3.0 / 2048.0), 0x3c02);
        // Too small for even a subnormal.
        assert_eq!(f32_to_f16(1e-9), 0);
    }

    fn round_trip(format: PixelFormat, color: Color) -> Color {
        let mut bytes = vec![0; format.bytes_per_pixel()];
        encode(format, color, &mut bytes);
        decode(format, &bytes)
    }

    #[test]
    fn every_color_format_keeps_what_it_can_hold() {
        let color = Color::rgba(0.25, 0.5, 0.75, 1.0);
        for format in PixelFormat::ALL.into_iter().filter(|f| f.is_color()) {
            let back = round_trip(format, color);
            let tolerance = match format {
                PixelFormat::Rgb10a2 => 1.0 / 1023.0,
                PixelFormat::Rg11b10f => 1.0 / 32.0,
                _ => 1.0 / 255.0,
            };
            let expected = [color.r, color.g, color.b, color.a];
            let got = [back.r, back.g, back.b, back.a];
            for channel in 0..format.components() as usize {
                assert!(
                    (got[channel] - expected[channel]).abs() <= tolerance,
                    "{format}: channel {channel} is {}, not {}",
                    got[channel],
                    expected[channel]
                );
            }
        }
    }

    #[test]
    fn missing_channels_read_as_zero_color_and_opaque_alpha() {
        let back = round_trip(PixelFormat::R8, Color::rgba(1.0, 1.0, 1.0, 0.0));
        assert_eq!(back, Color::rgba(1.0, 0.0, 0.0, 1.0));
        let back = round_trip(PixelFormat::Rg11b10f, Color::rgba(0.0, 0.0, 0.0, 0.0));
        assert_eq!(back.a, 1.0);
    }

    #[test]
    fn floats_are_not_clamped_but_normalized_channels_are() {
        let bright = Color::rgba(4.0, -1.0, 0.5, 2.0);
        assert_eq!(round_trip(PixelFormat::Rgba32f, bright), bright);
        assert_eq!(round_trip(PixelFormat::Rgba16f, bright), bright);
        assert_eq!(
            round_trip(PixelFormat::Rgba16, bright),
            Color::rgba(1.0, 0.0, 32768.0 / 65535.0, 1.0)
        );
        // rg11b10f has no sign, so negatives become 0.
        let back = round_trip(PixelFormat::Rg11b10f, bright);
        assert_eq!((back.r, back.g, back.b), (4.0, 0.0, 0.5));
    }
}

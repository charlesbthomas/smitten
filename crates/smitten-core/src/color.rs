/// An RGBA color with components from 0.0 to 1.0, as in LÖVE 11.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red, from 0.0 to 1.0.
    pub r: f32,
    /// Green, from 0.0 to 1.0.
    pub g: f32,
    /// Blue, from 0.0 to 1.0.
    pub b: f32,
    /// Alpha, from 0.0 (transparent) to 1.0 (opaque).
    pub a: f32,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);

    /// An opaque color.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// A color with an alpha, from 0.0 (transparent) to 1.0 (opaque).
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// The same color with its alpha replaced.
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// The color `t` of the way from this one to `other`: this color at
    /// 0.0, `other` at 1.0, an even mix at 0.5. Alpha is blended too.
    /// `t` outside 0.0 to 1.0 is cut to that range.
    pub fn lerp(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |from: f32, to: f32| from + (to - from) * t;
        Self {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
            a: mix(self.a, other.a),
        }
    }

    /// A color from components from 0 to 255, as image editors show them,
    /// the equivalent of `love.math.colorFromBytes`.
    #[doc(alias = "love.math.colorFromBytes")]
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        let unit = |byte: u8| f32::from(byte) / 255.0;
        Self::rgba(unit(r), unit(g), unit(b), unit(a))
    }

    /// An opaque color from components from 0 to 255.
    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::from_rgba8(r, g, b, 255)
    }

    /// The components from 0 to 255, the equivalent of
    /// `love.math.colorToBytes`. Components outside 0.0 to 1.0 are cut to
    /// that range, and each is rounded to the nearest whole number.
    #[doc(alias = "love.math.colorToBytes")]
    pub fn to_rgba8(self) -> [u8; 4] {
        let byte = |unit: f32| (unit.clamp(0.0, 1.0) * 255.0).round() as u8;
        [byte(self.r), byte(self.g), byte(self.b), byte(self.a)]
    }

    /// Converts the color from the usual sRGB encoding to linear light,
    /// where adding and averaging behave physically, the equivalent of
    /// `love.math.gammaToLinear`. Alpha is kept as it is.
    #[doc(alias = "love.math.gammaToLinear")]
    pub fn to_linear(self) -> Self {
        let linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        Self::rgba(linear(self.r), linear(self.g), linear(self.b), self.a)
    }

    /// Converts the color from linear light back to sRGB, the equivalent
    /// of `love.math.linearToGamma`. Alpha is kept as it is.
    #[doc(alias = "love.math.linearToGamma")]
    pub fn to_gamma(self) -> Self {
        let gamma = |c: f32| {
            if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
        };
        Self::rgba(gamma(self.r), gamma(self.g), gamma(self.b), self.a)
    }
}

impl Default for Color {
    /// White, the color drawing starts with.
    fn default() -> Self {
        Color::WHITE
    }
}

impl From<[f32; 4]> for Color {
    fn from([r, g, b, a]: [f32; 4]) -> Self {
        Color::rgba(r, g, b, a)
    }
}

impl From<[f32; 3]> for Color {
    fn from([r, g, b]: [f32; 3]) -> Self {
        Color::rgb(r, g, b)
    }
}

impl From<Color> for [f32; 4] {
    fn from(color: Color) -> Self {
        [color.r, color.g, color.b, color.a]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_convert_to_and_from_bytes() {
        let color = Color::from_rgba8(255, 0, 51, 128);
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 0.0);
        assert!((color.b - 0.2).abs() < 1e-6);
        assert_eq!(color.to_rgba8(), [255, 0, 51, 128]);
        assert_eq!(
            Color::rgba(2.0, -1.0, 0.5, 1.0).to_rgba8(),
            [255, 0, 128, 255]
        );
    }

    #[test]
    fn gamma_and_linear_undo_each_other() {
        for step in 0..=10 {
            let value = step as f32 / 10.0;
            let round_trip = Color::rgba(value, value, value, 0.5).to_linear().to_gamma();
            assert!((round_trip.r - value).abs() < 1e-5);
            assert_eq!(round_trip.a, 0.5);
        }
        assert!((Color::rgb(0.5, 0.5, 0.5).to_linear().g - 0.214).abs() < 0.001);
    }

    #[test]
    fn colors_blend_and_take_a_new_alpha() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgba(0.0, 0.0, 1.0, 0.0);
        assert_eq!(red.lerp(blue, 0.0), red);
        assert_eq!(red.lerp(blue, 1.0), blue);
        assert_eq!(red.lerp(blue, 0.5), Color::rgba(0.5, 0.0, 0.5, 0.5));
        assert_eq!(red.lerp(blue, 7.0), blue);
        assert_eq!(red.with_alpha(0.25), Color::rgba(1.0, 0.0, 0.0, 0.25));
    }
}

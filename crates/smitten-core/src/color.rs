/// An RGBA color with components from 0.0 to 1.0, as in LÖVE 11.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);

    /// An opaque color.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;

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

/// How something drawn is combined with what is already there, the
/// equivalent of LÖVE's `BlendMode`.
///
/// Each mode also has a [`BlendAlphaMode`], which says whether the colors
/// drawn are already multiplied by their alpha. `set_blend_mode` picks the
/// usual one for the mode; `set_blend_mode_ex` takes it as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    /// Ordinary transparency. The default, as in LÖVE.
    #[default]
    Alpha,
    /// Transparency for colors that are already multiplied by their alpha.
    /// Use this to draw a canvas, whose contents are stored that way.
    ///
    /// The same as `Alpha` with [`BlendAlphaMode::Premultiplied`], kept as
    /// a mode of its own because it is so common.
    Premultiplied,
    /// Adds to what is there, brightening it. Good for glows and fire.
    Add,
    /// Subtracts from what is there, darkening it.
    Subtract,
    /// Multiplies with what is there. White changes nothing, black gives
    /// black. Always premultiplied, as in LÖVE.
    Multiply,
    /// Overwrites what is there, including with transparent pixels.
    Replace,
    /// The inverse of multiplying the inverses: brightens what is there,
    /// but never past white. Black changes nothing. Softer than `Add` for
    /// light.
    Screen,
    /// Keeps the brighter of what is drawn and what is there, channel by
    /// channel. Always premultiplied, as in LÖVE.
    Lighten,
    /// Keeps the darker of what is drawn and what is there, channel by
    /// channel. Always premultiplied, as in LÖVE.
    Darken,
}

impl BlendMode {
    /// Every blend mode, in declaration order.
    pub const ALL: [BlendMode; 9] = [
        BlendMode::Alpha,
        BlendMode::Premultiplied,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Multiply,
        BlendMode::Replace,
        BlendMode::Screen,
        BlendMode::Lighten,
        BlendMode::Darken,
    ];

    /// The alpha mode `set_blend_mode` uses with this mode:
    /// [`BlendAlphaMode::Premultiplied`] for the modes that only work that
    /// way, and [`BlendAlphaMode::AlphaMultiply`] for the rest, as LÖVE's
    /// default.
    pub fn default_alpha_mode(self) -> BlendAlphaMode {
        if self.needs_premultiplied() {
            BlendAlphaMode::Premultiplied
        } else {
            BlendAlphaMode::AlphaMultiply
        }
    }

    /// Whether this mode only works with colors that are already
    /// multiplied by their alpha. LÖVE refuses `Multiply`, `Lighten` and
    /// `Darken` with [`BlendAlphaMode::AlphaMultiply`].
    pub fn needs_premultiplied(self) -> bool {
        matches!(
            self,
            BlendMode::Premultiplied | BlendMode::Multiply | BlendMode::Lighten | BlendMode::Darken
        )
    }
}

/// Whether the colors drawn are already multiplied by their alpha, the
/// equivalent of LÖVE's `BlendAlphaMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendAlphaMode {
    /// The colors are multiplied by their alpha as they are drawn, so
    /// that a half transparent red adds half as much red. The default, as
    /// in LÖVE.
    #[default]
    AlphaMultiply,
    /// The colors already are, as the contents of a canvas are. Draw a
    /// canvas with see-through parts this way, in any blend mode.
    Premultiplied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_modes_that_need_it_default_to_premultiplied() {
        let premultiplied: Vec<_> = BlendMode::ALL
            .into_iter()
            .filter(|mode| mode.default_alpha_mode() == BlendAlphaMode::Premultiplied)
            .collect();
        assert_eq!(
            premultiplied,
            [
                BlendMode::Premultiplied,
                BlendMode::Multiply,
                BlendMode::Lighten,
                BlendMode::Darken
            ]
        );
    }
}

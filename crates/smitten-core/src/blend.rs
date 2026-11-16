/// How something drawn is combined with what is already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    /// Ordinary transparency. The default, as in LÖVE.
    #[default]
    Alpha,
    /// Transparency for colors that are already multiplied by their alpha.
    /// Use this to draw a canvas, whose contents are stored that way.
    Premultiplied,
    /// Adds to what is there, brightening it. Good for glows and fire.
    Add,
    /// Subtracts from what is there, darkening it.
    Subtract,
    /// Multiplies with what is there. White changes nothing, black gives
    /// black.
    Multiply,
    /// Overwrites what is there, including with transparent pixels.
    Replace,
}

impl BlendMode {
    /// Every blend mode, in declaration order.
    pub const ALL: [BlendMode; 6] = [
        BlendMode::Alpha,
        BlendMode::Premultiplied,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Multiply,
        BlendMode::Replace,
    ];
}

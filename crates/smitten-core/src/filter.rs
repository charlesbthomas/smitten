/// How a texture is sampled when it is drawn scaled or rotated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Filter {
    /// Smooth blending between pixels. The default, as in LÖVE.
    #[default]
    Linear,
    /// Hard pixel edges, for pixel art.
    Nearest,
}

/// What a texture shows beyond its own edges, when a quad or a mesh asks
/// for more of it than there is. LÖVE's `WrapMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Wrap {
    /// The edge pixels are stretched outward. The default, as in LÖVE.
    #[default]
    Clamp,
    /// The texture starts again, like tiles.
    Repeat,
    /// The texture starts again, flipped each time so its edges meet.
    MirroredRepeat,
    /// Transparent black beyond the edges, LÖVE's `clampzero`. Not every
    /// GPU can do it: where
    /// `graphics::is_supported(GraphicsFeature::ClampZero)` is false,
    /// [`Wrap::Clamp`] is used instead, as in LÖVE.
    ClampZero,
}

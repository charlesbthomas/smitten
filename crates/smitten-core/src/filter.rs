/// How a texture is sampled when it is drawn scaled or rotated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Filter {
    /// Smooth blending between pixels. The default, as in LÖVE.
    #[default]
    Linear,
    /// Hard pixel edges, for pixel art.
    Nearest,
}

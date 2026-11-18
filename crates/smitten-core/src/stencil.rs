/// What drawing does to the stencil values of the pixels it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StencilAction {
    /// Sets them to the given value. The default, as in LÖVE.
    #[default]
    Replace,
    /// Adds one, stopping at 255.
    Increment,
    /// Subtracts one, stopping at 0.
    Decrement,
    /// Adds one, wrapping from 255 back to 0.
    IncrementWrap,
    /// Subtracts one, wrapping from 0 back to 255.
    DecrementWrap,
    /// Flips every bit.
    Invert,
}

/// How a pixel's stencil value is compared with the test value to decide
/// whether the pixel is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareMode {
    /// Draw where the stencil value equals the test value.
    Equal,
    /// Draw where the stencil value differs from the test value.
    NotEqual,
    /// Draw where the stencil value is less than the test value.
    Less,
    /// Draw where the stencil value is less than or equal to the test value.
    LessEqual,
    /// Draw where the stencil value is greater than the test value.
    Greater,
    /// Draw where the stencil value is greater than or equal to the test
    /// value.
    GreaterEqual,
    /// Draw nowhere.
    Never,
    /// Draw everywhere. With [`StencilMode::Test`] this is the same as no
    /// test.
    Always,
}

/// How a draw call uses the stencil buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StencilMode {
    /// Draws normally and leaves stencil values alone.
    #[default]
    Off,
    /// Changes stencil values and draws no color.
    Write(StencilAction),
    /// Draws only the pixels whose stencil value passes the comparison.
    Test(CompareMode),
}

/// How draws use the depth buffer, the equivalent of LÖVE's depth mode.
///
/// Each pixel drawn has a depth from 0, nearest, to 1, farthest. A pixel
/// is drawn only where its depth compares with the depth already there as
/// `compare` says, and with `write` its depth replaces it. The default,
/// [`CompareMode::Always`] without writing, leaves depth out entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthMode {
    /// [`CompareMode::Less`] draws where the new pixel is nearer than
    /// what is there, and so on.
    pub compare: CompareMode,
    /// Whether drawing also stores its depth, for later draws to test against.
    pub write: bool,
}

impl Default for DepthMode {
    fn default() -> Self {
        DepthMode {
            compare: CompareMode::Always,
            write: false,
        }
    }
}

impl DepthMode {
    /// Whether the depth buffer is read or written at all.
    pub fn is_used(self) -> bool {
        self.compare != CompareMode::Always || self.write
    }
}

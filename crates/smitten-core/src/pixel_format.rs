/// How the pixels of an `ImageData`, an image or a canvas are stored.
///
/// In the names, the letters say which channels there are, the number how
/// many bits each has, and `f` that they are floating point. Without `f`,
/// each channel holds a number from 0 to 1. The depth and stencil formats
/// are only for canvases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PixelFormat {
    /// One 8-bit channel, red.
    R8,
    /// Two 8-bit channels, red and green.
    Rg8,
    /// Four 8-bit channels. The format everything starts out in.
    #[default]
    Rgba8,
    /// Four 8-bit channels, with red, green and blue stored sRGB-encoded
    /// so that dark colors get more of the 256 steps. The GPU decodes them
    /// to linear values when it reads them, and encodes them when it
    /// writes them.
    Srgba8,
    /// One 16-bit channel.
    R16,
    /// Two 16-bit channels.
    Rg16,
    /// Four 16-bit channels.
    Rgba16,
    /// One 16-bit floating-point channel.
    R16f,
    /// Two 16-bit floating-point channels.
    Rg16f,
    /// Four 16-bit floating-point channels, for colors brighter than white
    /// (HDR) at half the memory of [`PixelFormat::Rgba32f`].
    Rgba16f,
    /// One 32-bit floating-point channel.
    R32f,
    /// Two 32-bit floating-point channels.
    Rg32f,
    /// Four 32-bit floating-point channels.
    Rgba32f,
    /// 10 bits each of red, green and blue, and 2 of alpha, in 4 bytes.
    Rgb10a2,
    /// Unsigned floating-point red and green with 11 bits each and blue
    /// with 10, in 4 bytes. It has no alpha.
    Rg11b10f,
    /// A 16-bit depth buffer.
    Depth16,
    /// A 24-bit depth buffer.
    Depth24,
    /// A 32-bit floating-point depth buffer.
    Depth32f,
    /// A 24-bit depth buffer with an 8-bit stencil buffer.
    Depth24Stencil8,
    /// A 32-bit floating-point depth buffer with an 8-bit stencil buffer.
    Depth32fStencil8,
}

impl PixelFormat {
    /// Every format, in the order they are declared.
    pub const ALL: [PixelFormat; 20] = [
        PixelFormat::R8,
        PixelFormat::Rg8,
        PixelFormat::Rgba8,
        PixelFormat::Srgba8,
        PixelFormat::R16,
        PixelFormat::Rg16,
        PixelFormat::Rgba16,
        PixelFormat::R16f,
        PixelFormat::Rg16f,
        PixelFormat::Rgba16f,
        PixelFormat::R32f,
        PixelFormat::Rg32f,
        PixelFormat::Rgba32f,
        PixelFormat::Rgb10a2,
        PixelFormat::Rg11b10f,
        PixelFormat::Depth16,
        PixelFormat::Depth24,
        PixelFormat::Depth32f,
        PixelFormat::Depth24Stencil8,
        PixelFormat::Depth32fStencil8,
    ];

    /// How many bytes one pixel takes.
    pub fn bytes_per_pixel(self) -> usize {
        use PixelFormat::*;
        match self {
            R8 => 1,
            Rg8 | R16 | R16f | Depth16 => 2,
            Rgba8 | Srgba8 | Rg16 | Rg16f | R32f | Rgb10a2 | Rg11b10f | Depth24 | Depth32f
            | Depth24Stencil8 => 4,
            Rgba16 | Rg32f | Rgba16f | Depth32fStencil8 => 8,
            Rgba32f => 16,
        }
    }

    /// How many channels a pixel has. Depth and stencil count as one
    /// channel each.
    pub fn components(self) -> u32 {
        use PixelFormat::*;
        match self {
            R8 | R16 | R16f | R32f | Depth16 | Depth24 | Depth32f => 1,
            Rg8 | Rg16 | Rg16f | Rg32f | Depth24Stencil8 | Depth32fStencil8 => 2,
            Rg11b10f => 3,
            Rgba8 | Srgba8 | Rgba16 | Rgba16f | Rgba32f | Rgb10a2 => 4,
        }
    }

    /// Whether this is a depth format, with or without stencil.
    pub fn is_depth(self) -> bool {
        use PixelFormat::*;
        matches!(
            self,
            Depth16 | Depth24 | Depth32f | Depth24Stencil8 | Depth32fStencil8
        )
    }

    /// Whether this format has a stencil part.
    pub fn has_stencil(self) -> bool {
        matches!(
            self,
            PixelFormat::Depth24Stencil8 | PixelFormat::Depth32fStencil8
        )
    }

    /// Whether this format holds colors, rather than depth or stencil.
    pub fn is_color(self) -> bool {
        !self.is_depth()
    }

    /// Whether the channels are stored sRGB-encoded.
    pub fn is_srgb(self) -> bool {
        self == PixelFormat::Srgba8
    }

    /// The format with the same storage but sRGB-encoded, if there is one.
    /// Images and canvases use it when gamma-correct rendering is on.
    pub fn to_srgb(self) -> Option<PixelFormat> {
        match self {
            PixelFormat::Rgba8 | PixelFormat::Srgba8 => Some(PixelFormat::Srgba8),
            _ => None,
        }
    }

    /// The format with the same storage but not sRGB-encoded.
    pub fn to_linear(self) -> PixelFormat {
        match self {
            PixelFormat::Srgba8 => PixelFormat::Rgba8,
            other => other,
        }
    }

    /// The format's name, such as `"rgba16f"`.
    pub fn name(self) -> &'static str {
        use PixelFormat::*;
        match self {
            R8 => "r8",
            Rg8 => "rg8",
            Rgba8 => "rgba8",
            Srgba8 => "srgba8",
            R16 => "r16",
            Rg16 => "rg16",
            Rgba16 => "rgba16",
            R16f => "r16f",
            Rg16f => "rg16f",
            Rgba16f => "rgba16f",
            R32f => "r32f",
            Rg32f => "rg32f",
            Rgba32f => "rgba32f",
            Rgb10a2 => "rgb10a2",
            Rg11b10f => "rg11b10f",
            Depth16 => "depth16",
            Depth24 => "depth24",
            Depth32f => "depth32f",
            Depth24Stencil8 => "depth24stencil8",
            Depth32fStencil8 => "depth32fstencil8",
        }
    }
}

impl std::str::FromStr for PixelFormat {
    type Err = crate::ParseNameError;

    /// Reads a format's name, such as `"rgba16f"`.
    fn from_str(name: &str) -> Result<PixelFormat, Self::Err> {
        PixelFormat::ALL
            .into_iter()
            .find(|format| format.name() == name)
            .ok_or_else(|| crate::ParseNameError::new("pixel format", name))
    }
}

impl std::fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::PixelFormat;

    #[test]
    fn every_format_round_trips_through_its_name() {
        for format in PixelFormat::ALL {
            assert_eq!(format.name().parse::<PixelFormat>(), Ok(format));
        }
        assert!("rgba9".parse::<PixelFormat>().is_err());
    }

    #[test]
    fn sizes_match_the_channels() {
        assert_eq!(PixelFormat::Rgba32f.bytes_per_pixel(), 16);
        assert_eq!(PixelFormat::Rg11b10f.components(), 3);
        assert!(PixelFormat::Depth24Stencil8.has_stencil());
        assert!(!PixelFormat::Depth24.has_stencil());
        assert!(PixelFormat::Depth16.is_depth());
        assert!(PixelFormat::Rgba16f.is_color());
        assert_eq!(PixelFormat::Rgba8.to_srgb(), Some(PixelFormat::Srgba8));
        assert_eq!(PixelFormat::Srgba8.to_linear(), PixelFormat::Rgba8);
        assert_eq!(PixelFormat::R16f.to_srgb(), None);
    }
}

//! Images stored compressed for the GPU, read from DDS and KTX files.

use std::fmt;
use std::path::Path;

use smitten_core::backend::CompressedFormat;

use super::ImageError;

/// Pixels compressed in a form the GPU reads directly, the equivalent of
/// LÖVE's `CompressedImageData`. Load one with
/// [`graphics::new_compressed_data`](super::new_compressed_data) and draw
/// it with [`Image::from_compressed_data`](super::Image::from_compressed_data),
/// or let [`graphics::new_image`](super::new_image) load a `.dds` or
/// `.ktx` file.
///
/// A compressed image takes a quarter to an eighth of the GPU memory of an
/// ordinary one and loads without decoding, but its pixels cannot be read
/// or changed. Smitten reads DDS files in the BC formats, and KTX 1
/// files in the BC and ASTC formats, with any mipmaps they hold. ETC
/// and PVRTC files are refused, since the GPU API Smitten uses has no way
/// to draw them. [`graphics::is_compressed_format_supported`](super::is_compressed_format_supported)
/// says which formats the GPU reads.
#[derive(Clone, PartialEq, Eq)]
pub struct CompressedImageData {
    format: CompressedFormat,
    width: u32,
    height: u32,
    /// The full-size picture, then each mipmap.
    levels: Vec<Vec<u8>>,
}

impl CompressedImageData {
    /// Reads compressed pixels from the bytes of a DDS or KTX file.
    pub fn from_bytes(bytes: &[u8]) -> Result<CompressedImageData, ImageError> {
        parse(bytes).map_err(ImageError::Invalid)
    }

    /// Loads a DDS or KTX file.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes.
    pub fn from_file(path: impl AsRef<Path>) -> Result<CompressedImageData, ImageError> {
        let path = path.as_ref();
        let bytes = crate::filesystem::read(path).map_err(|source| ImageError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        parse(&bytes).map_err(|message| ImageError::Decode {
            path: path.to_path_buf(),
            message,
        })
    }

    /// How the pixels are compressed.
    pub fn format(&self) -> CompressedFormat {
        self.format
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Size in pixels as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// How many mipmap levels the data holds, counting the full-size
    /// picture: 1 for none.
    pub fn mipmap_count(&self) -> u32 {
        self.levels.len() as u32
    }

    /// The compressed bytes of each level, the full-size picture first.
    pub(crate) fn levels(&self) -> Vec<&[u8]> {
        self.levels.iter().map(Vec::as_slice).collect()
    }
}

impl fmt::Debug for CompressedImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompressedImageData")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("mipmaps", &self.levels.len())
            .finish()
    }
}

/// Whether `bytes` look like a compressed texture file Smitten reads: DDS
/// or KTX.
pub(crate) fn is_compressed_file(bytes: &[u8]) -> bool {
    is_dds(bytes) || is_ktx(bytes)
}

/// Whether `bytes` look like a DDS file.
fn is_dds(bytes: &[u8]) -> bool {
    bytes.starts_with(b"DDS ")
}

/// The first 12 bytes of a KTX version 1 file.
const KTX_IDENTIFIER: [u8; 12] = [
    0xAB, b'K', b'T', b'X', b' ', b'1', b'1', 0xBB, b'\r', b'\n', 0x1A, b'\n',
];
/// The first bytes of a KTX version 2 file.
const KTX2_IDENTIFIER: &[u8] = b"\xABKTX 20\xBB";

/// Whether `bytes` look like a KTX file of either version.
fn is_ktx(bytes: &[u8]) -> bool {
    bytes.starts_with(&KTX_IDENTIFIER) || bytes.starts_with(KTX2_IDENTIFIER)
}

fn parse(bytes: &[u8]) -> Result<CompressedImageData, String> {
    if is_ktx(bytes) {
        parse_ktx(bytes)
    } else {
        parse_dds(bytes)
    }
}

/// The compressed format of an OpenGL internal format, as KTX files name
/// them. The sRGB variants are read as plain colors, as DDS ones are.
fn gl_format(internal: u32) -> Result<CompressedFormat, String> {
    use CompressedFormat::*;
    const ASTC: [CompressedFormat; 14] = [
        Astc4x4, Astc5x4, Astc5x5, Astc6x5, Astc6x6, Astc8x5, Astc8x6, Astc8x8, Astc10x5, Astc10x6,
        Astc10x8, Astc10x10, Astc12x10, Astc12x12,
    ];
    Ok(match internal {
        // S3TC, plain and sRGB.
        0x83F0 | 0x83F1 | 0x8C4C | 0x8C4D => Bc1,
        0x83F2 | 0x8C4E => Bc2,
        0x83F3 | 0x8C4F => Bc3,
        // RGTC.
        0x8DBB => Bc4,
        0x8DBD => Bc5,
        // BPTC.
        0x8E8C | 0x8E8D => Bc7,
        0x8E8E => Bc6hSigned,
        0x8E8F => Bc6h,
        // ASTC, plain from 0x93B0 and sRGB from 0x93D0, in the same order.
        0x93B0..=0x93BD => ASTC[(internal - 0x93B0) as usize],
        0x93D0..=0x93DD => ASTC[(internal - 0x93D0) as usize],
        // ETC1, ETC2 and EAC, and PVRTC.
        0x8D64 | 0x9270..=0x9279 => {
            return Err("ETC textures are not supported: the GPU API has no ETC formats".into());
        }
        0x8C00..=0x8C03 | 0x9137 | 0x9138 => {
            return Err(
                "PVRTC textures are not supported: the GPU API has no PVRTC formats".into(),
            );
        }
        0 => return Err("only compressed KTX files are supported, not raw pixels".into()),
        other => return Err(format!("OpenGL format {other:#06x} is not supported")),
    })
}

/// Reads a KTX version 1 file: a 64-byte header, key and value data,
/// then each mipmap level's size and bytes, padded to 4 bytes.
fn parse_ktx(bytes: &[u8]) -> Result<CompressedImageData, String> {
    if !bytes.starts_with(&KTX_IDENTIFIER) {
        return Err("KTX 2 files are not supported; save as KTX 1".to_string());
    }
    if bytes.len() < KTX_HEADER_SIZE {
        return Err("the KTX file ends inside its header".to_string());
    }
    let big_endian = match bytes[12..16] {
        [0x01, 0x02, 0x03, 0x04] => false,
        [0x04, 0x03, 0x02, 0x01] => true,
        _ => return Err("the KTX file has no valid byte order".to_string()),
    };
    let word = |offset: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let field = |index: usize| word(12 + 4 * index).unwrap_or(0);
    let gl_type = field(1);
    let internal = field(4);
    let (width, height, depth) = (field(6), field(7), field(8));
    let (array_elements, faces, mipmaps) = (field(9), field(10), field(11));
    let key_value_bytes = field(12) as usize;
    if gl_type != 0 {
        return Err("only compressed KTX files are supported, not raw pixels".to_string());
    }
    let format = gl_format(internal)?;
    if depth > 1 || array_elements > 0 || faces > 1 {
        return Err(
            "only single 2D images are supported, not arrays, cubes or volumes".to_string(),
        );
    }
    if width == 0 || height == 0 {
        return Err("the image has no pixels".to_string());
    }
    // 0 asks for the mipmaps to be made, which compressed pixels cannot be.
    let full_chain = 32 - width.max(height).leading_zeros();
    let mipmaps = mipmaps.clamp(1, full_chain);

    let mut levels = Vec::with_capacity(mipmaps as usize);
    let mut offset = KTX_HEADER_SIZE
        .checked_add(key_value_bytes)
        .ok_or("the KTX file's header is too long")?;
    for level in 0..mipmaps {
        let size = format.data_size((width >> level).max(1), (height >> level).max(1));
        let data = word(offset)
            .filter(|&stored| stored as usize == size)
            .and_then(|_| bytes.get(offset + 4..offset + 4 + size));
        let Some(data) = data else {
            if level == 0 {
                return Err("the KTX file ends before its pixels do".to_string());
            }
            // Keep the levels that are there.
            break;
        };
        levels.push(data.to_vec());
        offset += 4 + size.next_multiple_of(4);
    }
    Ok(CompressedImageData {
        format,
        width,
        height,
        levels,
    })
}

const KTX_HEADER_SIZE: usize = 64;

const HEADER_SIZE: usize = 128;
const DX10_HEADER_SIZE: usize = 20;
/// `dwFlags`: the mipmap count is filled in.
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
/// The pixel format's `dwFlags`: `dwFourCC` is filled in.
const DDPF_FOURCC: u32 = 0x4;
/// `dwCaps2`: the file is a cube map.
const DDSCAPS2_CUBEMAP: u32 = 0x200;
/// `dwCaps2`: the file is a volume.
const DDSCAPS2_VOLUME: u32 = 0x20_0000;
/// The DX10 header's `resourceDimension` for a 2D texture.
const DIMENSION_TEXTURE2D: u32 = 3;
/// The DX10 header's `miscFlag`: the file is a cube map.
const MISC_TEXTURECUBE: u32 = 0x4;

fn parse_dds(bytes: &[u8]) -> Result<CompressedImageData, String> {
    if !is_dds(bytes) || bytes.len() < HEADER_SIZE {
        return Err("not a DDS file".to_string());
    }
    let word = |offset: usize| {
        bytes
            .get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0)
    };
    let flags = word(8);
    let height = word(12);
    let width = word(16);
    let mipmaps = if flags & DDSD_MIPMAPCOUNT != 0 {
        word(28).max(1)
    } else {
        1
    };
    let format_flags = word(80);
    let four_cc = &bytes[84..88];
    let caps2 = word(112);
    if caps2 & (DDSCAPS2_CUBEMAP | DDSCAPS2_VOLUME) != 0 {
        return Err("cube map and volume DDS files are not supported".to_string());
    }
    if format_flags & DDPF_FOURCC == 0 {
        return Err("only compressed DDS files are supported, not raw pixels".to_string());
    }

    let mut data_start = HEADER_SIZE;
    let format = match four_cc {
        b"DXT1" => CompressedFormat::Bc1,
        b"DXT2" | b"DXT3" => CompressedFormat::Bc2,
        b"DXT4" | b"DXT5" => CompressedFormat::Bc3,
        b"ATI1" | b"BC4U" => CompressedFormat::Bc4,
        b"ATI2" | b"BC5U" => CompressedFormat::Bc5,
        b"DX10" => {
            if bytes.len() < HEADER_SIZE + DX10_HEADER_SIZE {
                return Err("the DDS file ends inside its header".to_string());
            }
            data_start += DX10_HEADER_SIZE;
            let dimension = word(132);
            let misc = word(136);
            let array_size = word(140);
            if dimension != DIMENSION_TEXTURE2D || misc & MISC_TEXTURECUBE != 0 || array_size > 1 {
                return Err(
                    "only single 2D images are supported, not arrays, cubes or volumes".to_string(),
                );
            }
            // DXGI_FORMAT values. The sRGB variants are read as plain
            // colors, as LÖVE does without gamma correction.
            match word(128) {
                71 | 72 => CompressedFormat::Bc1,
                74 | 75 => CompressedFormat::Bc2,
                77 | 78 => CompressedFormat::Bc3,
                80 => CompressedFormat::Bc4,
                83 => CompressedFormat::Bc5,
                95 => CompressedFormat::Bc6h,
                96 => CompressedFormat::Bc6hSigned,
                98 | 99 => CompressedFormat::Bc7,
                other => return Err(format!("DXGI format {other} is not supported")),
            }
        }
        other => {
            return Err(format!(
                "compression {:?} is not supported",
                String::from_utf8_lossy(other)
            ));
        }
    };
    if width == 0 || height == 0 {
        return Err("the image has no pixels".to_string());
    }
    // A chain longer than the image allows is cut short.
    let full_chain = 32 - width.max(height).leading_zeros();
    let mipmaps = mipmaps.min(full_chain);

    let mut levels = Vec::with_capacity(mipmaps as usize);
    let mut offset = data_start;
    for level in 0..mipmaps {
        let size = format.data_size((width >> level).max(1), (height >> level).max(1));
        let Some(data) = bytes.get(offset..offset + size) else {
            if level == 0 {
                return Err("the DDS file ends before its pixels do".to_string());
            }
            // Keep the levels that are there.
            break;
        };
        levels.push(data.to_vec());
        offset += size;
    }
    Ok(CompressedImageData {
        format,
        width,
        height,
        levels,
    })
}

/// Builds the bytes of a DDS file, for tests.
#[cfg(test)]
pub(crate) fn dds_file(four_cc: &[u8; 4], width: u32, height: u32, levels: &[&[u8]]) -> Vec<u8> {
    let mut file = vec![0; HEADER_SIZE];
    file[..4].copy_from_slice(b"DDS ");
    let mut put = |offset: usize, value: u32| {
        file[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    put(4, 124);
    put(8, 0x1007 | DDSD_MIPMAPCOUNT);
    put(12, height);
    put(16, width);
    put(28, levels.len() as u32);
    put(76, 32);
    put(80, DDPF_FOURCC);
    file[84..88].copy_from_slice(four_cc);
    for level in levels {
        file.extend_from_slice(level);
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dxt1_file_with_mipmaps_is_read_level_by_level() {
        // 8 by 8 is four blocks, 4 by 4 one, and 2 by 2 and 1 by 1 one each.
        let levels: [&[u8]; 4] = [&[1; 32], &[2; 8], &[3; 8], &[4; 8]];
        let data = CompressedImageData::from_bytes(&dds_file(b"DXT1", 8, 8, &levels)).unwrap();
        assert_eq!(data.format(), CompressedFormat::Bc1);
        assert_eq!(data.dimensions(), (8, 8));
        assert_eq!(data.mipmap_count(), 4);
        assert_eq!(data.levels(), levels);
    }

    #[test]
    fn each_four_cc_names_its_format() {
        for (four_cc, format) in [
            (b"DXT3", CompressedFormat::Bc2),
            (b"DXT5", CompressedFormat::Bc3),
            (b"ATI1", CompressedFormat::Bc4),
            (b"ATI2", CompressedFormat::Bc5),
        ] {
            let level = vec![0; format.block_size()];
            let data =
                CompressedImageData::from_bytes(&dds_file(four_cc, 4, 4, &[&level])).unwrap();
            assert_eq!(data.format(), format);
        }
    }

    #[test]
    fn a_dx10_header_names_bc7() {
        let mut file = dds_file(b"DX10", 4, 4, &[]);
        let mut header = Vec::new();
        for value in [98u32, DIMENSION_TEXTURE2D, 0, 1, 0] {
            header.extend_from_slice(&value.to_le_bytes());
        }
        file.extend_from_slice(&header);
        file.extend_from_slice(&[7; 16]);
        let data = CompressedImageData::from_bytes(&file).unwrap();
        assert_eq!(data.format(), CompressedFormat::Bc7);
        assert_eq!(data.levels(), [&[7; 16][..]]);
    }

    #[test]
    fn odd_sizes_round_up_to_whole_blocks() {
        assert_eq!(CompressedFormat::Bc1.data_size(5, 3), 2 * 8);
        assert_eq!(CompressedFormat::Bc3.data_size(1, 1), 16);
        assert_eq!(CompressedFormat::Bc7.data_size(9, 8), 3 * 2 * 16);
        assert_eq!(CompressedFormat::Astc8x5.data_size(17, 10), 3 * 2 * 16);
        assert_eq!(CompressedFormat::Astc12x12.data_size(1, 1), 16);
    }

    #[test]
    fn a_dx10_header_names_bc6h() {
        for (dxgi, format) in [
            (95u32, CompressedFormat::Bc6h),
            (96, CompressedFormat::Bc6hSigned),
        ] {
            let mut file = dds_file(b"DX10", 4, 4, &[]);
            for value in [dxgi, DIMENSION_TEXTURE2D, 0, 1, 0] {
                file.extend_from_slice(&value.to_le_bytes());
            }
            file.extend_from_slice(&[0; 16]);
            assert_eq!(
                CompressedImageData::from_bytes(&file).unwrap().format(),
                format
            );
        }
    }

    /// The bytes of a KTX 1 file holding `levels`, little-endian unless
    /// `big_endian`.
    fn ktx_file(
        internal: u32,
        width: u32,
        height: u32,
        levels: &[&[u8]],
        big_endian: bool,
    ) -> Vec<u8> {
        let word = |value: u32| {
            if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let mut file = KTX_IDENTIFIER.to_vec();
        let key_value = b"KTXorientation\0S=r,T=d\0\0";
        for value in [
            0x0403_0201,
            0,
            1,
            0,
            internal,
            0x1908,
            width,
            height,
            0,
            0,
            1,
            levels.len() as u32,
            key_value.len() as u32,
        ] {
            file.extend_from_slice(&word(value));
        }
        file.extend_from_slice(key_value);
        for level in levels {
            file.extend_from_slice(&word(level.len() as u32));
            file.extend_from_slice(level);
            file.resize(file.len().next_multiple_of(4), 0);
        }
        file
    }

    #[test]
    fn a_ktx_file_is_read_level_by_level_in_either_byte_order() {
        // ASTC 6x6: 13 by 7 is three blocks by two, then 6 by 3 is one.
        let levels: [&[u8]; 4] = [&[1; 96], &[2; 16], &[3; 16], &[4; 16]];
        for big_endian in [false, true] {
            let file = ktx_file(0x93B4, 13, 7, &levels, big_endian);
            assert!(is_compressed_file(&file));
            let data = CompressedImageData::from_bytes(&file).unwrap();
            assert_eq!(data.format(), CompressedFormat::Astc6x6);
            assert_eq!(data.dimensions(), (13, 7));
            assert_eq!(data.levels(), levels);
        }
        let bc1 = ktx_file(0x83F1, 4, 4, &[&[9; 8]], false);
        let data = CompressedImageData::from_bytes(&bc1).unwrap();
        assert_eq!(data.format(), CompressedFormat::Bc1);
        assert_eq!(data.levels(), [&[9; 8][..]]);
    }

    #[test]
    fn ktx_files_the_gpu_cannot_draw_are_refused_with_a_reason() {
        let etc = ktx_file(0x9274, 4, 4, &[&[0; 8]], false);
        let error = CompressedImageData::from_bytes(&etc)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ETC"), "{error}");
        let truncated = &ktx_file(0x93B0, 8, 8, &[&[0; 64]], false)[..80];
        assert!(CompressedImageData::from_bytes(truncated).is_err());
        let ktx2 = b"\xABKTX 20\xBB\r\n\x1A\n";
        assert!(is_compressed_file(ktx2));
        let error = CompressedImageData::from_bytes(ktx2)
            .unwrap_err()
            .to_string();
        assert!(error.contains("KTX 2"), "{error}");
    }

    #[test]
    fn missing_mipmaps_are_dropped_but_missing_pixels_are_an_error() {
        let short = dds_file(b"DXT1", 8, 8, &[&[0; 32], &[0; 8]]);
        let mut header_says_four = short.clone();
        header_says_four[28..32].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(
            CompressedImageData::from_bytes(&header_says_four)
                .unwrap()
                .mipmap_count(),
            2
        );
        let truncated = &short[..HEADER_SIZE + 10];
        assert!(CompressedImageData::from_bytes(truncated).is_err());
    }

    #[test]
    fn other_files_are_refused_with_a_reason() {
        assert!(CompressedImageData::from_bytes(b"\x89PNG").is_err());
        let error = CompressedImageData::from_bytes(&dds_file(b"ETC1", 4, 4, &[&[0; 8]]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("ETC1"), "{error}");
    }
}

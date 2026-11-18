//! Compressing, encoding, hashing and packing bytes, the equivalent of
//! `love.data`.
//!
//! Like [`math`](crate::math), everything here works when no game is
//! running.
//!
//! ```
//! use smitten::data::{self, CompressedDataFormat, EncodeFormat, HashFunction};
//!
//! let save = b"level=3;coins=120;level=3;coins=120";
//! let packed = data::compress(CompressedDataFormat::Zlib, save, None);
//! assert_eq!(data::decompress(&packed).unwrap(), save);
//!
//! let text = data::encode(EncodeFormat::Base64, b"hello");
//! assert_eq!(text, "aGVsbG8=");
//! assert_eq!(data::decode(EncodeFormat::Base64, &text).unwrap(), b"hello");
//!
//! let digest = data::hash(HashFunction::Sha256, b"abc");
//! assert!(data::encode(EncodeFormat::Hex, &digest).starts_with("ba7816bf"));
//! ```

mod pack;

use std::fmt;
use std::io::{Read, Write};

pub use pack::{Value, pack, packed_size, unpack, unpack_at};

/// Why bytes could not be decompressed, decoded or unpacked.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    /// The bytes are not valid for the format they were said to be in.
    Corrupt(String),
    /// A [`pack`] format string is wrong, or the values do not match it.
    Format(String),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::Corrupt(message) => write!(f, "could not read data: {message}"),
            DataError::Format(message) => write!(f, "bad pack format: {message}"),
        }
    }
}

impl std::error::Error for DataError {}

// ---------------------------------------------------------------------------
// Compression

/// A way of compressing bytes, LÖVE's `CompressedDataFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressedDataFormat {
    /// Very fast, but compresses less than the others. As in LÖVE, the
    /// compressed bytes start with the original size as four
    /// little-endian bytes, followed by one LZ4 block.
    Lz4,
    /// Deflate with a small header and a checksum (RFC 1950).
    Zlib,
    /// Deflate with the header and checksum `.gz` files use (RFC 1952).
    Gzip,
    /// Raw deflate with no header or checksum (RFC 1951).
    Deflate,
}

/// Compressed bytes together with the format they are in, the equivalent
/// of LÖVE's `CompressedData`. Make one with [`compress`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedData {
    format: CompressedDataFormat,
    bytes: Vec<u8>,
}

impl CompressedData {
    /// Wraps bytes that are already compressed in `format`, such as ones
    /// read back from a file, so [`decompress`] can be given them.
    pub fn new(format: CompressedDataFormat, bytes: Vec<u8>) -> CompressedData {
        CompressedData { format, bytes }
    }

    /// The format the bytes are in, like `CompressedData:getFormat`.
    #[doc(alias = "CompressedData:getFormat")]
    pub fn format(&self) -> CompressedDataFormat {
        self.format
    }

    /// The compressed bytes, for writing to a file or sending.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The compressed bytes, keeping them.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl AsRef<[u8]> for CompressedData {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Compresses `bytes`, like `love.data.compress`.
///
/// `level` trades speed for size, from 0 (fastest) to 9 (smallest), as in
/// LÖVE. `None` picks the format's usual level. LZ4 has only one level and
/// ignores it.
#[doc(alias = "love.data.compress")]
pub fn compress(format: CompressedDataFormat, bytes: &[u8], level: Option<u32>) -> CompressedData {
    let level = flate2::Compression::new(level.unwrap_or(6).min(9));
    // Writing into a Vec cannot fail, so neither can these encoders.
    let compressed = match format {
        CompressedDataFormat::Lz4 => lz4_flex::compress_prepend_size(bytes),
        CompressedDataFormat::Zlib => {
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), level);
            encoder.write_all(bytes).expect("writing to memory");
            encoder.finish().expect("writing to memory")
        }
        CompressedDataFormat::Gzip => {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), level);
            encoder.write_all(bytes).expect("writing to memory");
            encoder.finish().expect("writing to memory")
        }
        CompressedDataFormat::Deflate => {
            let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), level);
            encoder.write_all(bytes).expect("writing to memory");
            encoder.finish().expect("writing to memory")
        }
    };
    CompressedData::new(format, compressed)
}

/// Gives back the bytes that [`compress`] was given, like
/// `love.data.decompress`.
#[doc(alias = "love.data.decompress")]
pub fn decompress(data: &CompressedData) -> Result<Vec<u8>, DataError> {
    decompress_bytes(data.format, &data.bytes)
}

/// Decompresses bytes in `format` that are not wrapped in a
/// [`CompressedData`], such as ones read straight from a file.
pub fn decompress_bytes(format: CompressedDataFormat, bytes: &[u8]) -> Result<Vec<u8>, DataError> {
    let corrupt = |error: &dyn fmt::Display| DataError::Corrupt(error.to_string());
    let mut out = Vec::new();
    match format {
        CompressedDataFormat::Lz4 => {
            return lz4_flex::decompress_size_prepended(bytes).map_err(|error| corrupt(&error));
        }
        CompressedDataFormat::Zlib => flate2::read::ZlibDecoder::new(bytes)
            .read_to_end(&mut out)
            .map_err(|error| corrupt(&error))?,
        CompressedDataFormat::Gzip => flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .map_err(|error| corrupt(&error))?,
        CompressedDataFormat::Deflate => flate2::read::DeflateDecoder::new(bytes)
            .read_to_end(&mut out)
            .map_err(|error| corrupt(&error))?,
    };
    Ok(out)
}

// ---------------------------------------------------------------------------
// Encoding

/// A way of writing bytes as text, LÖVE's `EncodeFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncodeFormat {
    /// Four characters for every three bytes, padded with `=`.
    Base64,
    /// Two lowercase hexadecimal digits for every byte.
    Hex,
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Writes `bytes` as text, like `love.data.encode`.
#[doc(alias = "love.data.encode")]
pub fn encode(format: EncodeFormat, bytes: &[u8]) -> String {
    match format {
        EncodeFormat::Hex => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        EncodeFormat::Base64 => {
            let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for chunk in bytes.chunks(3) {
                let group = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |group, (index, &byte)| {
                        group | (byte as u32) << (16 - 8 * index)
                    });
                for index in 0..4 {
                    if index <= chunk.len() {
                        let digit = (group >> (18 - 6 * index)) & 63;
                        text.push(BASE64[digit as usize] as char);
                    } else {
                        text.push('=');
                    }
                }
            }
            text
        }
    }
}

/// Like [`encode`], but breaks the text into lines of at most
/// `line_length` characters, as `love.data.encode`'s `linelength` does.
/// Lines end with `\n`. A `line_length` of 0 makes one line.
pub fn encode_lines(format: EncodeFormat, bytes: &[u8], line_length: usize) -> String {
    let text = encode(format, bytes);
    if line_length == 0 {
        return text;
    }
    let mut lines = String::with_capacity(text.len() + text.len() / line_length + 1);
    for (index, chunk) in text.as_bytes().chunks(line_length).enumerate() {
        if index > 0 {
            lines.push('\n');
        }
        // Encoded text is ASCII, so any split falls between characters.
        lines.push_str(std::str::from_utf8(chunk).expect("encoded text is ASCII"));
    }
    lines
}

/// Reads bytes back from text written by [`encode`], like
/// `love.data.decode`. Whitespace, such as the line breaks
/// [`encode_lines`] adds, is skipped. Base64 padding may be left off.
#[doc(alias = "love.data.decode")]
pub fn decode(format: EncodeFormat, text: &str) -> Result<Vec<u8>, DataError> {
    let digits = text.bytes().filter(|byte| !byte.is_ascii_whitespace());
    match format {
        EncodeFormat::Hex => {
            let digits: Vec<u8> = digits.collect();
            if !digits.len().is_multiple_of(2) {
                return Err(DataError::Corrupt("hex text has an odd length".into()));
            }
            digits
                .chunks(2)
                .map(|pair| {
                    let digit = |byte: u8| {
                        (byte as char).to_digit(16).ok_or_else(|| {
                            DataError::Corrupt(format!("{:?} is not a hex digit", byte as char))
                        })
                    };
                    Ok((digit(pair[0])? << 4 | digit(pair[1])?) as u8)
                })
                .collect()
        }
        EncodeFormat::Base64 => {
            let mut bytes = Vec::new();
            let mut group = 0u32;
            let mut count = 0;
            let mut padding = false;
            for byte in digits {
                if byte == b'=' {
                    padding = true;
                    continue;
                }
                if padding {
                    return Err(DataError::Corrupt(
                        "base64 text goes on after padding".into(),
                    ));
                }
                let value = BASE64
                    .iter()
                    .position(|&digit| digit == byte)
                    .ok_or_else(|| {
                        DataError::Corrupt(format!("{:?} is not a base64 digit", byte as char))
                    })?;
                group = group << 6 | value as u32;
                count += 1;
                if count == 4 {
                    bytes.extend_from_slice(&group.to_be_bytes()[1..]);
                    group = 0;
                    count = 0;
                }
            }
            match count {
                0 => {}
                2 => bytes.push((group >> 4) as u8),
                3 => bytes.extend_from_slice(&((group >> 2) as u16).to_be_bytes()),
                _ => return Err(DataError::Corrupt("base64 text is cut short".into())),
            }
            Ok(bytes)
        }
    }
}

// ---------------------------------------------------------------------------
// Hashing

/// A hash function, LÖVE's `HashFunction`.
///
/// MD5 and SHA-1 are broken for security purposes. They are here for
/// checking files against known sums and for talking to old services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashFunction {
    /// MD5, 16 bytes. Not secure.
    Md5,
    /// SHA-1, 20 bytes. Not secure.
    Sha1,
    /// SHA-224, 28 bytes.
    Sha224,
    /// SHA-256, 32 bytes.
    Sha256,
    /// SHA-384, 48 bytes.
    Sha384,
    /// SHA-512, 64 bytes.
    Sha512,
}

/// The raw digest of `bytes`, like `love.data.hash`. Use
/// [`encode`] with [`EncodeFormat::Hex`] to get the usual text form.
#[doc(alias = "love.data.hash")]
pub fn hash(function: HashFunction, bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    match function {
        HashFunction::Md5 => md5::Md5::digest(bytes).to_vec(),
        HashFunction::Sha1 => sha1::Sha1::digest(bytes).to_vec(),
        HashFunction::Sha224 => sha2::Sha224::digest(bytes).to_vec(),
        HashFunction::Sha256 => sha2::Sha256::digest(bytes).to_vec(),
        HashFunction::Sha384 => sha2::Sha384::digest(bytes).to_vec(),
        HashFunction::Sha512 => sha2::Sha512::digest(bytes).to_vec(),
    }
}

// ---------------------------------------------------------------------------
// ByteData

/// A block of bytes that can be read and changed in place, the
/// equivalent of LÖVE's `ByteData` from `love.data.newByteData`.
///
/// It derefs to `[u8]`, so slicing it does what LÖVE's `newDataView`
/// does, and every slice method works on it.
///
/// ```
/// use smitten::data::ByteData;
///
/// let mut data = ByteData::new(4);
/// data[1] = 7;
/// assert_eq!(data.size(), 4);
/// assert_eq!(&data[..2], &[0, 7]);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ByteData {
    bytes: Vec<u8>,
}

impl ByteData {
    /// `size` bytes, all zero.
    pub fn new(size: usize) -> ByteData {
        ByteData {
            bytes: vec![0; size],
        }
    }

    /// A copy of `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> ByteData {
        ByteData {
            bytes: bytes.to_vec(),
        }
    }

    /// How many bytes there are, like `Data:getSize`.
    #[doc(alias = "Data:getSize")]
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    /// The bytes, keeping them.
    pub fn into_vec(self) -> Vec<u8> {
        self.bytes
    }
}

impl From<Vec<u8>> for ByteData {
    fn from(bytes: Vec<u8>) -> ByteData {
        ByteData { bytes }
    }
}

impl From<&[u8]> for ByteData {
    fn from(bytes: &[u8]) -> ByteData {
        ByteData::from_bytes(bytes)
    }
}

impl From<&str> for ByteData {
    fn from(text: &str) -> ByteData {
        ByteData::from_bytes(text.as_bytes())
    }
}

impl std::ops::Deref for ByteData {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl std::ops::DerefMut for ByteData {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl AsRef<[u8]> for ByteData {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsMut<[u8]> for ByteData {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMATS: [CompressedDataFormat; 4] = [
        CompressedDataFormat::Lz4,
        CompressedDataFormat::Zlib,
        CompressedDataFormat::Gzip,
        CompressedDataFormat::Deflate,
    ];

    #[test]
    fn every_format_round_trips_and_shrinks_repetitive_bytes() {
        let original: Vec<u8> = b"smitten ".iter().copied().cycle().take(4000).collect();
        for format in FORMATS {
            for level in [None, Some(0), Some(9), Some(50)] {
                let compressed = compress(format, &original, level);
                assert_eq!(compressed.format(), format);
                if level != Some(0) {
                    assert!(compressed.bytes().len() < 200, "{format:?} {level:?}");
                }
                assert_eq!(decompress(&compressed).unwrap(), original, "{format:?}");
            }
            let empty = compress(format, b"", None);
            assert_eq!(decompress(&empty).unwrap(), b"");
        }
    }

    #[test]
    fn formats_have_their_usual_headers() {
        let bytes = b"hello hello hello";
        let lz4 = compress(CompressedDataFormat::Lz4, bytes, None);
        assert_eq!(&lz4.bytes()[..4], &(bytes.len() as u32).to_le_bytes());
        let zlib = compress(CompressedDataFormat::Zlib, bytes, None);
        assert_eq!(zlib.bytes()[0], 0x78);
        let gzip = compress(CompressedDataFormat::Gzip, bytes, None);
        assert_eq!(&gzip.bytes()[..2], &[0x1f, 0x8b]);
    }

    #[test]
    fn corrupt_bytes_are_an_error() {
        for format in FORMATS {
            let result = decompress_bytes(format, &[0xff, 0xff, 0xff, 0xff, 0x01, 0x02]);
            assert!(matches!(result, Err(DataError::Corrupt(_))), "{format:?}");
        }
    }

    #[test]
    fn base64_matches_the_standard_examples() {
        let cases: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (bytes, text) in cases {
            assert_eq!(encode(EncodeFormat::Base64, bytes), text);
            assert_eq!(decode(EncodeFormat::Base64, text).unwrap(), bytes);
        }
        assert_eq!(decode(EncodeFormat::Base64, "Zm9vYg").unwrap(), b"foob");
        assert_eq!(
            decode(EncodeFormat::Base64, "Zm9v\nYmFy\n").unwrap(),
            b"foobar"
        );
        assert!(decode(EncodeFormat::Base64, "Zm9v!").is_err());
        assert!(decode(EncodeFormat::Base64, "Z").is_err());
        assert!(decode(EncodeFormat::Base64, "Zg==Zg").is_err());
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(encode(EncodeFormat::Hex, &bytes), "0001abff");
        assert_eq!(decode(EncodeFormat::Hex, "0001ABff").unwrap(), bytes);
        assert!(decode(EncodeFormat::Hex, "abc").is_err());
        assert!(decode(EncodeFormat::Hex, "zz").is_err());
    }

    #[test]
    fn lines_are_broken_at_the_given_length() {
        let text = encode_lines(EncodeFormat::Hex, &[1, 2, 3, 4, 5], 4);
        assert_eq!(text, "0102\n0304\n05");
        assert_eq!(decode(EncodeFormat::Hex, &text).unwrap(), [1, 2, 3, 4, 5]);
        assert_eq!(encode_lines(EncodeFormat::Hex, &[1, 2], 0), "0102");
    }

    #[test]
    fn hashes_match_known_digests() {
        let hex = |function| encode(EncodeFormat::Hex, &hash(function, b"abc"));
        assert_eq!(hex(HashFunction::Md5), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(HashFunction::Sha1),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(HashFunction::Sha224),
            "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7"
        );
        assert_eq!(
            hex(HashFunction::Sha256),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash(HashFunction::Sha384, b"abc").len(), 48);
        assert!(hex(HashFunction::Sha512).starts_with("ddaf35a193617aba"));
    }

    #[test]
    fn byte_data_reads_and_writes_in_place() {
        let mut data = ByteData::from("abcd");
        data[0] = b'A';
        assert_eq!(&data[..], b"Abcd");
        assert_eq!(data.size(), 4);
        assert_eq!(ByteData::new(3).into_vec(), vec![0, 0, 0]);
    }
}

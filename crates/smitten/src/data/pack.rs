//! Packing values into bytes with Lua's `string.pack` formats.

use super::DataError;

/// One value to [`pack`] or one [`unpack`] gave back.
///
/// Lua has one number type that is either an integer or a float, and one
/// string type that holds any bytes. These are the same three.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// For the integer options, such as `i4` or `B`.
    Int(i64),
    /// For `f`, `d` and `n`.
    Float(f64),
    /// For `s`, `z` and `c`.
    Bytes(Vec<u8>),
}

impl Value {
    /// The integer, if this is one, or a float with no fractional part.
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Value::Int(value) => Some(value),
            Value::Float(value) => float_to_int(value),
            Value::Bytes(_) => None,
        }
    }

    /// The number, as a float.
    pub fn as_float(&self) -> Option<f64> {
        match *self {
            Value::Int(value) => Some(value as f64),
            Value::Float(value) => Some(value),
            Value::Bytes(_) => None,
        }
    }

    /// The bytes, if this is a string.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The string, if this is one and holds valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        self.as_bytes()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
    }
}

fn float_to_int(value: f64) -> Option<i64> {
    // -2^63 is exact as a float; 2^63 is the first value out of range.
    let in_range = (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value);
    (in_range && value.fract() == 0.0).then_some(value as i64)
}

macro_rules! from_int {
    ($($type:ty),*) => {$(
        impl From<$type> for Value {
            fn from(value: $type) -> Value {
                Value::Int(value as i64)
            }
        }
    )*};
}
from_int!(i8, u8, i16, u16, i32, u32, i64);

impl From<f32> for Value {
    fn from(value: f32) -> Value {
        Value::Float(value as f64)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Value {
        Value::Float(value)
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Value {
        Value::Bytes(text.as_bytes().to_vec())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Value {
        Value::Bytes(text.into_bytes())
    }
}

impl From<&[u8]> for Value {
    fn from(bytes: &[u8]) -> Value {
        Value::Bytes(bytes.to_vec())
    }
}

impl From<Vec<u8>> for Value {
    fn from(bytes: Vec<u8>) -> Value {
        Value::Bytes(bytes)
    }
}

// ---------------------------------------------------------------------------
// Reading formats

/// The largest integer, in bytes, that an option may ask for.
const MAX_INT_SIZE: usize = 16;
/// The alignment `!` with no number asks for, as on 64-bit platforms.
const NATIVE_ALIGN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Int {
        signed: bool,
    },
    Float,
    /// A string of exactly `size` bytes (`c`).
    Char,
    /// A string after its length, which takes `size` bytes (`s`).
    Str,
    /// A string ended by a zero byte (`z`).
    Zstr,
    /// One zero byte (`x`).
    Padding,
    /// Only alignment (`X`).
    PadAlign,
    /// Changes endianness or alignment, and takes no space.
    Nop,
}

struct Option_ {
    kind: Kind,
    size: usize,
    /// Zero bytes to add before it, to align it.
    padding: usize,
}

struct Format<'a> {
    rest: &'a [u8],
    little: bool,
    max_align: usize,
}

fn format_error(message: impl Into<String>) -> DataError {
    DataError::Format(message.into())
}

impl<'a> Format<'a> {
    fn new(format: &'a str) -> Format<'a> {
        Format {
            rest: format.as_bytes(),
            little: cfg!(target_endian = "little"),
            max_align: 1,
        }
    }

    fn number(&mut self) -> Option<usize> {
        let digits = self
            .rest
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        let (number, rest) = self.rest.split_at(digits);
        self.rest = rest;
        // Lua stops reading digits once the number grows too big; any such
        // size is refused below anyway.
        Some(
            std::str::from_utf8(number)
                .ok()
                .and_then(|text| text.parse().ok())
                .unwrap_or(usize::MAX),
        )
    }

    fn int_size(&mut self, default: usize) -> Result<usize, DataError> {
        let size = self.number().unwrap_or(default);
        if !(1..=MAX_INT_SIZE).contains(&size) {
            return Err(format_error(format!(
                "integral size ({size}) out of limits [1,{MAX_INT_SIZE}]"
            )));
        }
        Ok(size)
    }

    /// Reads one option's letter and size, without alignment.
    fn kind(&mut self) -> Result<Option<(Kind, usize)>, DataError> {
        let Some((&letter, rest)) = self.rest.split_first() else {
            return Ok(None);
        };
        self.rest = rest;
        let int = |signed| Kind::Int { signed };
        let found = match letter {
            b'b' => (int(true), 1),
            b'B' => (int(false), 1),
            b'h' => (int(true), 2),
            b'H' => (int(false), 2),
            b'i' => (int(true), self.int_size(4)?),
            b'I' => (int(false), self.int_size(4)?),
            b'l' | b'j' => (int(true), 8),
            b'L' | b'J' | b'T' => (int(false), 8),
            b'f' => (Kind::Float, 4),
            b'd' | b'n' => (Kind::Float, 8),
            b's' => (Kind::Str, self.int_size(8)?),
            b'c' => {
                let size = self
                    .number()
                    .ok_or_else(|| format_error("missing size for format option 'c'"))?;
                (Kind::Char, size)
            }
            b'z' => (Kind::Zstr, 0),
            b'x' => (Kind::Padding, 1),
            b'X' => (Kind::PadAlign, 0),
            b' ' => (Kind::Nop, 0),
            b'<' => {
                self.little = true;
                (Kind::Nop, 0)
            }
            b'>' => {
                self.little = false;
                (Kind::Nop, 0)
            }
            b'=' => {
                self.little = cfg!(target_endian = "little");
                (Kind::Nop, 0)
            }
            b'!' => {
                self.max_align = self.int_size(NATIVE_ALIGN)?;
                (Kind::Nop, 0)
            }
            other => {
                return Err(format_error(format!(
                    "invalid format option '{}'",
                    other as char
                )));
            }
        };
        Ok(Some(found))
    }

    /// Reads the next option and works out its alignment, given how many
    /// bytes come before it.
    fn next(&mut self, offset: usize) -> Result<Option<Option_>, DataError> {
        let Some((kind, size)) = self.kind()? else {
            return Ok(None);
        };
        let mut align = size;
        if kind == Kind::PadAlign {
            // `X` takes its alignment from the option after it, which is
            // not packed itself.
            match self.kind()? {
                Some((next, next_size)) if next != Kind::Char && next_size != 0 => {
                    align = next_size;
                }
                _ => return Err(format_error("invalid next option for option 'X'")),
            }
        }
        let padding = if align <= 1 || kind == Kind::Char {
            0
        } else {
            let align = align.min(self.max_align);
            if !align.is_power_of_two() {
                return Err(format_error("format asks for alignment not power of 2"));
            }
            (align - (offset & (align - 1))) & (align - 1)
        };
        Ok(Some(Option_ {
            kind,
            size,
            padding,
        }))
    }
}

// ---------------------------------------------------------------------------
// Packing

fn put_int(out: &mut Vec<u8>, value: u64, little: bool, size: usize, negative: bool) {
    let fill = if negative { 0xff } else { 0 };
    let bytes: Vec<u8> = (0..size)
        .map(|index| {
            if index < 8 {
                (value >> (8 * index)) as u8
            } else {
                fill
            }
        })
        .collect();
    if little {
        out.extend(bytes);
    } else {
        out.extend(bytes.into_iter().rev());
    }
}

fn get_int(bytes: &[u8], little: bool, signed: bool) -> Result<i64, DataError> {
    let size = bytes.len();
    let byte = |index: usize| {
        if little {
            bytes[index]
        } else {
            bytes[size - 1 - index]
        }
    };
    let mut value: u64 = 0;
    for index in 0..size.min(8) {
        value |= (byte(index) as u64) << (8 * index);
    }
    if size < 8 {
        if signed {
            let shift = 64 - 8 * size;
            value = (((value << shift) as i64) >> shift) as u64;
        }
    } else if size > 8 {
        // The extra bytes must only repeat the sign.
        let fill = if signed && (value as i64) < 0 {
            0xff
        } else {
            0
        };
        if (8..size).any(|index| byte(index) != fill) {
            return Err(DataError::Corrupt(format!(
                "{size}-byte integer does not fit into a 64-bit integer"
            )));
        }
    }
    Ok(value as i64)
}

/// Packs `values` into bytes as `format` says, in Lua's `string.pack`
/// formats.
///
/// | Option | Packs |
/// |---|---|
/// | `<`, `>`, `=` | Little-endian, big-endian, or this machine's order from here on |
/// | `![n]` | Aligns to at most `n` bytes from here on (8 without `n`) |
/// | `b`, `B` | A signed or unsigned byte |
/// | `h`, `H` | A signed or unsigned 2-byte integer |
/// | `i[n]`, `I[n]` | A signed or unsigned `n`-byte integer, from 1 to 16 (4 without `n`) |
/// | `l`, `L`, `j`, `J`, `T` | An 8-byte integer, signed for `l` and `j` |
/// | `f`, `d`, `n` | A 4-byte float, or an 8-byte float for `d` and `n` |
/// | `s[n]` | A string after its length as an `n`-byte unsigned integer (8 without `n`) |
/// | `z` | A string followed by a zero byte |
/// | `cn` | A string of exactly `n` bytes, padded with zeros |
/// | `x` | One zero byte |
/// | `Xop` | Zero bytes up to the alignment of option `op`, which is not packed |
///
/// Spaces are skipped. Sizes are those of 64-bit Linux and macOS, so `l` is
/// 8 bytes on every system.
///
/// ```
/// use smitten::data::{self, Value};
///
/// let bytes = data::pack("<i2 s1 d", &[Value::from(-2), "hi".into(), 0.5.into()]).unwrap();
/// assert_eq!(&bytes[..5], &[0xfe, 0xff, 2, b'h', b'i']);
/// let (values, next) = data::unpack_at("<i2 s1 d", &bytes, 0).unwrap();
/// assert_eq!(values, [Value::Int(-2), Value::from("hi"), Value::Float(0.5)]);
/// assert_eq!(next, bytes.len());
/// ```
pub fn pack(format: &str, values: &[Value]) -> Result<Vec<u8>, DataError> {
    let mut parsed = Format::new(format);
    let mut out = Vec::new();
    let mut values = values.iter().enumerate();
    let mut next_value = |what: &str| {
        values
            .next()
            .ok_or_else(|| format_error(format!("missing a value for {what}")))
    };
    while let Some(option) = parsed.next(out.len())? {
        out.resize(out.len() + option.padding, 0);
        let little = parsed.little;
        let size = option.size;
        match option.kind {
            Kind::Int { signed } => {
                let (index, value) = next_value("an integer")?;
                let number = value.as_int().ok_or_else(|| {
                    format_error(format!("value {} is not an integer", index + 1))
                })?;
                if size < 8 {
                    let fits = if signed {
                        let limit = 1i64 << (size * 8 - 1);
                        (-limit..limit).contains(&number)
                    } else {
                        (number as u64) < 1u64 << (size * 8)
                    };
                    if !fits {
                        return Err(format_error(format!(
                            "value {} overflows {size} bytes",
                            index + 1
                        )));
                    }
                }
                put_int(&mut out, number as u64, little, size, signed && number < 0);
            }
            Kind::Float => {
                let (index, value) = next_value("a float")?;
                let number = value
                    .as_float()
                    .ok_or_else(|| format_error(format!("value {} is not a number", index + 1)))?;
                let bytes = if size == 4 {
                    let bits = (number as f32).to_bits();
                    if little {
                        bits.to_le_bytes().to_vec()
                    } else {
                        bits.to_be_bytes().to_vec()
                    }
                } else if little {
                    number.to_le_bytes().to_vec()
                } else {
                    number.to_be_bytes().to_vec()
                };
                out.extend(bytes);
            }
            Kind::Char | Kind::Str | Kind::Zstr => {
                let (index, value) = next_value("a string")?;
                let bytes = value
                    .as_bytes()
                    .ok_or_else(|| format_error(format!("value {} is not a string", index + 1)))?;
                match option.kind {
                    Kind::Char => {
                        if bytes.len() > size {
                            return Err(format_error(format!(
                                "value {} is longer than {size} bytes",
                                index + 1
                            )));
                        }
                        out.extend_from_slice(bytes);
                        out.resize(out.len() + size - bytes.len(), 0);
                    }
                    Kind::Str => {
                        let length = bytes.len() as u64;
                        if size < 8 && length >= 1u64 << (size * 8) {
                            return Err(format_error(format!(
                                "the length of value {} does not fit in {size} bytes",
                                index + 1
                            )));
                        }
                        put_int(&mut out, length, little, size, false);
                        out.extend_from_slice(bytes);
                    }
                    _ => {
                        if bytes.contains(&0) {
                            return Err(format_error(format!(
                                "value {} contains a zero byte",
                                index + 1
                            )));
                        }
                        out.extend_from_slice(bytes);
                        out.push(0);
                    }
                }
            }
            Kind::Padding => out.push(0),
            Kind::PadAlign | Kind::Nop => {}
        }
    }
    Ok(out)
}

/// How many bytes [`pack`] makes with `format`. Formats with `s` or `z`
/// have no fixed size and are an error.
pub fn packed_size(format: &str) -> Result<usize, DataError> {
    let mut parsed = Format::new(format);
    let mut total: usize = 0;
    while let Some(option) = parsed.next(total)? {
        if matches!(option.kind, Kind::Str | Kind::Zstr) {
            return Err(format_error("variable-length format"));
        }
        total = total
            .checked_add(option.padding + option.size)
            .ok_or_else(|| format_error("format result too large"))?;
    }
    Ok(total)
}

/// Reads values that [`pack`] packed with the same `format`.
pub fn unpack(format: &str, bytes: &[u8]) -> Result<Vec<Value>, DataError> {
    unpack_at(format, bytes, 0).map(|(values, _)| values)
}

/// Like [`unpack`], but starts `offset` bytes in and also gives back the
/// offset just past what was read, for reading what follows.
///
/// Offsets count from 0, where Lua counts positions from 1, so Lua's
/// position `p` is offset `p - 1`.
pub fn unpack_at(
    format: &str,
    bytes: &[u8],
    offset: usize,
) -> Result<(Vec<Value>, usize), DataError> {
    if offset > bytes.len() {
        return Err(DataError::Corrupt("initial position out of data".into()));
    }
    let short = || DataError::Corrupt("data too short".into());
    let mut parsed = Format::new(format);
    let mut position = offset;
    let mut values = Vec::new();
    // Alignment counts from the start of the data, as Lua's does.
    while let Some(option) = parsed.next(position)? {
        let size = option.size;
        let start = position
            .checked_add(option.padding)
            .filter(|start| start.saturating_add(size) <= bytes.len())
            .ok_or_else(short)?;
        let field = &bytes[start..start + size];
        position = start + size;
        let little = parsed.little;
        match option.kind {
            Kind::Int { signed } => values.push(Value::Int(get_int(field, little, signed)?)),
            Kind::Float => {
                let number = if size == 4 {
                    let raw: [u8; 4] = field.try_into().expect("four bytes");
                    let bits = if little {
                        u32::from_le_bytes(raw)
                    } else {
                        u32::from_be_bytes(raw)
                    };
                    f32::from_bits(bits) as f64
                } else {
                    let raw: [u8; 8] = field.try_into().expect("eight bytes");
                    if little {
                        f64::from_le_bytes(raw)
                    } else {
                        f64::from_be_bytes(raw)
                    }
                };
                values.push(Value::Float(number));
            }
            Kind::Char => values.push(Value::Bytes(field.to_vec())),
            Kind::Str => {
                let length = get_int(field, little, false)? as u64;
                let end = usize::try_from(length)
                    .ok()
                    .and_then(|length| position.checked_add(length))
                    .filter(|&end| end <= bytes.len())
                    .ok_or_else(short)?;
                values.push(Value::Bytes(bytes[position..end].to_vec()));
                position = end;
            }
            Kind::Zstr => {
                let length = bytes[position..]
                    .iter()
                    .position(|&byte| byte == 0)
                    .ok_or_else(|| DataError::Corrupt("unfinished string for format 'z'".into()))?;
                values.push(Value::Bytes(bytes[position..position + length].to_vec()));
                position += length + 1;
            }
            Kind::Padding | Kind::PadAlign | Kind::Nop => {}
        }
    }
    Ok((values, position))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(format: &str, values: &[Value]) -> Vec<u8> {
        let bytes = pack(format, values).unwrap();
        assert_eq!(unpack(format, &bytes).unwrap(), values, "{format}");
        bytes
    }

    #[test]
    fn integers_pack_in_either_byte_order() {
        assert_eq!(round_trip("<i4", &[0x0102_0304.into()]), [4, 3, 2, 1]);
        assert_eq!(round_trip(">i4", &[0x0102_0304.into()]), [1, 2, 3, 4]);
        assert_eq!(round_trip("<h", &[(-2).into()]), [0xfe, 0xff]);
        assert_eq!(round_trip("<B b", &[255.into(), (-1).into()]), [255, 255]);
        assert_eq!(round_trip("<i3", &[(-1).into()]), [0xff; 3]);
        assert_eq!(round_trip("<j", &[Value::Int(i64::MIN)]).len(), 8);
        assert_eq!(round_trip("<J", &[Value::Int(-1)]), [0xff; 8]);
    }

    #[test]
    fn wide_integers_extend_the_sign() {
        let bytes = round_trip("<i12", &[(-5).into()]);
        assert_eq!(&bytes[8..], &[0xff; 4]);
        let bytes = round_trip(">I16", &[7.into()]);
        assert_eq!(bytes[15], 7);
        let mut too_big = vec![0u8; 12];
        too_big[9] = 1;
        assert!(unpack("<i12", &too_big).is_err());
    }

    #[test]
    fn integers_that_do_not_fit_are_refused() {
        assert!(pack("b", &[128.into()]).is_err());
        assert!(pack("b", &[(-129).into()]).is_err());
        assert!(pack("B", &[256.into()]).is_err());
        assert!(pack("B", &[(-1).into()]).is_err());
        assert!(pack("i17", &[1.into()]).is_err());
        assert!(pack("i0", &[1.into()]).is_err());
        assert!(pack("i", &[1.5.into()]).is_err());
        assert_eq!(pack("<i", &[2.0.into()]).unwrap(), [2, 0, 0, 0]);
    }

    #[test]
    fn floats_and_strings_round_trip() {
        round_trip("<f d n", &[0.5.into(), (-1.25).into(), 1e100.into()]);
        let bytes = round_trip("<s2 z c5", &["hey".into(), "zero".into(), "abc\0\0".into()]);
        assert_eq!(&bytes[..5], &[3, 0, b'h', b'e', b'y']);
        assert_eq!(pack("c3", &["a".into()]).unwrap(), [b'a', 0, 0]);
        assert!(pack("c2", &["abc".into()]).is_err());
        assert!(pack("z", &["a\0b".into()]).is_err());
        assert!(pack("s1", &[vec![0u8; 256].into()]).is_err());
        assert!(pack("i", &["no".into()]).is_err());
        assert!(pack("i i", &[1.into()]).is_err());
    }

    #[test]
    fn alignment_pads_with_zeros() {
        assert_eq!(
            pack("<!4 b i4", &[1.into(), 2.into()]).unwrap(),
            [1, 0, 0, 0, 2, 0, 0, 0]
        );
        // Without `!`, nothing is aligned.
        assert_eq!(pack("<b i4", &[1.into(), 2.into()]).unwrap().len(), 5);
        assert_eq!(
            pack("<! b Xi8", &[1.into()]).unwrap(),
            [1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(pack("<!2 b d", &[1.into(), 0.0.into()]).unwrap().len(), 10);
        assert!(pack("!4 b i3", &[1.into(), 1.into()]).is_err());
        assert!(pack("X", &[]).is_err());
        assert!(pack("Xc2", &[]).is_err());
        round_trip("<!8 b d b j", &[1.into(), 2.5.into(), 3.into(), 4.into()]);
    }

    #[test]
    fn sizes_are_known_without_packing() {
        assert_eq!(packed_size("i4 i8 b").unwrap(), 13);
        assert_eq!(packed_size("!8 b d").unwrap(), 16);
        assert_eq!(packed_size("c10 x h").unwrap(), 13);
        assert!(packed_size("s").is_err());
        assert!(packed_size("z").is_err());
        assert!(packed_size("q").is_err());
    }

    #[test]
    fn unpacking_can_start_partway_and_report_where_it_stopped() {
        let bytes = pack("<B i2 i2", &[9.into(), 1.into(), 2.into()]).unwrap();
        let (values, next) = unpack_at("<i2", &bytes, 1).unwrap();
        assert_eq!(values, [Value::Int(1)]);
        assert_eq!(next, 3);
        let (values, next) = unpack_at("<i2", &bytes, next).unwrap();
        assert_eq!(values, [Value::Int(2)]);
        assert_eq!(next, bytes.len());
        assert!(unpack_at("<i2", &bytes, next).is_err());
        assert!(unpack_at("<i2", &bytes, 99).is_err());
        assert!(unpack("<s1", &[5, b'a']).is_err());
        assert!(unpack("z", b"abc").is_err());
    }

    #[test]
    fn values_convert() {
        assert_eq!(Value::from(3u8).as_int(), Some(3));
        assert_eq!(Value::from(3.0).as_int(), Some(3));
        assert_eq!(Value::from(3.5).as_int(), None);
        assert_eq!(Value::from(2).as_float(), Some(2.0));
        assert_eq!(Value::from("hi").as_str(), Some("hi"));
        assert_eq!(Value::from(1).as_bytes(), None);
    }
}

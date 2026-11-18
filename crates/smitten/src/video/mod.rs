//! Decoding video, the equivalent of `love.video`.
//!
//! Most games want [`Video::load`](crate::graphics::Video::load),
//! which plays a video and draws it. A [`VideoStream`] is the decoder
//! underneath, for reading a video's frames as pixels without drawing
//! them.
//!
//! As in LÖVE, videos are Ogg Theora files, usually named `.ogv`, and
//! their sound, if they have any, is an Ogg Vorbis track in the same file.
//! FFmpeg makes one from almost any video:
//!
//! ```text
//! ffmpeg -i input.mp4 -c:v libtheora -q:v 7 -c:a libvorbis -q:a 4 output.ogv
//! ```
//!
//! Decoding is done in Rust by the `theora-rs` crate, which gives the same
//! pixels as the reference decoder.

mod convert;

use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use theora_rs::{DecoderContext, HeaderParser, Info, OggPacket, SetupInfo};

/// Why a video could not be loaded.
#[non_exhaustive]
#[derive(Debug)]
pub enum VideoError {
    /// The file could not be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The data is not a video Smitten can decode.
    Decode(String),
    /// The frames could not be put on the GPU.
    Upload(String),
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VideoError::Read { path, source } => {
                write!(f, "could not read video {}: {source}", path.display())
            }
            VideoError::Decode(message) => write!(f, "could not load video: {message}"),
            VideoError::Upload(message) => write!(f, "could not show video: {message}"),
        }
    }
}

impl std::error::Error for VideoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VideoError::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// One compressed frame.
struct Packet {
    packet: OggPacket,
    keyframe: bool,
    /// When the frame starts showing, in frames from the start. Encoders
    /// leave out frames that repeat the one before, so these can skip.
    frame: u64,
}

/// A frame as it is read, before its number is worked out.
struct Unnumbered {
    packet: OggPacket,
    keyframe: bool,
    /// The page's granule position, given by the last packet ending on
    /// each page.
    granule: Option<u64>,
}

/// Decodes the frames of a video, the equivalent of LÖVE's `VideoStream`.
///
/// Unlike LÖVE's, a stream does not keep time; ask it for the frame at a
/// moment with [`VideoStream::seek`] and read the picture with
/// [`VideoStream::pixels`]. A [`Video`](crate::graphics::Video) does the
/// timekeeping.
pub struct VideoStream {
    info: Info,
    setup: SetupInfo,
    /// Every frame after the headers, in order. A video is held
    /// compressed in memory, much as a streamed sound is.
    packets: Vec<Packet>,
    decoder: DecoderContext,
    /// The packet the decoder takes next.
    next: usize,
    /// The frame `pixels` shows, once one has been decoded.
    shown: Option<usize>,
    /// The picture, four bytes a pixel in rows from the top.
    rgba: Vec<u8>,
    /// The file it was loaded from, if it came from one.
    path: Option<PathBuf>,
}

impl VideoStream {
    /// Opens a video held in memory.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<VideoStream, VideoError> {
        let bytes = bytes.into();
        let decode_error = |message: &str| VideoError::Decode(message.to_owned());
        let mut reader = ogg::reading::PacketReader::new(Cursor::new(&bytes[..]));
        let mut parser = HeaderParser::new();
        let mut serial = None;
        let mut packets = Vec::new();
        let mut number = 0;
        loop {
            let packet = match reader.read_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                // A damaged end still leaves the frames before it.
                Err(_) if serial.is_some() && parser.is_ready() => break,
                Err(error) => return Err(VideoError::Decode(format!("not an Ogg file ({error})"))),
            };
            if serial.is_none() {
                let data = &packet.data;
                let theora = data.len() >= 7 && data[0] == 0x80 && &data[1..7] == b"theora";
                if packet.first_in_stream() && theora {
                    serial = Some(packet.stream_serial());
                } else {
                    continue;
                }
            }
            if serial != Some(packet.stream_serial()) {
                continue;
            }
            let keyframe = packet.data.first().is_some_and(|&first| first & 0x40 == 0);
            let (last_in_page, granule) = (packet.last_in_page(), packet.absgp_page());
            let packet = OggPacket {
                b_o_s: packet.first_in_stream(),
                e_o_s: packet.last_in_stream(),
                granulepos: -1,
                packetno: number,
                packet: packet.data,
            };
            number += 1;
            if !parser.is_ready() {
                parser.push(&packet).map_err(|error| {
                    VideoError::Decode(format!("bad Theora header ({error:?})"))
                })?;
                continue;
            }
            // Empty packets repeat the frame before; they are not keyframes.
            let keyframe = keyframe && !packet.packet.is_empty();
            packets.push(Unnumbered {
                packet,
                keyframe,
                granule: last_in_page
                    .then_some(granule)
                    .filter(|&granule| granule != u64::MAX),
            });
        }
        if serial.is_none() {
            return Err(decode_error("the file has no Theora video in it"));
        }
        let (info, _, setup) = parser.into_parts();
        let setup = setup.ok_or_else(|| decode_error("the video's headers are incomplete"))?;
        if info.pic_width == 0
            || info.pic_height == 0
            || info.fps_numerator == 0
            || info.fps_denominator == 0
        {
            return Err(decode_error(
                "the video's headers give no size or frame rate",
            ));
        }
        let packets = number_frames(packets, &info);
        let decoder = new_decoder(&info, &setup)?;
        let rgba = vec![0; info.pic_width as usize * info.pic_height as usize * 4];
        let mut stream = VideoStream {
            info,
            setup,
            packets,
            decoder,
            next: 0,
            shown: None,
            rgba,
            path: None,
        };
        // Show the first frame straight away.
        stream.show_frame(0);
        Ok(stream)
    }

    /// The path the video was loaded from, as given to
    /// [`VideoStream::load`], the equivalent of `VideoStream:getFilename`.
    /// `None` for one made with [`VideoStream::from_bytes`].
    pub fn filename(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The size of the picture in pixels, as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.info.pic_width, self.info.pic_height)
    }

    /// Frames per second.
    pub fn frame_rate(&self) -> f64 {
        f64::from(self.info.fps_numerator) / f64::from(self.info.fps_denominator)
    }

    /// How long the video is in frames at [`VideoStream::frame_rate`].
    pub fn frame_count(&self) -> u64 {
        self.packets.last().map_or(0, |packet| packet.frame + 1)
    }

    /// The length in seconds.
    pub fn duration(&self) -> f64 {
        self.frame_count() as f64 / self.frame_rate()
    }

    /// The number of the frame [`VideoStream::pixels`] holds, counting
    /// from 0 at the start.
    pub fn frame(&self) -> u64 {
        self.shown.map_or(0, |index| self.packets[index].frame)
    }

    /// Decodes the frame showing at `seconds` from the start. Returns
    /// whether the picture changed.
    pub fn seek(&mut self, seconds: f64) -> bool {
        let frame = (seconds.max(0.0) * self.frame_rate()).floor() as u64;
        // The last frame that starts by then; a repeated frame is not
        // stored, so the one before it carries on.
        let index = self
            .packets
            .partition_point(|packet| packet.frame <= frame)
            .saturating_sub(1);
        self.show_frame(index)
    }

    /// The current picture: four bytes a pixel (red, green, blue, alpha)
    /// in rows from the top, as [`Image::from_rgba`] takes.
    ///
    /// [`Image::from_rgba`]: crate::graphics::Image::from_rgba
    pub fn pixels(&self) -> &[u8] {
        &self.rgba
    }

    /// Decodes packet `target`, starting again from a keyframe if it is
    /// behind or far ahead. Returns whether the picture changed.
    fn show_frame(&mut self, target: usize) -> bool {
        if self.packets.is_empty() || self.shown == Some(target) {
            return false;
        }
        let target = target.min(self.packets.len() - 1);
        let keyframe = (0..=target)
            .rev()
            .find(|&index| self.packets[index].keyframe)
            .unwrap_or(0);
        if target < self.next || keyframe > self.next {
            // Going back, or jumping past a keyframe: start from it with a
            // fresh decoder, which needs no earlier frames.
            match new_decoder(&self.info, &self.setup) {
                Ok(decoder) => self.decoder = decoder,
                Err(_) => return false,
            }
            self.next = keyframe;
        }
        let mut decoded = false;
        while self.next <= target {
            let packet = &self.packets[self.next].packet;
            // A damaged frame is skipped; the next keyframe repairs it.
            if theora_rs::th_decode_packetin(&mut self.decoder, packet).is_ok() {
                decoded = true;
            }
            self.next += 1;
        }
        self.shown = Some(target);
        if !decoded {
            return false;
        }
        match theora_rs::th_decode_ycbcr_ref(&self.decoder) {
            Ok(planes) => {
                convert::to_rgba(&planes, &self.info, &mut self.rgba);
                true
            }
            Err(_) => false,
        }
    }
}

/// Works out when each frame shows from the granule positions of the
/// pages they end on. A granule position counts frames as the last
/// keyframe's number shifted up, plus the frames since it.
fn number_frames(packets: Vec<Unnumbered>, info: &Info) -> Vec<Packet> {
    let shift = info.keyframe_granule_shift.clamp(0, 63) as u32;
    // From Theora 3.2.1 on, a granule position counts the frame itself,
    // so the first frame is 1.
    let version = (
        info.version_major,
        info.version_minor,
        info.version_subminor,
    );
    let counts_from_one = u64::from(version >= (3, 2, 1));
    let to_frame = |granule: u64| {
        let keyframe = granule >> shift;
        let since = granule - (keyframe << shift);
        (keyframe + since).saturating_sub(counts_from_one)
    };

    let mut frames: Vec<Option<u64>> = vec![None; packets.len()];
    let mut page_start = 0;
    for (index, packet) in packets.iter().enumerate() {
        if let Some(granule) = packet.granule {
            // The packets before it on its page are the frames before it.
            let last = to_frame(granule);
            for (back, slot) in frames[page_start..=index].iter_mut().rev().enumerate() {
                *slot = Some(last.saturating_sub(back as u64));
            }
            page_start = index + 1;
        }
    }

    // Keep the numbers rising, fill any the pages did not give, and start
    // the video at 0 even if it was cut from a longer one.
    let start = frames.iter().flatten().next().copied().unwrap_or(0);
    let mut previous: Option<u64> = None;
    packets
        .into_iter()
        .zip(frames)
        .map(|(packet, frame)| {
            let earliest = previous.map_or(0, |previous| previous + 1);
            let frame = frame.map_or(earliest, |frame| frame.saturating_sub(start).max(earliest));
            previous = Some(frame);
            Packet {
                packet: packet.packet,
                keyframe: packet.keyframe,
                frame,
            }
        })
        .collect()
}

fn new_decoder(info: &Info, setup: &SetupInfo) -> Result<DecoderContext, VideoError> {
    theora_rs::th_decode_alloc(info, setup)
        .map_err(|error| VideoError::Decode(format!("could not start decoding ({error:?})")))
}

impl fmt::Debug for VideoStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoStream")
            .field("dimensions", &self.dimensions())
            .field("frame_rate", &self.frame_rate())
            .field("frame_count", &self.frame_count())
            .field("frame", &self.frame())
            .finish()
    }
}

impl VideoStream {
    /// Opens a video file for decoding, the equivalent of
    /// `love.video.newVideoStream`.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes.
    pub fn load(path: impl AsRef<Path>) -> Result<VideoStream, VideoError> {
        let path = path.as_ref();
        let mut stream = VideoStream::from_bytes(read(path)?)?;
        stream.path = Some(path.to_path_buf());
        Ok(stream)
    }
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>, VideoError> {
    crate::filesystem::read(path).map_err(|source| VideoError::Read {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests;

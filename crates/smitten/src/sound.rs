//! Sound as samples that can be read and changed.
//!
//! A [`SoundData`] holds a whole sound decoded into memory. Read its
//! samples to draw a waveform or react to loudness, or write them to make
//! sounds in code, then play them with
//! [`Source::from_sound_data`](crate::audio::Source::from_sound_data).
//!
//! A [`Decoder`] reads a file a piece at a time instead, handing back a
//! short [`SoundData`] each time, for working through a long sound without
//! holding all of it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::audio::AudioError;
use crate::audio::decode::{self, Decoded, Frame};

/// A sound's samples, decoded into memory.
///
/// Samples are from -1.0 to 1.0. A stereo sound's samples are interleaved:
/// left, right, left, right. Sample indexes count per channel, so index 10
/// of a stereo sound is the eleventh moment, with a left and a right
/// sample.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundData {
    samples: Vec<f32>,
    rate: u32,
    channels: u16,
}

impl SoundData {
    /// Creates a silent sound `sample_count` samples long per channel.
    ///
    /// Panics if `rate` is zero or `channels` is not 1 or 2.
    pub fn new(sample_count: usize, rate: u32, channels: u16) -> SoundData {
        assert!(rate > 0, "a sound's sample rate must be above zero");
        assert!(
            channels == 1 || channels == 2,
            "a sound must have 1 or 2 channels, not {channels}"
        );
        SoundData {
            samples: vec![0.0; sample_count * usize::from(channels)],
            rate,
            channels,
        }
    }

    /// Decodes the contents of an audio file. Files with more than two
    /// channels keep only their front left and right.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<SoundData, AudioError> {
        let decoded = decode::Decoder::open(bytes.into())
            .map_err(AudioError::Decode)?
            .decode_all();
        Ok(SoundData::from_frames(
            &decoded.frames,
            decoded.rate,
            decoded.channels,
        ))
    }

    /// Samples per second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// 1 for mono or 2 for stereo.
    pub fn channel_count(&self) -> u16 {
        self.channels
    }

    /// How many samples each channel has.
    pub fn sample_count(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// The length of the sound.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.sample_count() as f64 / f64::from(self.rate))
    }

    /// Returns sample `index` of `channel`, where channel 0 is the left (or
    /// only) channel and 1 the right.
    ///
    /// Panics if either is out of range.
    pub fn sample(&self, index: usize, channel: u16) -> f32 {
        self.samples[self.offset(index, channel)]
    }

    /// Changes sample `index` of `channel`. The value is clamped to -1.0 to
    /// 1.0.
    ///
    /// Panics if either is out of range.
    pub fn set_sample(&mut self, index: usize, channel: u16, value: f32) {
        let offset = self.offset(index, channel);
        self.samples[offset] = value.clamp(-1.0, 1.0);
    }

    /// Every sample, interleaved for a stereo sound.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Every sample, to change in place. Values outside -1.0 to 1.0 are
    /// clipped when the sound plays.
    pub fn samples_mut(&mut self) -> &mut [f32] {
        &mut self.samples
    }

    /// Makes a sound from raw PCM samples, interleaved for stereo.
    /// `bit_depth` is 8, for unsigned bytes centred on 128 as in WAV files,
    /// or 16, for signed little-endian pairs of bytes.
    ///
    /// Smitten keeps samples as floats whatever they came from. A partial
    /// sample at the end of `bytes` is ignored.
    ///
    /// Panics if `rate` is zero, `channels` is not 1 or 2, or `bit_depth`
    /// is not 8 or 16.
    pub fn from_pcm(bytes: &[u8], rate: u32, channels: u16, bit_depth: u8) -> SoundData {
        let samples: Vec<f32> = match bit_depth {
            8 => bytes
                .iter()
                .map(|&byte| (f32::from(byte) - 128.0) / 128.0)
                .collect(),
            16 => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                .collect(),
            _ => panic!("PCM samples must be 8 or 16 bit, not {bit_depth}"),
        };
        let frames = samples.len() / usize::from(channels.max(1));
        let mut sound = SoundData::new(frames, rate, channels);
        let length = sound.samples.len();
        sound.samples.copy_from_slice(&samples[..length]);
        sound
    }

    /// The samples as raw PCM, interleaved for stereo, in the layout
    /// [`from_pcm`](Self::from_pcm) reads: 8 bit unsigned or 16 bit signed
    /// little-endian. Samples are clipped to -1.0 to 1.0.
    ///
    /// Panics if `bit_depth` is not 8 or 16.
    pub fn to_pcm(&self, bit_depth: u8) -> Vec<u8> {
        let clipped = self.samples.iter().map(|sample| sample.clamp(-1.0, 1.0));
        match bit_depth {
            8 => clipped
                .map(|sample| (sample * 127.0 + 128.0).round() as u8)
                .collect(),
            16 => clipped
                .flat_map(|sample| ((sample * 32767.0).round() as i16).to_le_bytes())
                .collect(),
            _ => panic!("PCM samples must be 8 or 16 bit, not {bit_depth}"),
        }
    }

    fn offset(&self, index: usize, channel: u16) -> usize {
        assert!(
            channel < self.channels,
            "channel {channel} is out of range for a sound with {} channel(s)",
            self.channels
        );
        assert!(
            index < self.sample_count(),
            "sample {index} is out of range for a sound {} samples long",
            self.sample_count()
        );
        index * usize::from(self.channels) + usize::from(channel)
    }

    /// Wraps samples that are already interleaved.
    pub(crate) fn from_samples(samples: Vec<f32>, rate: u32, channels: u16) -> SoundData {
        SoundData {
            samples,
            rate,
            channels,
        }
    }

    /// Makes a sound from frames as the decoder gives them, which hold a
    /// mono sample twice.
    fn from_frames(frames: &[Frame], rate: u32, channels: u16) -> SoundData {
        let samples = if channels == 1 {
            frames.iter().map(|frame| frame[0]).collect()
        } else {
            frames.iter().flatten().copied().collect()
        };
        SoundData {
            samples,
            rate,
            channels,
        }
    }

    /// The samples in the form the mixer plays.
    pub(crate) fn to_decoded(&self) -> Decoded {
        let frames = if self.channels == 1 {
            self.samples
                .iter()
                .map(|&sample| [sample, sample])
                .collect()
        } else {
            self.samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|frame| [frame[0], frame[1]])
                .collect()
        };
        Decoded {
            frames,
            rate: self.rate,
            channels: self.channels,
        }
    }
}

impl SoundData {
    /// Loads and decodes a sound file.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes.
    pub fn load(path: impl AsRef<Path>) -> Result<SoundData, AudioError> {
        let path = path.as_ref();
        let bytes = crate::filesystem::read(path).map_err(|source| AudioError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        SoundData::from_bytes(bytes)
    }
}

/// How many samples per channel a [`Decoder`] made by [`Decoder::load`]
/// hands back at a time. At 44.1 kHz this is about a tenth of a second.
pub const DEFAULT_DECODER_BUFFER_SIZE: usize = 4_096;

/// Reads a sound file a piece at a time.
///
/// Each call to [`Decoder::decode`] returns the next piece as a
/// [`SoundData`], which suits analysing a long sound or feeding a
/// [queueable source](crate::audio::QueueableSource) with sound that
/// has been changed on the way.
pub struct Decoder {
    bytes: Arc<[u8]>,
    inner: decode::Decoder,
    buffer_size: usize,
    /// Frames decoded but not handed back yet.
    pending: Vec<Frame>,
    /// Frames still to throw away, to reach a seek target in a file that
    /// cannot seek.
    discard: u64,
    /// The index of the next frame `decode` hands back.
    position: u64,
    ended: bool,
}

impl Decoder {
    /// Starts reading the contents of an audio file. Each piece is
    /// `buffer_size` samples per channel long, apart from the last.
    pub fn from_bytes(
        bytes: impl Into<Arc<[u8]>>,
        buffer_size: usize,
    ) -> Result<Decoder, AudioError> {
        let bytes = bytes.into();
        let inner = decode::Decoder::open(bytes.clone()).map_err(AudioError::Decode)?;
        Ok(Decoder {
            bytes,
            inner,
            buffer_size: buffer_size.max(1),
            pending: Vec::new(),
            discard: 0,
            position: 0,
            ended: false,
        })
    }

    /// Returns the next piece of the sound, or `None` once it has all been
    /// read.
    pub fn decode(&mut self) -> Option<SoundData> {
        while self.pending.len() < self.buffer_size && !self.ended {
            let before = self.pending.len();
            if !self.inner.decode_next(&mut self.pending) {
                self.ended = true;
            }
            let fresh = (self.pending.len() - before) as u64;
            let thrown = fresh.min(self.discard);
            self.pending.drain(before..before + thrown as usize);
            self.discard -= thrown;
        }
        if self.pending.is_empty() {
            return None;
        }
        let take = self.pending.len().min(self.buffer_size);
        let piece =
            SoundData::from_frames(&self.pending[..take], self.rate(), self.channel_count());
        self.pending.drain(..take);
        self.position += take as u64;
        Some(piece)
    }

    /// Moves to `position` from the start, so the next piece begins there.
    pub fn seek(&mut self, position: Duration) {
        let frame = (position.as_secs_f64() * f64::from(self.rate())).floor() as u64;
        self.pending.clear();
        self.ended = false;
        self.discard = 0;
        if !self.inner.seek(frame) {
            // Not seekable: read forward from the start instead.
            if let Ok(inner) = decode::Decoder::open(self.bytes.clone()) {
                self.inner = inner;
                self.discard = frame;
            }
        }
        self.position = frame;
    }

    /// Goes back to the start of the sound.
    pub fn rewind(&mut self) {
        self.seek(Duration::ZERO);
    }

    /// Where the next piece starts, from the start.
    pub fn tell(&self) -> Duration {
        Duration::from_secs_f64(self.position as f64 / f64::from(self.rate()))
    }

    /// Samples per second.
    pub fn rate(&self) -> u32 {
        self.inner.rate()
    }

    /// 1 for mono or 2 for stereo. Files with more channels count as
    /// stereo, since only their front left and right are kept.
    pub fn channel_count(&self) -> u16 {
        self.inner.channels()
    }

    /// How many samples per channel each piece holds.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// The length of the sound, when the file states it.
    pub fn duration(&self) -> Option<Duration> {
        let frames = self.inner.total_frames()?;
        Some(Duration::from_secs_f64(
            frames as f64 / f64::from(self.rate()),
        ))
    }

    /// The file being read, for starting a source that plays it.
    pub(crate) fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
}

impl Clone for Decoder {
    /// Makes a decoder for the same file, back at its start.
    fn clone(&self) -> Self {
        Decoder::from_bytes(self.bytes.clone(), self.buffer_size)
            .expect("a sound that opened once can be opened again")
    }
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("rate", &self.rate())
            .field("channels", &self.channel_count())
            .field("buffer_size", &self.buffer_size)
            .field("position", &self.position)
            .finish()
    }
}

impl Decoder {
    /// Opens a sound file for reading a piece at a time. Each piece is
    /// `buffer_size` samples per channel, such as
    /// [`DEFAULT_DECODER_BUFFER_SIZE`].
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes.
    pub fn load(path: impl AsRef<Path>, buffer_size: usize) -> Result<Decoder, AudioError> {
        let path = path.as_ref();
        let bytes = crate::filesystem::read(path).map_err(|source| AudioError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Decoder::from_bytes(bytes, buffer_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decode::tests::{ramp, wav};

    #[test]
    fn pcm_round_trips_at_both_bit_depths() {
        let mut sound = SoundData::new(3, 8000, 2);
        sound
            .samples_mut()
            .copy_from_slice(&[0.0, 1.0, -1.0, 0.5, -0.5, 0.25]);
        for depth in [8, 16] {
            let bytes = sound.to_pcm(depth);
            assert_eq!(bytes.len(), 6 * usize::from(depth / 8));
            let back = SoundData::from_pcm(&bytes, 8000, 2, depth);
            assert_eq!(back.sample_count(), 3);
            let tolerance = if depth == 8 {
                1.0 / 64.0
            } else {
                1.0 / 16384.0
            };
            for (a, b) in sound.samples().iter().zip(back.samples()) {
                assert!((a - b).abs() <= tolerance, "{depth} bit: {a} became {b}");
            }
        }
        assert_eq!(
            SoundData::from_pcm(&[128, 255, 0], 8000, 1, 8).sample(0, 0),
            0.0
        );
        assert_eq!(
            SoundData::from_pcm(&[0, 0x80, 0xff], 8000, 1, 16).samples(),
            &[-1.0]
        );
    }

    #[test]
    fn a_decoder_hands_back_pieces_of_the_buffer_size() {
        let file = wav(22_050, 1, &ramp(10_000));
        let whole = SoundData::from_bytes(file.clone()).unwrap();
        let mut decoder = Decoder::from_bytes(file, 3_000).unwrap();
        assert_eq!(decoder.rate(), 22_050);
        assert_eq!(decoder.channel_count(), 1);
        assert!((decoder.duration().unwrap().as_secs_f64() - 10_000.0 / 22_050.0).abs() < 1e-9);

        let mut lengths = Vec::new();
        let mut samples = Vec::new();
        while let Some(piece) = decoder.decode() {
            lengths.push(piece.sample_count());
            samples.extend_from_slice(piece.samples());
        }
        assert_eq!(lengths, [3_000, 3_000, 3_000, 1_000]);
        assert_eq!(samples, whole.samples());
        assert!(decoder.decode().is_none());
        assert!((decoder.tell().as_secs_f64() - 10_000.0 / 22_050.0).abs() < 1e-9);
    }

    #[test]
    fn a_decoder_seeks_and_rewinds() {
        let file = wav(10_000, 1, &ramp(10_000));
        let whole = SoundData::from_bytes(file.clone()).unwrap();
        let mut decoder = Decoder::from_bytes(file, 500).unwrap();
        decoder.seek(Duration::from_secs_f64(0.25));
        assert!((decoder.tell().as_secs_f64() - 0.25).abs() < 1e-9);
        let piece = decoder.decode().unwrap();
        assert_eq!(piece.samples(), &whole.samples()[2_500..3_000]);

        decoder.rewind();
        let piece = decoder.decode().unwrap();
        assert_eq!(piece.samples(), &whole.samples()[..500]);

        // A clone starts again from the beginning.
        decoder.decode();
        let mut copy = decoder.clone();
        assert_eq!(copy.decode().unwrap().samples(), &whole.samples()[..500]);
    }

    #[test]
    fn a_stereo_decoder_keeps_both_channels() {
        let mut decoder =
            Decoder::from_bytes(wav(8_000, 2, &[16_384, -16_384, 0, 8_192]), 10).unwrap();
        let piece = decoder.decode().unwrap();
        assert_eq!(piece.channel_count(), 2);
        assert_eq!(piece.sample_count(), 2);
        assert!(piece.sample(0, 0) > 0.4 && piece.sample(0, 1) < -0.4);
    }

    #[test]
    fn a_mono_file_keeps_one_channel() {
        let data = SoundData::from_bytes(wav(8_000, 1, &[0, 16_384, -16_384])).unwrap();
        assert_eq!(data.channel_count(), 1);
        assert_eq!(data.rate(), 8_000);
        assert_eq!(data.sample_count(), 3);
        assert!((data.sample(1, 0) - 0.5).abs() < 1e-3);
        assert!((data.sample(2, 0) + 0.5).abs() < 1e-3);
        assert!((data.duration().as_secs_f64() - 3.0 / 8_000.0).abs() < 1e-9);
    }

    #[test]
    fn a_stereo_file_interleaves_left_and_right() {
        let data = SoundData::from_bytes(wav(44_100, 2, &[16_384, -16_384, 0, 8_192])).unwrap();
        assert_eq!(data.channel_count(), 2);
        assert_eq!(data.sample_count(), 2);
        assert_eq!(data.samples().len(), 4);
        assert!(data.sample(0, 0) > 0.4 && data.sample(0, 1) < -0.4);
        assert!(data.sample(1, 1) > 0.2);
    }

    #[test]
    fn samples_can_be_written_and_are_clamped() {
        let mut data = SoundData::new(4, 22_050, 2);
        assert!(data.samples().iter().all(|&sample| sample == 0.0));
        data.set_sample(3, 1, 0.25);
        data.set_sample(0, 0, 5.0);
        assert_eq!(data.sample(3, 1), 0.25);
        assert_eq!(data.sample(0, 0), 1.0);
        assert_eq!(data.samples()[7], 0.25);
    }

    #[test]
    #[should_panic(expected = "channel 1 is out of range")]
    fn reading_a_missing_channel_panics() {
        SoundData::new(4, 22_050, 1).sample(0, 1);
    }

    #[test]
    fn the_mixer_hears_mono_as_mono() {
        let mut data = SoundData::new(2, 48_000, 1);
        data.set_sample(1, 0, 0.5);
        let decoded = data.to_decoded();
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.frames, [[0.0, 0.0], [0.5, 0.5]]);
    }

    #[test]
    fn not_audio_is_an_error() {
        assert!(SoundData::from_bytes(&b"not a sound"[..]).is_err());
    }
}

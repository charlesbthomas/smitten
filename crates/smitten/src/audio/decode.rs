//! Turns encoded audio files into stereo samples, using symphonia.

use std::io::Cursor;
use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder as CodecDecoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One moment of stereo sound: left and right, each from -1.0 to 1.0.
pub(crate) type Frame = [f32; 2];

/// A whole sound decoded into memory.
pub(crate) struct Decoded {
    pub frames: Vec<Frame>,
    /// Frames per second.
    pub rate: u32,
    /// 1 for mono, whose frames hold the same sample twice, or 2 for
    /// stereo.
    pub channels: u16,
}

/// Decodes a file a little at a time.
pub(crate) struct Decoder {
    format: Box<dyn FormatReader>,
    codec: Box<dyn CodecDecoder>,
    track_id: u32,
    rate: u32,
    /// 1 or 2, from the file's header and then from what it decodes to.
    channels: u16,
    total_frames: Option<u64>,
    /// Frames still to throw away after a seek that landed early.
    skip: u64,
}

impl Decoder {
    /// Starts decoding an audio file held in memory. The format is detected
    /// from its contents.
    pub fn open(bytes: Arc<[u8]>) -> Result<Self, String> {
        let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                stream,
                // Gapless: the encoder's start delay and end padding are
                // trimmed, so a file decodes to the length it was made at
                // and stems of one song stay the same length as they loop.
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(|error| format!("unrecognized audio format ({error})"))?;
        let format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("the file contains no audio")?;
        let rate = track
            .codec_params
            .sample_rate
            .filter(|&rate| rate > 0)
            .ok_or("the file does not say its sample rate")?;
        let codec = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|error| format!("unsupported audio codec ({error})"))?;

        let channels = match track.codec_params.channels.map(|channels| channels.count()) {
            Some(1) => 1,
            _ => 2,
        };
        Ok(Self {
            track_id: track.id,
            channels,
            total_frames: track.codec_params.n_frames,
            rate,
            format,
            codec,
            skip: 0,
        })
    }

    /// Frames per second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// 1 for mono or 2 for stereo. Files with more channels count as
    /// stereo, since only their front left and right are kept.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// The length of the sound, when the file states it.
    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    /// Decodes the next piece of the file onto the end of `out`.
    /// Returns `false` once the file has ended.
    pub fn decode_next(&mut self, out: &mut Vec<Frame>) -> bool {
        loop {
            // Any error from the container, including a clean end of file,
            // means there is nothing more to read.
            let Ok(packet) = self.format.next_packet() else {
                return false;
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.codec.decode(&packet) {
                Ok(decoded) => decoded,
                // A damaged packet is skipped; the rest may still play.
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            };

            let spec = *decoded.spec();
            let channels = spec.channels.count();
            if channels == 0 {
                continue;
            }
            self.channels = if channels == 1 { 1 } else { 2 };
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            samples.copy_interleaved_ref(decoded);

            let before = out.len();
            for frame in samples.samples().chunks_exact(channels) {
                if self.skip > 0 {
                    self.skip -= 1;
                    continue;
                }
                // Mono plays from both speakers. Beyond stereo, only the
                // front left and right channels are kept.
                let right = if channels > 1 { frame[1] } else { frame[0] };
                out.push([frame[0], right]);
            }
            if out.len() > before {
                return true;
            }
        }
    }

    /// Moves to `frame`, so the next decoded frame is that one.
    /// Returns `false` if the file cannot seek.
    pub fn seek(&mut self, frame: u64) -> bool {
        let target = SeekTo::TimeStamp {
            ts: frame,
            track_id: self.track_id,
        };
        match self.format.seek(SeekMode::Accurate, target) {
            Ok(landed) => {
                self.codec.reset();
                // Formats may only be able to land on an earlier packet.
                self.skip = landed.required_ts.saturating_sub(landed.actual_ts);
                true
            }
            Err(_) => false,
        }
    }

    /// Decodes everything that is left.
    pub fn decode_all(mut self) -> Decoded {
        let mut frames = Vec::with_capacity(self.total_frames.unwrap_or(0) as usize);
        while self.decode_next(&mut frames) {}
        Decoded {
            frames,
            rate: self.rate,
            channels: self.channels,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a 16-bit PCM WAV file from interleaved samples.
    pub(crate) fn wav(rate: u32, channels: u16, samples: &[i16]) -> Arc<[u8]> {
        let data_bytes = samples.len() as u32 * 2;
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&channels.to_le_bytes());
        file.extend_from_slice(&rate.to_le_bytes());
        file.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        file.extend_from_slice(&(channels * 2).to_le_bytes());
        file.extend_from_slice(&16u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in samples {
            file.extend_from_slice(&sample.to_le_bytes());
        }
        file.into()
    }

    /// A mono ramp long enough to span several packets.
    pub(crate) fn ramp(frames: usize) -> Vec<i16> {
        (0..frames).map(|i| (i % 30_000) as i16).collect()
    }

    #[test]
    fn mono_is_decoded_to_both_channels() {
        let decoder = Decoder::open(wav(8_000, 1, &[0, 16_384, -16_384])).unwrap();
        assert_eq!(decoder.rate(), 8_000);
        assert_eq!(decoder.total_frames(), Some(3));
        assert_eq!(decoder.channels(), 1);

        let sound = decoder.decode_all();
        assert_eq!(sound.frames.len(), 3);
        assert_eq!(sound.frames[0], [0.0, 0.0]);
        assert!((sound.frames[1][0] - 0.5).abs() < 1e-3);
        assert_eq!(sound.frames[1][0], sound.frames[1][1]);
        assert!((sound.frames[2][0] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn stereo_keeps_left_and_right_apart() {
        let sound = Decoder::open(wav(44_100, 2, &[16_384, -16_384, 0, 8_192]))
            .unwrap()
            .decode_all();
        assert_eq!(sound.rate, 44_100);
        assert_eq!(sound.channels, 2);
        assert_eq!(sound.frames.len(), 2);
        assert!(sound.frames[0][0] > 0.4 && sound.frames[0][1] < -0.4);
        assert!(sound.frames[1][0].abs() < 1e-3 && sound.frames[1][1] > 0.2);
    }

    #[test]
    fn decoding_in_pieces_gives_the_same_frames_as_all_at_once() {
        let file = wav(22_050, 1, &ramp(10_000));
        let whole = Decoder::open(file.clone()).unwrap().decode_all();

        let mut pieces = Vec::new();
        let mut decoder = Decoder::open(file).unwrap();
        let mut calls = 0;
        while decoder.decode_next(&mut pieces) {
            calls += 1;
        }
        assert!(calls > 1, "the file should span several packets");
        assert_eq!(pieces, whole.frames);
        assert_eq!(pieces.len(), 10_000);
    }

    #[test]
    fn seeking_continues_from_the_requested_frame() {
        let file = wav(22_050, 1, &ramp(10_000));
        let whole = Decoder::open(file.clone()).unwrap().decode_all();

        let mut decoder = Decoder::open(file).unwrap();
        assert!(decoder.seek(6_000));
        let mut rest = Vec::new();
        while decoder.decode_next(&mut rest) {}
        assert_eq!(rest, whole.frames[6_000..]);
    }

    #[test]
    fn data_that_is_not_audio_is_an_error() {
        assert!(Decoder::open(Arc::from(&b"definitely not audio"[..])).is_err());
    }

    /// A Vorbis file decodes to exactly the frames it was encoded from,
    /// with no start delay or end padding left in.
    #[test]
    fn ogg_vorbis_decodes_to_its_own_length() {
        let bytes: Arc<[u8]> = include_bytes!("../../../examples/assets/loop.ogg").as_slice().into();
        let decoder = Decoder::open(bytes).unwrap();
        assert_eq!(decoder.total_frames(), Some(176_400));
        let decoded = decoder.decode_all();
        assert_eq!((decoded.rate, decoded.frames.len()), (44_100, 176_400));
    }
}

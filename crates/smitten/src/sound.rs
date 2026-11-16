//! Sound as samples that can be read and changed, the equivalent of
//! `love.sound`.
//!
//! A [`SoundData`] holds a whole sound decoded into memory. Read its
//! samples to draw a waveform or react to loudness, or write them to make
//! sounds in code, then play them with
//! [`Source::from_sound_data`](crate::audio::Source::from_sound_data).

use std::path::Path;
use std::sync::Arc;

use crate::audio::AudioError;
use crate::audio::decode::{Decoded, Decoder};

/// A sound's samples, the equivalent of LÖVE's `SoundData`.
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
        let decoded = Decoder::open(bytes.into())
            .map_err(AudioError::Decode)?
            .decode_all();
        let samples = if decoded.channels == 1 {
            decoded.frames.iter().map(|frame| frame[0]).collect()
        } else {
            decoded.frames.iter().flatten().copied().collect()
        };
        Ok(SoundData {
            samples,
            rate: decoded.rate,
            channels: decoded.channels,
        })
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

    /// The length in seconds.
    pub fn duration(&self) -> f64 {
        self.sample_count() as f64 / f64::from(self.rate)
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

    /// The samples in the form the mixer plays.
    pub(crate) fn to_decoded(&self) -> Decoded {
        let frames = if self.channels == 1 {
            self.samples
                .iter()
                .map(|&sample| [sample, sample])
                .collect()
        } else {
            self.samples
                .chunks_exact(2)
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

/// Loads and decodes a sound file, the equivalent of
/// `love.sound.newSoundData(path)`.
///
/// A relative path is looked up as [`crate::filesystem`] describes. Panics
/// with the file name if it cannot be loaded; use [`try_new_sound_data`]
/// to handle the failure instead.
pub fn new_sound_data(path: impl AsRef<Path>) -> SoundData {
    let path = path.as_ref();
    try_new_sound_data(path).unwrap_or_else(|error| match error {
        AudioError::Decode(message) => {
            panic!("could not load sound {}: {message}", path.display())
        }
        error => panic!("{error}"),
    })
}

/// Loads and decodes a sound file, returning an error if it fails.
pub fn try_new_sound_data(path: impl AsRef<Path>) -> Result<SoundData, AudioError> {
    let path = path.as_ref();
    let bytes = crate::filesystem::read(path).map_err(|source| AudioError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    SoundData::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decode::tests::wav;

    #[test]
    fn a_mono_file_keeps_one_channel() {
        let data = SoundData::from_bytes(wav(8_000, 1, &[0, 16_384, -16_384])).unwrap();
        assert_eq!(data.channel_count(), 1);
        assert_eq!(data.rate(), 8_000);
        assert_eq!(data.sample_count(), 3);
        assert!((data.sample(1, 0) - 0.5).abs() < 1e-3);
        assert!((data.sample(2, 0) + 0.5).abs() < 1e-3);
        assert!((data.duration() - 3.0 / 8_000.0).abs() < 1e-9);
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

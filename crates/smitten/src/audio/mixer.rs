//! Combines every playing sound into the stream sent to the speakers.
//!
//! The mixer is shared between the game, which starts and adjusts sounds,
//! and the audio thread, which calls [`Mixer::mix`]. It has no dependency
//! on a platform, so everything here can be tested by mixing into a buffer.

use std::collections::HashMap;
use std::sync::Arc;

use smitten_core::backend::AUDIO_SAMPLE_RATE;

use super::decode::{Decoder, Frame, SoundData};

/// Where a voice gets its frames from.
pub(crate) enum Data {
    /// Fully decoded up front. Cheap to play many times at once.
    Static(Arc<SoundData>),
    /// Decoded while it plays. Uses little memory however long it is.
    Stream(Stream),
}

/// A sliding window over a file that is decoded as playback advances.
pub(crate) struct Stream {
    /// Kept so the stream can be reopened if it cannot seek.
    bytes: Arc<[u8]>,
    decoder: Decoder,
    window: Vec<Frame>,
    /// The index of `window[0]` within the whole sound.
    window_start: u64,
    /// The sound's length, known for certain once decoding reaches the end.
    end: Option<u64>,
}

impl Stream {
    pub fn new(bytes: Arc<[u8]>) -> Result<Self, String> {
        let decoder = Decoder::open(bytes.clone())?;
        Ok(Self {
            bytes,
            decoder,
            window: Vec::new(),
            window_start: 0,
            end: None,
        })
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    /// Returns frame `index`, decoding forward as needed, or `None` if the
    /// sound is shorter than that. Asking for an earlier frame than last
    /// time requires a [`Stream::seek`] first.
    fn frame(&mut self, index: u64) -> Option<Frame> {
        loop {
            if index < self.window_start {
                return None;
            }
            let offset = (index - self.window_start) as usize;
            if let Some(&frame) = self.window.get(offset) {
                return Some(frame);
            }
            if self.end.is_some() {
                return None;
            }
            // Keep one frame of history, since the mixer blends each frame
            // with the next, then decode some more.
            let keep_from = offset.saturating_sub(1).min(self.window.len());
            self.window.drain(..keep_from);
            self.window_start += keep_from as u64;
            if !self.decoder.decode_next(&mut self.window) {
                self.end = Some(self.window_start + self.window.len() as u64);
            }
        }
    }

    fn seek(&mut self, index: u64) {
        self.window.clear();
        self.end = None;
        if self.decoder.seek(index) {
            self.window_start = index;
        } else if let Ok(decoder) = Decoder::open(self.bytes.clone()) {
            // Not seekable: start over and let playback read forward.
            self.decoder = decoder;
            self.window_start = 0;
        }
    }
}

impl Data {
    fn rate(&self) -> u32 {
        match self {
            Data::Static(sound) => sound.rate,
            Data::Stream(stream) => stream.decoder.rate(),
        }
    }

    fn frame(&mut self, index: u64) -> Option<Frame> {
        match self {
            Data::Static(sound) => sound.frames.get(usize::try_from(index).ok()?).copied(),
            Data::Stream(stream) => stream.frame(index),
        }
    }

    /// The length in frames, if it is known yet.
    fn len(&self) -> Option<u64> {
        match self {
            Data::Static(sound) => Some(sound.frames.len() as u64),
            Data::Stream(stream) => stream.end.or(stream.decoder.total_frames()),
        }
    }

    fn seek(&mut self, index: u64) {
        if let Data::Stream(stream) = self {
            stream.seek(index);
        }
    }
}

pub(crate) struct Voice {
    data: Data,
    /// Playback position in the sound's own frames. Fractional because the
    /// sound's rate and pitch rarely match the output rate exactly.
    position: f64,
    playing: bool,
    looping: bool,
    volume: f32,
    pitch: f32,
    /// Set when the game dropped its handle while the sound was playing.
    /// The voice is removed as soon as it stops.
    orphaned: bool,
    /// Shared by a loaded sound and every copy made of it, so that all of
    /// them can be given a newer version of the file. Zero until the voice
    /// joins the mixer.
    group: u64,
}

impl Voice {
    pub fn new(data: Data) -> Self {
        Self {
            data,
            position: 0.0,
            playing: false,
            looping: false,
            volume: 1.0,
            pitch: 1.0,
            orphaned: false,
            group: 0,
        }
    }

    /// Adds this voice's sound to `out` and advances it.
    fn mix_into(&mut self, out: &mut [f32], master: f32) {
        let step = f64::from(self.data.rate()) / f64::from(AUDIO_SAMPLE_RATE)
            * f64::from(self.pitch.max(0.0));
        let gain = self.volume * master;

        for output in out.chunks_exact_mut(2) {
            let mut index = self.position as u64;
            let mut found = self.data.frame(index);
            if found.is_none() {
                // Ran off the end of the sound.
                let length = self.data.len().unwrap_or(index);
                if self.looping && length > 0 {
                    self.position = (self.position - length as f64).max(0.0);
                    index = self.position as u64;
                    self.data.seek(index);
                    found = self.data.frame(index);
                }
            }
            let Some(current) = found else {
                self.playing = false;
                self.position = 0.0;
                self.data.seek(0);
                return;
            };
            // Blend toward the next frame for a smooth result when the
            // position falls between two frames.
            let next = self.data.frame(index + 1).unwrap_or(current);
            let blend = (self.position - index as f64) as f32;
            output[0] += (current[0] + (next[0] - current[0]) * blend) * gain;
            output[1] += (current[1] + (next[1] - current[1]) * blend) * gain;
            self.position += step;
        }
    }
}

pub(crate) struct Mixer {
    voices: HashMap<u64, Voice>,
    next_id: u64,
    master: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            voices: HashMap::new(),
            next_id: 1,
            master: 1.0,
        }
    }
}

impl Mixer {
    /// Fills `out` with interleaved stereo at [`AUDIO_SAMPLE_RATE`].
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for voice in self.voices.values_mut().filter(|voice| voice.playing) {
            voice.mix_into(out, self.master);
        }
        self.voices
            .retain(|_, voice| voice.playing || !voice.orphaned);
        // Loud sounds added together can exceed what the device accepts.
        for sample in out {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    pub fn add(&mut self, mut voice: Voice) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if voice.group == 0 {
            voice.group = id;
        }
        self.voices.insert(id, voice);
        id
    }

    /// Called when the game drops its handle. A sound that is still
    /// playing is allowed to finish, as in LÖVE.
    pub fn release(&mut self, id: u64) {
        match self.voices.get_mut(&id) {
            Some(voice) if voice.playing && !voice.looping => voice.orphaned = true,
            _ => {
                self.voices.remove(&id);
            }
        }
    }

    /// Creates an independent voice that plays the same sound with the
    /// same settings, stopped at the beginning.
    pub fn duplicate(&mut self, id: u64) -> Result<u64, String> {
        let original = self.voices.get(&id).ok_or("the source no longer exists")?;
        let data = match &original.data {
            Data::Static(sound) => Data::Static(sound.clone()),
            Data::Stream(stream) => Data::Stream(Stream::new(stream.bytes().clone())?),
        };
        let copy = Voice {
            looping: original.looping,
            volume: original.volume,
            pitch: original.pitch,
            group: original.group,
            ..Voice::new(data)
        };
        Ok(self.add(copy))
    }

    /// The voices the game still holds that came from the sound loaded as
    /// `group`, and whether each is streamed.
    pub fn group(&self, group: u64) -> Vec<(u64, bool)> {
        self.voices
            .iter()
            .filter(|(_, voice)| voice.group == group && !voice.orphaned)
            .map(|(&id, voice)| (id, matches!(voice.data, Data::Stream(_))))
            .collect()
    }

    /// Gives a voice a different sound to play. It keeps its settings and
    /// whether it is playing, and starts the new sound from the beginning.
    pub fn replace_data(&mut self, id: u64, data: Data) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.data = data;
            voice.position = 0.0;
        }
    }

    pub fn play(&mut self, id: u64) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.playing = true;
        }
    }

    pub fn pause(&mut self, id: u64) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.playing = false;
        }
    }

    pub fn stop(&mut self, id: u64) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.playing = false;
            voice.position = 0.0;
            voice.data.seek(0);
        }
    }

    pub fn pause_all(&mut self) {
        for voice in self.voices.values_mut() {
            voice.playing = false;
        }
    }

    pub fn stop_all(&mut self) {
        for voice in self.voices.values_mut() {
            voice.playing = false;
            voice.position = 0.0;
            voice.data.seek(0);
        }
        self.voices.retain(|_, voice| !voice.orphaned);
    }

    pub fn is_playing(&self, id: u64) -> bool {
        self.voices.get(&id).is_some_and(|voice| voice.playing)
    }

    pub fn playing_count(&self) -> usize {
        self.voices.values().filter(|voice| voice.playing).count()
    }

    pub fn set_looping(&mut self, id: u64, looping: bool) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.looping = looping;
        }
    }

    pub fn is_looping(&self, id: u64) -> bool {
        self.voices.get(&id).is_some_and(|voice| voice.looping)
    }

    pub fn set_volume(&mut self, id: u64, volume: f32) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.volume = volume.max(0.0);
        }
    }

    pub fn volume(&self, id: u64) -> f32 {
        self.voices.get(&id).map_or(0.0, |voice| voice.volume)
    }

    pub fn set_pitch(&mut self, id: u64, pitch: f32) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.pitch = pitch.max(0.0);
        }
    }

    pub fn pitch(&self, id: u64) -> f32 {
        self.voices.get(&id).map_or(1.0, |voice| voice.pitch)
    }

    /// Moves playback to `seconds` from the start.
    pub fn seek(&mut self, id: u64, seconds: f64) {
        if let Some(voice) = self.voices.get_mut(&id) {
            let frame = (seconds.max(0.0) * f64::from(voice.data.rate())).floor();
            voice.position = frame;
            voice.data.seek(frame as u64);
        }
    }

    /// The playback position in seconds from the start.
    pub fn tell(&self, id: u64) -> f64 {
        self.voices
            .get(&id)
            .map_or(0.0, |voice| voice.position / f64::from(voice.data.rate()))
    }

    /// The sound's length in seconds, if it is known.
    pub fn duration(&self, id: u64) -> Option<f64> {
        let voice = self.voices.get(&id)?;
        Some(voice.data.len()? as f64 / f64::from(voice.data.rate()))
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master = volume.max(0.0);
    }

    pub fn master_volume(&self) -> f32 {
        self.master
    }
}

#[cfg(test)]
mod tests {
    use super::super::decode::tests::{ramp, wav};
    use super::*;

    const RATE: u32 = AUDIO_SAMPLE_RATE;

    /// A sound at the output rate whose every sample is `level`.
    fn constant(frames: usize, level: f32) -> Data {
        Data::Static(Arc::new(SoundData {
            frames: vec![[level, level]; frames],
            rate: RATE,
        }))
    }

    /// Mixes `frames` frames and returns the left channel.
    fn left(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        mixer.mix(&mut out);
        out.chunks_exact(2).map(|frame| frame[0]).collect()
    }

    #[test]
    fn copies_of_a_sound_belong_to_its_group() {
        let mut mixer = Mixer::default();
        let original = mixer.add(Voice::new(constant(4, 0.5)));
        let copy = mixer.duplicate(original).unwrap();
        let other = mixer.add(Voice::new(constant(4, 0.25)));

        let mut group: Vec<u64> = mixer.group(original).iter().map(|&(id, _)| id).collect();
        group.sort_unstable();
        assert_eq!(group, [original, copy]);
        assert_eq!(mixer.group(other), [(other, false)]);

        // Voices the game has let go of are no longer part of it.
        mixer.release(original);
        mixer.release(copy);
        assert!(mixer.group(original).is_empty());
    }

    #[test]
    fn a_replaced_sound_keeps_playing_from_its_start_with_its_settings() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(100, 0.5)));
        mixer.set_volume(id, 0.5);
        mixer.play(id);
        assert_eq!(left(&mut mixer, 2), [0.25, 0.25]);

        mixer.replace_data(id, constant(3, 1.0));
        assert!(mixer.is_playing(id));
        assert_eq!(left(&mut mixer, 4), [0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn nothing_plays_until_play_is_called() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(100, 0.5)));
        assert!(left(&mut mixer, 10).iter().all(|&sample| sample == 0.0));

        mixer.play(id);
        assert!(left(&mut mixer, 10).iter().all(|&sample| sample == 0.5));
    }

    #[test]
    fn a_sound_stops_and_rewinds_when_it_ends() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(4, 0.5)));
        mixer.play(id);

        assert_eq!(left(&mut mixer, 6), [0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        assert!(!mixer.is_playing(id));
        assert_eq!(mixer.tell(id), 0.0);
    }

    #[test]
    fn a_looping_sound_keeps_going() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(Data::Static(Arc::new(SoundData {
            frames: vec![[0.1; 2], [0.2; 2], [0.3; 2]],
            rate: RATE,
        }))));
        mixer.set_looping(id, true);
        mixer.play(id);

        // The last frame holds its value instead of blending into silence.
        assert_eq!(left(&mut mixer, 7), [0.1, 0.2, 0.3, 0.1, 0.2, 0.3, 0.1]);
        assert!(mixer.is_playing(id));
    }

    #[test]
    fn source_and_master_volume_multiply() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(100, 0.8)));
        mixer.set_volume(id, 0.5);
        mixer.set_master_volume(0.5);
        mixer.play(id);

        assert!((left(&mut mixer, 1)[0] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn sounds_add_together_and_are_clamped() {
        let mut mixer = Mixer::default();
        for _ in 0..3 {
            let id = mixer.add(Voice::new(constant(100, 0.5)));
            mixer.play(id);
        }
        assert_eq!(mixer.playing_count(), 3);
        assert_eq!(left(&mut mixer, 1), [1.0]);
    }

    #[test]
    fn pause_keeps_the_position_and_stop_rewinds() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(RATE as usize, 0.5)));
        mixer.play(id);
        left(&mut mixer, RATE as usize / 4);

        mixer.pause(id);
        assert!((mixer.tell(id) - 0.25).abs() < 1e-6);
        assert!(left(&mut mixer, 10).iter().all(|&sample| sample == 0.0));
        assert!((mixer.tell(id) - 0.25).abs() < 1e-6);

        mixer.stop(id);
        assert_eq!(mixer.tell(id), 0.0);
    }

    #[test]
    fn doubling_the_pitch_halves_the_playing_time() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(100, 0.5)));
        mixer.set_pitch(id, 2.0);
        mixer.play(id);

        let heard = left(&mut mixer, 100);
        assert_eq!(heard.iter().filter(|&&sample| sample != 0.0).count(), 50);
    }

    #[test]
    fn a_slower_sound_is_stretched_to_the_output_rate() {
        // Half the output rate, so each source frame covers two outputs
        // and the values in between are blended.
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(Data::Static(Arc::new(SoundData {
            frames: vec![[0.0; 2], [1.0; 2]],
            rate: RATE / 2,
        }))));
        mixer.play(id);

        assert_eq!(left(&mut mixer, 5), [0.0, 0.5, 1.0, 1.0, 0.0]);
        assert_eq!(mixer.duration(id), Some(2.0 / f64::from(RATE / 2)));
    }

    #[test]
    fn seek_moves_the_position() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(RATE as usize, 0.5)));
        mixer.seek(id, 0.5);
        assert_eq!(mixer.tell(id), 0.5);
        assert_eq!(mixer.duration(id), Some(1.0));
    }

    #[test]
    fn a_dropped_source_finishes_playing_and_is_then_forgotten() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(4, 0.5)));
        mixer.play(id);
        mixer.release(id);

        assert_eq!(left(&mut mixer, 2), [0.5, 0.5]);
        assert_eq!(mixer.voices.len(), 1);
        left(&mut mixer, 8);
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn a_dropped_source_that_is_stopped_or_looping_is_removed_at_once() {
        let mut mixer = Mixer::default();
        let stopped = mixer.add(Voice::new(constant(4, 0.5)));
        mixer.release(stopped);

        // A loop would otherwise play forever with nothing able to stop it.
        let looping = mixer.add(Voice::new(constant(4, 0.5)));
        mixer.set_looping(looping, true);
        mixer.play(looping);
        mixer.release(looping);

        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn a_duplicate_plays_independently_with_the_same_settings() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(100, 0.5)));
        mixer.set_volume(id, 0.5);
        mixer.play(id);

        let copy = mixer.duplicate(id).unwrap();
        assert!(!mixer.is_playing(copy));
        assert_eq!(mixer.volume(copy), 0.5);

        mixer.play(copy);
        assert_eq!(left(&mut mixer, 1), [0.5]);
    }

    #[test]
    fn a_stream_sounds_the_same_as_the_decoded_sound() {
        let file = wav(RATE, 1, &ramp(10_000));
        let decoded = Decoder::open(file.clone()).unwrap().decode_all();

        let mut mixer = Mixer::default();
        let stream = mixer.add(Voice::new(Data::Stream(Stream::new(file).unwrap())));
        mixer.play(stream);
        let streamed = left(&mut mixer, 10_050);

        let expected: Vec<f32> = decoded.frames.iter().map(|frame| frame[0]).collect();
        assert_eq!(streamed[..10_000], expected[..]);
        assert!(streamed[10_000..].iter().all(|&sample| sample == 0.0));
        assert!(!mixer.is_playing(stream));
    }

    #[test]
    fn a_looping_stream_starts_over_from_the_beginning() {
        let file = wav(RATE, 1, &ramp(3_000));
        let mut mixer = Mixer::default();
        let stream = mixer.add(Voice::new(Data::Stream(Stream::new(file).unwrap())));
        mixer.set_looping(stream, true);
        mixer.play(stream);

        let heard = left(&mut mixer, 7_000);
        assert_eq!(heard[..3_000], heard[3_000..6_000]);
        assert_eq!(heard[..1_000], heard[6_000..]);
        assert!(mixer.is_playing(stream));
    }

    #[test]
    fn seeking_a_stream_plays_from_the_new_position() {
        let file = wav(RATE, 1, &ramp(10_000));
        let decoded = Decoder::open(file.clone()).unwrap().decode_all();

        let mut mixer = Mixer::default();
        let stream = mixer.add(Voice::new(Data::Stream(Stream::new(file).unwrap())));
        mixer.play(stream);
        left(&mut mixer, 500);
        mixer.seek(stream, 6_000.0 / f64::from(RATE));

        let heard = left(&mut mixer, 100);
        let expected: Vec<f32> = decoded.frames[6_000..6_100]
            .iter()
            .map(|frame| frame[0])
            .collect();
        assert_eq!(heard, expected);
    }
}

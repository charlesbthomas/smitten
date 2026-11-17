//! Combines every playing sound into the stream sent to the speakers.
//!
//! The mixer is shared between the game, which starts and adjusts sounds,
//! and the audio thread, which calls [`Mixer::mix`]. It has no dependency
//! on a platform, so everything here can be tested by mixing into a buffer.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use smitten_core::backend::AUDIO_SAMPLE_RATE;

use super::decode::{Decoded, Decoder, Frame};
use super::effects::{Effect, EffectState, Filter, FilterState};
use super::spatial::{Heard, Listener, Placement};

/// Where a voice gets its frames from.
pub(crate) enum Data {
    /// Fully decoded up front. Cheap to play many times at once.
    Static(Arc<Decoded>),
    /// Decoded while it plays. Uses little memory however long it is.
    Stream(Stream),
    /// Buffers of samples handed over by the game as it makes them.
    Queue(Queue),
}

/// The buffers waiting to be played by a queueable source.
pub(crate) struct Queue {
    buffers: VecDeque<Vec<Frame>>,
    /// The index of `buffers[0][0]` counted from when the source started.
    base: u64,
    rate: u32,
    channels: u16,
    /// How many buffers may wait at once.
    capacity: usize,
}

impl Queue {
    pub fn new(rate: u32, channels: u16, capacity: usize) -> Self {
        Self {
            buffers: VecDeque::new(),
            base: 0,
            rate,
            channels,
            capacity: capacity.max(1),
        }
    }

    /// Returns frame `index`, forgetting buffers that have been played
    /// past. As with a stream, frames earlier than the last one asked for
    /// are gone.
    fn frame(&mut self, index: u64) -> Option<Frame> {
        // A buffer is kept while it holds the frame before `index`, since
        // the mixer blends each frame with the next and may ask for that
        // one again.
        while let Some(front) = self.buffers.front() {
            let end = self.base + front.len() as u64;
            if end >= index {
                break;
            }
            self.base = end;
            self.buffers.pop_front();
        }
        let mut offset = index.checked_sub(self.base)?;
        for buffer in &self.buffers {
            if let Some(&frame) = buffer.get(offset as usize) {
                return Some(frame);
            }
            offset -= buffer.len() as u64;
        }
        None
    }

    /// Forgets everything queued and counts from the start again.
    fn clear(&mut self) {
        self.buffers.clear();
        self.base = 0;
    }
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
            Data::Queue(queue) => queue.rate,
        }
    }

    /// 1 for mono or 2 for stereo.
    fn channels(&self) -> u16 {
        match self {
            Data::Static(sound) => sound.channels,
            Data::Stream(stream) => stream.decoder.channels(),
            Data::Queue(queue) => queue.channels,
        }
    }

    fn frame(&mut self, index: u64) -> Option<Frame> {
        match self {
            Data::Static(sound) => sound.frames.get(usize::try_from(index).ok()?).copied(),
            Data::Stream(stream) => stream.frame(index),
            Data::Queue(queue) => queue.frame(index),
        }
    }

    /// The length in frames, if it is known yet.
    fn len(&self) -> Option<u64> {
        match self {
            Data::Static(sound) => Some(sound.frames.len() as u64),
            Data::Stream(stream) => stream.end.or(stream.decoder.total_frames()),
            // A queue goes on for as long as the game keeps feeding it.
            Data::Queue(_) => None,
        }
    }

    /// Moves to `index`. A queue can only go back to the start, which
    /// empties it, as stopping a queueable source does in LÖVE.
    fn seek(&mut self, index: u64) {
        match self {
            Data::Static(_) => {}
            Data::Stream(stream) => stream.seek(index),
            Data::Queue(queue) => {
                if index == 0 {
                    queue.clear();
                }
            }
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
    placement: Placement,
    /// The filter on what the source sends straight to the speakers.
    filter: Option<Filter>,
    filter_state: FilterState,
    /// The effects the source is sent through.
    sends: Vec<EffectSend>,
    /// The speaker volumes used last time, which the next buffer fades
    /// from so that a moving sound does not click.
    last_gains: Option<[f32; 2]>,
}

/// A source's connection to an effect.
struct EffectSend {
    effect: String,
    /// A filter on what reaches the effect, separate from the source's own.
    filter: Option<Filter>,
    state: FilterState,
}

impl EffectSend {
    fn new(effect: String, filter: Option<Filter>) -> Self {
        Self {
            effect,
            filter,
            state: FilterState::default(),
        }
    }
}

/// An effect defined with `audio::set_effect`.
struct EffectSlot {
    effect: Effect,
    state: EffectState,
    /// What the sources sent to it in the current buffer.
    input: Vec<f32>,
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
            placement: Placement::default(),
            filter: None,
            filter_state: FilterState::default(),
            sends: Vec::new(),
            last_gains: None,
        }
    }

    /// What the listener hears of this voice right now. Only mono sounds
    /// are positioned, as in LÖVE; stereo ones are heard as they are.
    fn heard(&self, listener: &Listener) -> Heard {
        if self.data.channels() == 1 {
            self.placement.heard(listener, self.volume)
        } else {
            let gain = self
                .volume
                .clamp(self.placement.min_volume, self.placement.max_volume);
            Heard {
                gains: [gain, gain],
                pitch: 1.0,
            }
        }
    }

    /// Adds this voice's sound to `out`, at its own loudness, and advances
    /// it. `pitch` is a change in speed on top of the voice's own pitch.
    fn render(&mut self, out: &mut [f32], pitch: f32) {
        let step = f64::from(self.data.rate()) / f64::from(AUDIO_SAMPLE_RATE)
            * f64::from(self.pitch.max(0.0))
            * f64::from(pitch);

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
            output[0] += current[0] + (next[0] - current[0]) * blend;
            output[1] += current[1] + (next[1] - current[1]) * blend;
            self.position += step;
        }
    }
}

/// Scales interleaved stereo by speaker volumes that move from `from` to
/// `to` over the length of the buffer.
fn apply_gains(samples: &mut [f32], from: [f32; 2], to: [f32; 2]) {
    let frames = (samples.len() / 2).max(1) as f32;
    for (i, frame) in samples.chunks_exact_mut(2).enumerate() {
        let through = (i + 1) as f32 / frames;
        for channel in 0..2 {
            frame[channel] *= from[channel] + (to[channel] - from[channel]) * through;
        }
    }
}

fn add_into(out: &mut [f32], samples: &[f32]) {
    for (output, sample) in out.iter_mut().zip(samples) {
        *output += sample;
    }
}

pub(crate) struct Mixer {
    voices: HashMap<u64, Voice>,
    next_id: u64,
    master: f32,
    listener: Listener,
    effects: HashMap<String, EffectSlot>,
    /// Room for one voice's sound while it is placed and filtered, kept so
    /// the audio thread does not allocate on every buffer.
    voice_buffer: Vec<f32>,
    send_buffer: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            voices: HashMap::new(),
            next_id: 1,
            master: 1.0,
            listener: Listener::default(),
            effects: HashMap::new(),
            voice_buffer: Vec::new(),
            send_buffer: Vec::new(),
        }
    }
}

impl Mixer {
    /// Fills `out` with interleaved stereo at [`AUDIO_SAMPLE_RATE`].
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        self.voice_buffer.resize(out.len(), 0.0);
        self.send_buffer.resize(out.len(), 0.0);
        for slot in self.effects.values_mut() {
            slot.input.clear();
            slot.input.resize(out.len(), 0.0);
        }

        for voice in self.voices.values_mut().filter(|voice| voice.playing) {
            let heard = voice.heard(&self.listener);
            let buffer = &mut self.voice_buffer;
            buffer.fill(0.0);
            voice.render(buffer, heard.pitch);
            apply_gains(buffer, voice.last_gains.unwrap_or(heard.gains), heard.gains);
            voice.last_gains = Some(heard.gains);

            for send in &mut voice.sends {
                let Some(slot) = self.effects.get_mut(&send.effect) else {
                    continue;
                };
                self.send_buffer.copy_from_slice(buffer);
                if let Some(filter) = &send.filter {
                    send.state.process(filter, &mut self.send_buffer);
                }
                add_into(&mut slot.input, &self.send_buffer);
            }
            if let Some(filter) = &voice.filter {
                voice.filter_state.process(filter, buffer);
            }
            add_into(out, buffer);
        }
        // Effects run even with nothing sent to them, so that a reverb's
        // tail carries on after its sound has finished.
        for slot in self.effects.values_mut() {
            slot.state.process(&slot.input, out);
        }

        self.voices
            .retain(|_, voice| voice.playing || !voice.orphaned);
        for sample in out {
            // Loud sounds added together can exceed what the device accepts.
            *sample = (*sample * self.master).clamp(-1.0, 1.0);
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

    /// Plays a copy of a sound from its start, at `volume` and `pitch`
    /// times the original's, and forgets the copy when it ends.
    pub fn play_once(&mut self, id: u64, volume: f32, pitch: f32) -> Result<(), String> {
        let (base_volume, base_pitch) = (self.volume(id), self.pitch(id));
        let copy = self.duplicate(id)?;
        self.set_volume(copy, base_volume * volume);
        self.set_pitch(copy, base_pitch * pitch);
        // A copy that looped would never end, with nothing left to stop it.
        self.set_looping(copy, false);
        self.play(copy);
        self.release(copy);
        Ok(())
    }

    /// Creates an independent voice that plays the same sound with the
    /// same settings, stopped at the beginning.
    pub fn duplicate(&mut self, id: u64) -> Result<u64, String> {
        let original = self.voices.get(&id).ok_or("the source no longer exists")?;
        let data = match &original.data {
            Data::Static(sound) => Data::Static(sound.clone()),
            Data::Stream(stream) => Data::Stream(Stream::new(stream.bytes().clone())?),
            // A copy of a queue starts with nothing queued.
            Data::Queue(queue) => {
                Data::Queue(Queue::new(queue.rate, queue.channels, queue.capacity))
            }
        };
        let copy = Voice {
            looping: original.looping,
            volume: original.volume,
            pitch: original.pitch,
            group: original.group,
            placement: original.placement,
            filter: original.filter,
            sends: original
                .sends
                .iter()
                .map(|send| EffectSend::new(send.effect.clone(), send.filter))
                .collect(),
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

    /// Adds a buffer to the end of a queueable source. Returns `Ok(false)`
    /// if it already has as many buffers waiting as it can hold.
    pub fn queue(
        &mut self,
        id: u64,
        frames: Vec<Frame>,
        rate: u32,
        channels: u16,
    ) -> Result<bool, String> {
        let Some(Data::Queue(queue)) = self.voices.get_mut(&id).map(|voice| &mut voice.data) else {
            return Err("only a queueable source can have sound queued".to_owned());
        };
        if rate != queue.rate || channels != queue.channels {
            return Err(format!(
                "queued sound is {rate} Hz with {channels} channel(s), but the source plays {} Hz with {} channel(s)",
                queue.rate, queue.channels
            ));
        }
        if queue.buffers.len() >= queue.capacity {
            return Ok(false);
        }
        if !frames.is_empty() {
            queue.buffers.push_back(frames);
        }
        Ok(true)
    }

    /// How many more buffers a queueable source can take right now.
    pub fn free_buffers(&self, id: u64) -> usize {
        match self.voices.get(&id).map(|voice| &voice.data) {
            Some(Data::Queue(queue)) => queue.capacity.saturating_sub(queue.buffers.len()),
            _ => 0,
        }
    }

    pub fn play(&mut self, id: u64) {
        if let Some(voice) = self.voices.get_mut(&id) {
            if !voice.playing {
                // Start at the right volume rather than fading from the
                // last time the sound played.
                voice.last_gains = None;
            }
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
            // A queue has no end to go back to the start from.
            voice.looping = looping && !matches!(voice.data, Data::Queue(_));
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
            if matches!(voice.data, Data::Queue(_)) {
                return;
            }
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

    /// 1 for a mono sound, which can be positioned, or 2 for stereo.
    pub fn channels(&self, id: u64) -> u16 {
        self.voices
            .get(&id)
            .map_or(2, |voice| voice.data.channels())
    }

    pub fn placement(&self, id: u64) -> Placement {
        self.voices
            .get(&id)
            .map_or_else(Placement::default, |voice| voice.placement)
    }

    pub fn change_placement(&mut self, id: u64, change: impl FnOnce(&mut Placement)) {
        if let Some(voice) = self.voices.get_mut(&id) {
            change(&mut voice.placement);
        }
    }

    pub fn set_filter(&mut self, id: u64, filter: Option<Filter>) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.filter = filter.map(Filter::clamped);
        }
    }

    pub fn filter(&self, id: u64) -> Option<Filter> {
        self.voices.get(&id).and_then(|voice| voice.filter)
    }

    /// Sends a voice through an effect, or changes the filter on an
    /// existing send. Returns `false` if there is no such effect.
    pub fn set_send(&mut self, id: u64, effect: &str, filter: Option<Filter>) -> bool {
        if !self.effects.contains_key(effect) {
            return false;
        }
        let Some(voice) = self.voices.get_mut(&id) else {
            return false;
        };
        let filter = filter.map(Filter::clamped);
        match voice.sends.iter_mut().find(|send| send.effect == effect) {
            Some(send) => send.filter = filter,
            None => voice.sends.push(EffectSend::new(effect.to_owned(), filter)),
        }
        true
    }

    pub fn remove_send(&mut self, id: u64, effect: &str) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.sends.retain(|send| send.effect != effect);
        }
    }

    pub fn sends(&self, id: u64) -> Vec<String> {
        self.voices.get(&id).map_or_else(Vec::new, |voice| {
            voice.sends.iter().map(|send| send.effect.clone()).collect()
        })
    }

    pub fn listener(&self) -> Listener {
        self.listener
    }

    pub fn change_listener(&mut self, change: impl FnOnce(&mut Listener)) {
        change(&mut self.listener);
    }

    /// Defines an effect, or changes the settings of one already defined.
    pub fn set_effect(&mut self, name: &str, effect: Effect) {
        match self.effects.get_mut(name) {
            Some(slot) => {
                slot.state.configure(&effect);
                slot.effect = effect;
            }
            None => {
                let slot = EffectSlot {
                    state: EffectState::new(&effect),
                    effect,
                    input: Vec::new(),
                };
                self.effects.insert(name.to_owned(), slot);
            }
        }
    }

    pub fn effect(&self, name: &str) -> Option<Effect> {
        self.effects.get(name).map(|slot| slot.effect)
    }

    /// Removes an effect, and every source's send to it.
    pub fn remove_effect(&mut self, name: &str) -> bool {
        for voice in self.voices.values_mut() {
            voice.sends.retain(|send| send.effect != name);
        }
        self.effects.remove(name).is_some()
    }

    pub fn effect_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.effects.keys().cloned().collect();
        names.sort();
        names
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
        Data::Static(Arc::new(Decoded {
            frames: vec![[level, level]; frames],
            rate: RATE,
            channels: 2,
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
    fn a_sound_played_once_overlaps_itself_and_cleans_up() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(3, 0.5)));
        mixer.set_volume(id, 0.5);
        mixer.play_once(id, 1.0, 1.0).unwrap();
        mixer.play_once(id, 0.5, 1.0).unwrap();

        // The original is untouched and the copies sound together.
        assert!(!mixer.is_playing(id));
        assert_eq!(mixer.playing_count(), 2);
        assert_eq!(left(&mut mixer, 4), [0.375, 0.375, 0.375, 0.0]);
        // Once they have ended, only the original is left.
        assert_eq!(mixer.playing_count(), 0);
        assert_eq!(mixer.voices.len(), 1);
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
        let id = mixer.add(Voice::new(Data::Static(Arc::new(Decoded {
            frames: vec![[0.1; 2], [0.2; 2], [0.3; 2]],
            rate: RATE,
            channels: 2,
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
        let id = mixer.add(Voice::new(Data::Static(Arc::new(Decoded {
            frames: vec![[0.0; 2], [1.0; 2]],
            rate: RATE / 2,
            channels: 2,
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

    /// A mono sound at the output rate whose every sample is `level`.
    fn mono(frames: usize, level: f32) -> Data {
        Data::Static(Arc::new(Decoded {
            frames: vec![[level, level]; frames],
            rate: RATE,
            channels: 1,
        }))
    }

    /// Mixes `frames` frames and returns both channels of the last one.
    fn last_frame(mixer: &mut Mixer, frames: usize) -> [f32; 2] {
        let mut out = vec![0.0; frames * 2];
        mixer.mix(&mut out);
        [out[out.len() - 2], out[out.len() - 1]]
    }

    #[test]
    fn a_mono_sound_to_the_right_plays_from_the_right_speaker() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(mono(1_000, 0.5)));
        mixer.change_placement(id, |placement| placement.position = [0.5, 0.0, 0.0]);
        mixer.play(id);
        let [left, right] = last_frame(&mut mixer, 100);
        assert!(left.abs() < 1e-4);
        assert!((right - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_far_away_sound_is_quieter_and_follows_the_listener() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(mono(1_000, 0.5)));
        mixer.change_placement(id, |placement| placement.position = [0.0, 0.0, -2.0]);
        mixer.play(id);
        let [left, _] = last_frame(&mut mixer, 100);
        assert!((left - 0.25).abs() < 1e-4);

        // Moving the listener onto the sound brings it back to full volume,
        // fading over the next buffer rather than jumping.
        mixer.change_listener(|listener| listener.position = [0.0, 0.0, -2.0]);
        let mut out = vec![0.0; 200];
        mixer.mix(&mut out);
        assert!(out[0] > 0.25 && out[0] < 0.5);
        assert!((out[198] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_stereo_sound_ignores_its_position() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(1_000, 0.5)));
        mixer.change_placement(id, |placement| placement.position = [100.0, 0.0, 0.0]);
        mixer.play(id);
        assert_eq!(last_frame(&mut mixer, 10), [0.5, 0.5]);
        assert_eq!(mixer.channels(id), 2);
    }

    #[test]
    fn a_filter_changes_only_what_goes_to_the_speakers() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(constant(1_000, 0.5)));
        mixer.set_filter(
            id,
            Some(Filter {
                volume: 0.5,
                ..Filter::low_pass(1.0)
            }),
        );
        mixer.play(id);
        let [left, right] = last_frame(&mut mixer, 10);
        assert!((left - 0.25).abs() < 1e-5 && (right - 0.25).abs() < 1e-5);

        mixer.set_filter(id, None);
        assert_eq!(mixer.filter(id), None);
        assert_eq!(last_frame(&mut mixer, 10), [0.5, 0.5]);
    }

    #[test]
    fn a_source_sent_to_an_echo_is_heard_again_after_it_stops() {
        use super::super::effects::Echo;
        let mut mixer = Mixer::default();
        // A send to an effect that does not exist yet is refused.
        let id = mixer.add(Voice::new(constant(10, 0.5)));
        assert!(!mixer.set_send(id, "echo", None));

        let echo = Echo {
            delay: 0.01,
            tap_delay: 0.0,
            feedback: 0.0,
            spread: 0.0,
            ..Echo::default()
        };
        mixer.set_effect("echo", echo.into());
        assert!(mixer.set_send(id, "echo", None));
        assert_eq!(mixer.sends(id), ["echo"]);
        mixer.play(id);

        let heard = left(&mut mixer, 1_000);
        assert_eq!(heard[0], 0.5);
        assert!(heard[10..480].iter().all(|&sample| sample == 0.0));
        // The repeat arrives 10 ms (480 frames) later. With no time between
        // the echo's two taps, both land at once, each at half volume.
        assert!((heard[480] - 0.5).abs() < 1e-5);
        assert_eq!(heard[490], 0.0);

        assert!(mixer.remove_effect("echo"));
        assert!(mixer.sends(id).is_empty());
        assert!(mixer.effect_names().is_empty());
    }

    #[test]
    fn a_duplicate_keeps_the_position_filter_and_effects() {
        use super::super::effects::Reverb;
        let mut mixer = Mixer::default();
        mixer.set_effect("room", Reverb::default().into());
        let id = mixer.add(Voice::new(mono(10, 0.5)));
        mixer.change_placement(id, |placement| placement.position = [3.0, 0.0, 0.0]);
        mixer.set_filter(id, Some(Filter::low_pass(0.5)));
        mixer.set_send(id, "room", None);

        let copy = mixer.duplicate(id).unwrap();
        assert_eq!(mixer.placement(copy).position, [3.0, 0.0, 0.0]);
        assert_eq!(mixer.filter(copy), Some(Filter::low_pass(0.5)));
        assert_eq!(mixer.sends(copy), ["room"]);
    }

    #[test]
    fn a_moving_sound_changes_pitch() {
        let mut mixer = Mixer::default();
        let id = mixer.add(Voice::new(mono(RATE as usize, 0.5)));
        mixer.change_placement(id, |placement| {
            placement.position = [0.0, 0.0, -10.0];
            // Coming closer at a tenth of the speed of sound.
            placement.velocity = [0.0, 0.0, 34.33];
        });
        mixer.play(id);
        left(&mut mixer, 900);
        // 900 frames at a pitch of 1 / 0.9 cover 1000 frames of the sound.
        assert!((mixer.tell(id) * f64::from(RATE) - 1_000.0).abs() < 1.0);
    }

    fn queue_voice(mixer: &mut Mixer, capacity: usize) -> u64 {
        mixer.add(Voice::new(Data::Queue(Queue::new(RATE, 2, capacity))))
    }

    #[test]
    fn a_queue_plays_its_buffers_in_order_and_frees_them() {
        let mut mixer = Mixer::default();
        let id = queue_voice(&mut mixer, 2);
        assert_eq!(mixer.free_buffers(id), 2);
        assert_eq!(mixer.queue(id, vec![[0.25; 2]; 10], RATE, 2), Ok(true));
        assert_eq!(mixer.queue(id, vec![[0.5; 2]; 10], RATE, 2), Ok(true));
        // Full until the audio thread has played through a buffer.
        assert_eq!(mixer.queue(id, vec![[0.75; 2]; 10], RATE, 2), Ok(false));
        assert_eq!(mixer.free_buffers(id), 0);

        mixer.play(id);
        let out = left(&mut mixer, 15);
        assert_eq!(out[..10], [0.25; 10]);
        assert_eq!(out[10..], [0.5; 5]);
        assert_eq!(mixer.free_buffers(id), 1);
        assert!(mixer.is_playing(id));
    }

    #[test]
    fn a_queue_that_runs_dry_stops_and_plays_again_once_fed() {
        let mut mixer = Mixer::default();
        let id = queue_voice(&mut mixer, 4);
        mixer.queue(id, vec![[0.5; 2]; 10], RATE, 2).unwrap();
        mixer.play(id);
        let out = left(&mut mixer, 20);
        assert_eq!(out[..10], [0.5; 10]);
        assert_eq!(out[10..], [0.0; 10]);
        assert!(!mixer.is_playing(id));
        assert_eq!(mixer.free_buffers(id), 4);

        mixer.queue(id, vec![[0.25; 2]; 10], RATE, 2).unwrap();
        mixer.play(id);
        assert_eq!(left(&mut mixer, 5), [0.25; 5]);
    }

    #[test]
    fn a_queue_crosses_buffers_smoothly_at_a_low_pitch() {
        let mut mixer = Mixer::default();
        let id = queue_voice(&mut mixer, 8);
        for _ in 0..4 {
            mixer.queue(id, vec![[0.5; 2]; 3], RATE, 2).unwrap();
        }
        mixer.set_pitch(id, 0.5);
        mixer.play(id);
        // Twelve frames at half speed last about twenty-four.
        let out = left(&mut mixer, 22);
        assert!(out.iter().all(|&sample| sample == 0.5), "{out:?}");
        assert!(mixer.is_playing(id));
    }

    #[test]
    fn stopping_a_queue_empties_it_and_it_never_loops() {
        let mut mixer = Mixer::default();
        let id = queue_voice(&mut mixer, 2);
        mixer.queue(id, vec![[0.5; 2]; 10], RATE, 2).unwrap();
        mixer.set_looping(id, true);
        assert!(!mixer.is_looping(id));
        mixer.play(id);
        left(&mut mixer, 2);
        mixer.stop(id);
        assert_eq!(mixer.free_buffers(id), 2);
        mixer.play(id);
        assert_eq!(left(&mut mixer, 3), [0.0; 3]);
    }

    #[test]
    fn a_queue_refuses_sound_of_another_format() {
        let mut mixer = Mixer::default();
        let id = queue_voice(&mut mixer, 2);
        assert!(mixer.queue(id, vec![[0.5; 2]; 10], 22_050, 2).is_err());
        assert!(mixer.queue(id, vec![[0.5; 2]; 10], RATE, 1).is_err());
        let file = mixer.add(Voice::new(constant(10, 0.5)));
        assert!(mixer.queue(file, vec![[0.5; 2]; 10], RATE, 2).is_err());
        assert_eq!(mixer.free_buffers(file), 0);
    }

    #[test]
    fn a_copy_of_a_queue_starts_empty() {
        let mut mixer = Mixer::default();
        let id = queue_voice(&mut mixer, 3);
        mixer.queue(id, vec![[0.5; 2]; 10], RATE, 2).unwrap();
        let copy = mixer.duplicate(id).unwrap();
        assert_eq!(mixer.free_buffers(copy), 3);
        assert_eq!(mixer.channels(copy), 2);
        assert_eq!(mixer.duration(copy), None);
    }
}

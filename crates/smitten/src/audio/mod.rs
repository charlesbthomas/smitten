//! Sound, the equivalent of `love.audio`.
//!
//! Load a sound with [`Source::load`] and call [`Source::play`]. Short sound
//! effects should be [`SourceType::Static`]; music and other long sounds
//! should be [`SourceType::Stream`].
//!
//! WAV, Ogg Vorbis, MP3 and FLAC files are supported.
//!
//! # Positional sound
//!
//! A mono source can be placed with [`Source::set_position`] and heard by
//! a listener placed with [`set_position`]. It pans toward the side it is
//! on, gets quieter with distance as [`DistanceModel`] describes, and
//! changes pitch when it moves if it has a velocity. Stereo sources are
//! not positioned, as in LÖVE, so music is unaffected by where the
//! listener is.
//!
//! Distances are in whatever unit the game likes. With pixels, give
//! sources a larger reference distance with
//! [`Source::set_attenuation_distances`], since the default of 1.0 makes a
//! sound half as loud two pixels away.
//!
//! A sound level with the listener in a 2D game is panned fully to one
//! side. To soften that, place the listener a little in front of the
//! screen, such as `audio::set_position(x, y, 100.0)`.
//!
//! # Filters and effects
//!
//! [`Source::set_filter`] muffles or thins a source. [`set_effect`] defines
//! a named effect, such as a [`Reverb`] or an [`Echo`], and
//! [`Source::set_effect`] sends a source through it.
//!
//! # Making and recording sound
//!
//! [`QueueableSource`] plays sound the game makes as it goes, such as
//! a synthesizer, a voice chat or a [`Decoder`](crate::sound::Decoder)'s
//! pieces changed on the way. [`recording_devices`] lists the microphones,
//! whose [`RecordingDevice`] hands back what it hears as
//! [`SoundData`](crate::sound::SoundData).

pub(crate) mod decode;
mod effects;
mod mixer;
mod record;
mod spatial;

use glam::Vec3;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::context;
use decode::Decoder;
use mixer::{Data, Mixer, Queue, Stream, Voice};

pub use effects::{
    Chorus, Compressor, Distortion, Echo, Effect, Equalizer, Filter, FilterType, Flanger, Reverb,
    RingModulator, Waveform,
};
pub use record::{RecordingDevice, recording_devices};
pub use spatial::DistanceModel;

/// How a [`Source`] is sent through an effect, from [`Source::effect`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectSend {
    /// All of the sound reaches the effect.
    Unfiltered,
    /// What reaches the effect goes through this filter first.
    Filtered(Filter),
}

/// How a sound is held in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceType {
    /// Decoded completely when loaded. Starts instantly and is cheap to
    /// play many times, but a long sound uses a lot of memory.
    Static,
    /// Decoded as it plays. Uses little memory however long the sound is.
    Stream,
    /// Plays buffers of samples the game hands it, for sound made as the
    /// game runs: a [`QueueableSource`].
    Queue,
}

/// The mixer shared by the game and the audio thread.
#[derive(Clone, Default)]
pub(crate) struct AudioState {
    mixer: Arc<Mutex<Mixer>>,
    /// What each recording device has captured, shared by every handle
    /// to it.
    recordings: std::collections::HashMap<
        smitten_core::backend::RecordingDeviceId,
        Arc<Mutex<record::Capture>>,
    >,
    /// Whether the output device has been asked for yet.
    started: bool,
}

/// Locks the mixer. If a thread panicked while holding the lock, the mixer
/// is still usable, so carry on rather than panicking again.
fn lock(mixer: &Mutex<Mixer>) -> MutexGuard<'_, Mixer> {
    mixer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the shared mixer, opening the audio device on first use so that
/// games without sound never touch it.
fn shared_mixer() -> Arc<Mutex<Mixer>> {
    context::with(|ctx| {
        if !ctx.audio.started {
            ctx.audio.started = true;
            let mixer = ctx.audio.mixer.clone();
            let fill = Box::new(move |out: &mut [f32]| lock(&mixer).mix(out));
            if let Err(message) = ctx.backend.start_audio(fill) {
                // The game still runs; it is just silent.
                eprintln!("smitten: audio is disabled: {message}");
            }
        }
        ctx.audio.mixer.clone()
    })
}

/// A sound that can be played, the equivalent of LÖVE's `Source`.
///
/// One source plays one copy of its sound at a time. To overlap the same
/// sound with itself, [`Clone`] the source: the copy shares the decoded
/// audio but plays on its own.
///
/// A source that is dropped while playing finishes its sound first, unless
/// it is looping, in which case it stops.
pub struct Source {
    mixer: Arc<Mutex<Mixer>>,
    id: u64,
    kind: SourceType,
}

/// Two sources are equal when they are the same source. A clone plays on
/// its own, so it is a different source.
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mixer, &other.mixer) && self.id == other.id
    }
}

impl Eq for Source {}

impl std::hash::Hash for Source {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Source {
    /// Creates a source from the contents of an audio file.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>, kind: SourceType) -> Result<Source, AudioError> {
        let bytes = bytes.into();
        let data = match kind {
            SourceType::Static => {
                let decoder = Decoder::open(bytes).map_err(AudioError::Decode)?;
                Data::Static(Arc::new(decoder.decode_all()))
            }
            SourceType::Stream => Data::Stream(Stream::new(bytes).map_err(AudioError::Decode)?),
            SourceType::Queue => {
                return Err(AudioError::Decode(
                    "a queueable source is made with QueueableSource::new, not from a file"
                        .to_owned(),
                ));
            }
        };
        let mixer = shared_mixer();
        let id = lock(&mixer).add(Voice::new(data));
        Ok(Source { mixer, id, kind })
    }

    /// Creates a source that plays samples from a [`SoundData`]. The
    /// samples are copied, so changing the sound data afterwards does not
    /// change the source.
    ///
    /// [`SoundData`]: crate::sound::SoundData
    pub fn from_sound_data(data: &crate::sound::SoundData) -> Source {
        let data = Data::Static(Arc::new(data.to_decoded()));
        let mixer = shared_mixer();
        let id = lock(&mixer).add(Voice::new(data));
        Source {
            mixer,
            id,
            kind: SourceType::Static,
        }
    }

    /// Creates a source that plays the file a [`Decoder`] reads, the
    /// equivalent of `love.audio.newSource(decoder, type)`. The source
    /// starts from the beginning of the file, wherever the decoder is.
    ///
    /// [`Decoder`]: crate::sound::Decoder
    #[doc(alias = "love.audio.newSource")]
    pub fn from_decoder(
        decoder: &crate::sound::Decoder,
        kind: SourceType,
    ) -> Result<Source, AudioError> {
        Source::from_bytes(decoder.bytes().clone(), kind)
    }

    fn mixer(&self) -> MutexGuard<'_, Mixer> {
        lock(&self.mixer)
    }

    fn placement(&self) -> spatial::Placement {
        self.mixer().placement(self.id)
    }

    fn change_placement(&self, change: impl FnOnce(&mut spatial::Placement)) {
        self.mixer().change_placement(self.id, change);
    }

    /// Whether the sound is held in memory or decoded as it plays.
    pub fn source_type(&self) -> SourceType {
        self.kind
    }

    /// Starts or resumes playback. Does nothing if already playing.
    pub fn play(&self) {
        self.mixer().play(self.id);
    }

    /// Plays the sound from its start without disturbing this source, and
    /// lets it overlap any other playing of it. The way to fire off a
    /// sound effect: a jump, a hit, a coin.
    ///
    /// The copy has this source's settings, cannot be stopped or changed
    /// once started, and cleans up after itself when it ends. It never
    /// loops, even if this source does.
    pub fn play_once(&self) {
        self.play_once_with(1.0, 1.0);
    }

    /// Like [`Source::play_once`], at `volume` and `pitch` times this
    /// source's own. Varying the pitch a little each time keeps a sound
    /// that repeats from sounding mechanical.
    pub fn play_once_with(&self, volume: f32, pitch: f32) {
        self.mixer()
            .play_once(self.id, volume, pitch)
            .expect("a sound that loaded once can be opened again");
    }

    /// Stops playback, keeping the position so `play` resumes from it.
    pub fn pause(&self) {
        self.mixer().pause(self.id);
    }

    /// Stops playback and rewinds to the start.
    pub fn stop(&self) {
        self.mixer().stop(self.id);
    }

    /// Returns whether the sound is currently playing.
    pub fn is_playing(&self) -> bool {
        self.mixer().is_playing(self.id)
    }

    /// Sets whether the sound starts over when it reaches the end. A
    /// queueable source never loops.
    pub fn set_looping(&self, looping: bool) {
        self.mixer().set_looping(self.id, looping);
    }

    /// Returns whether the sound loops.
    pub fn is_looping(&self) -> bool {
        self.mixer().is_looping(self.id)
    }

    /// Sets the volume, where 1.0 is the sound's own loudness and 0.0 is
    /// silent.
    pub fn set_volume(&self, volume: f32) {
        self.mixer().set_volume(self.id, volume);
    }

    /// Returns the volume.
    pub fn volume(&self) -> f32 {
        self.mixer().volume(self.id)
    }

    /// Sets the pitch, where 1.0 is normal, 2.0 is an octave up and twice
    /// as fast, and 0.5 is an octave down and half as fast.
    pub fn set_pitch(&self, pitch: f32) {
        self.mixer().set_pitch(self.id, pitch);
    }

    /// Returns the pitch.
    pub fn pitch(&self) -> f32 {
        self.mixer().pitch(self.id)
    }

    /// Moves playback to `position` from the start. A queueable source
    /// cannot seek.
    pub fn seek(&self, position: Duration) {
        self.mixer().seek(self.id, position.as_secs_f64());
    }

    /// Returns the playback position from the start.
    pub fn tell(&self) -> Duration {
        Duration::from_secs_f64(self.mixer().tell(self.id).max(0.0))
    }

    /// Moves playback to `sample`, counted in sample frames of the sound
    /// at its own rate, as LÖVE's `Source:seek(sample, "samples")`. A
    /// queueable source cannot seek.
    #[doc(alias = "Source:seek")]
    pub fn seek_samples(&self, sample: u64) {
        self.mixer().seek_frame(self.id, sample);
    }

    /// Returns the playback position in sample frames of the sound at its
    /// own rate, as LÖVE's `Source:tell("samples")`.
    #[doc(alias = "Source:tell")]
    pub fn tell_samples(&self) -> u64 {
        self.mixer().tell_frame(self.id)
    }

    /// Returns the sound's length in sample frames, as LÖVE's
    /// `Source:getDuration("samples")`. `None` when
    /// [`duration`](Self::duration) is.
    #[doc(alias = "Source:getDuration")]
    pub fn sample_count(&self) -> Option<u64> {
        self.mixer().frame_count(self.id)
    }

    /// Returns the sound's length. A few streamed formats only learn
    /// their length by playing to the end, and give `None` until then.
    pub fn duration(&self) -> Option<Duration> {
        self.mixer()
            .duration(self.id)
            .map(|seconds| Duration::from_secs_f64(seconds.max(0.0)))
    }

    /// Returns 1 for a mono sound or 2 for stereo. Only mono sounds can be
    /// positioned.
    pub fn channel_count(&self) -> u16 {
        self.mixer().channels(self.id)
    }

    /// Places the sound. It starts at the origin, which is also where the
    /// listener starts, so a sound that is never placed is heard as it is.
    ///
    /// Only mono sounds are positioned; stereo ones ignore this.
    pub fn set_position(&self, position: impl Into<Vec3>) {
        let Vec3 { x, y, z } = position.into();
        self.change_placement(|placement| placement.position = [x, y, z]);
    }

    /// Returns where the sound is.
    pub fn position(&self) -> Vec3 {
        let [x, y, z] = self.placement().position;
        (x, y, z).into()
    }

    /// Sets how fast the sound is moving, in distance units per second.
    /// This does not move it; it only shifts its pitch, as a passing car
    /// sounds higher coming and lower going.
    pub fn set_velocity(&self, velocity: impl Into<Vec3>) {
        let Vec3 { x, y, z } = velocity.into();
        self.change_placement(|placement| placement.velocity = [x, y, z]);
    }

    /// Returns the velocity.
    pub fn velocity(&self) -> Vec3 {
        let [x, y, z] = self.placement().velocity;
        (x, y, z).into()
    }

    /// Points the sound in a direction, for use with [`Source::set_cone`].
    /// A direction of zero, the default, sounds the same all round.
    pub fn set_direction(&self, direction: impl Into<Vec3>) {
        let Vec3 { x, y, z } = direction.into();
        self.change_placement(|placement| placement.direction = [x, y, z]);
    }

    /// Returns the direction.
    pub fn direction(&self) -> Vec3 {
        let [x, y, z] = self.placement().direction;
        (x, y, z).into()
    }

    /// Makes a directed sound quieter off to the side. Inside a cone
    /// `inner` radians wide it is at full volume; outside one `outer`
    /// radians wide it is at `outer_volume`; between the two it fades.
    pub fn set_cone(&self, inner: f32, outer: f32, outer_volume: f32) {
        self.change_placement(|placement| {
            placement.cone_inner = inner;
            placement.cone_outer = outer;
            placement.cone_outer_volume = outer_volume.clamp(0.0, 1.0);
        });
    }

    /// Returns the inner angle, outer angle and outer volume of the cone.
    pub fn cone(&self) -> (f32, f32, f32) {
        let placement = self.placement();
        (
            placement.cone_inner,
            placement.cone_outer,
            placement.cone_outer_volume,
        )
    }

    /// Sets whether the position is measured from the listener instead of
    /// from the origin, so the sound moves with the listener. With a
    /// relative position, positive x is always to the listener's right.
    pub fn set_relative(&self, relative: bool) {
        self.change_placement(|placement| placement.relative = relative);
    }

    /// Returns whether the position is measured from the listener.
    pub fn is_relative(&self) -> bool {
        self.placement().relative
    }

    /// Sets the distance at which the sound starts getting quieter and the
    /// distance beyond which it gets no quieter, as [`DistanceModel`]
    /// describes. They start at 1.0 and the largest `f32`.
    pub fn set_attenuation_distances(&self, reference: f32, max: f32) {
        self.change_placement(|placement| {
            placement.reference_distance = reference.max(0.0);
            placement.max_distance = max.max(0.0);
        });
    }

    /// Returns the reference and maximum distances.
    pub fn attenuation_distances(&self) -> (f32, f32) {
        let placement = self.placement();
        (placement.reference_distance, placement.max_distance)
    }

    /// Sets how quickly the sound gets quieter with distance, where 1.0 is
    /// normal and 0.0 means not at all.
    pub fn set_rolloff(&self, rolloff: f32) {
        self.change_placement(|placement| placement.rolloff = rolloff.max(0.0));
    }

    /// Returns the rolloff.
    pub fn rolloff(&self) -> f32 {
        self.placement().rolloff
    }

    /// Keeps the sound's final volume, after distance and the cone, between
    /// `min` and `max`. The minimum starts at 0.0. Unlike LÖVE, the maximum
    /// starts unlimited, so a volume above 1.0 still makes a sound louder.
    pub fn set_volume_limits(&self, min: f32, max: f32) {
        self.change_placement(|placement| {
            placement.min_volume = min.max(0.0);
            placement.max_volume = max.max(placement.min_volume);
        });
    }

    /// Returns the minimum and maximum volume.
    pub fn volume_limits(&self) -> (f32, f32) {
        let placement = self.placement();
        (placement.min_volume, placement.max_volume)
    }

    /// Makes the air muffle the sound the farther it is from the listener,
    /// as LÖVE's `Source:setAirAbsorption`. `amount` runs from 0.0, the
    /// default, which leaves the sound alone, to 10.0. At 1.0 the high
    /// frequencies lose about 0.05 dB per unit of distance, as in OpenAL,
    /// which is what real air does if a unit is a metre.
    ///
    /// Like the rest of positioning, it only affects mono sounds. It
    /// applies to what goes to effects as well as to the speakers.
    #[doc(alias = "Source:setAirAbsorption")]
    pub fn set_air_absorption(&self, amount: f32) {
        self.change_placement(|placement| placement.air_absorption = amount.clamp(0.0, 10.0));
    }

    /// Returns the air absorption.
    pub fn air_absorption(&self) -> f32 {
        self.placement().air_absorption
    }

    /// Filters the sound, or with `None` removes the filter. The filter
    /// affects what goes straight to the speakers, not what is sent to
    /// effects.
    pub fn set_filter(&self, filter: Option<Filter>) {
        self.mixer().set_filter(self.id, filter);
    }

    /// Returns the filter, if there is one.
    pub fn filter(&self) -> Option<Filter> {
        self.mixer().filter(self.id)
    }

    /// Sends the sound through the effect called `name`, defined with
    /// [`set_effect`]. `filter` filters what reaches the effect. Calling
    /// this again for the same effect changes the filter.
    ///
    /// Returns `false`, and does nothing, if there is no such effect.
    pub fn set_effect(&self, name: &str, filter: Option<Filter>) -> bool {
        self.mixer().set_send(self.id, name, filter)
    }

    /// Returns how the sound is sent through the effect called `name`, as
    /// LÖVE's `Source:getEffect`, or `None` if it is not sent there.
    #[doc(alias = "Source:getEffect")]
    pub fn effect(&self, name: &str) -> Option<EffectSend> {
        self.mixer()
            .send_filter(self.id, name)
            .map(|filter| match filter {
                Some(filter) => EffectSend::Filtered(filter),
                None => EffectSend::Unfiltered,
            })
    }

    /// Stops sending the sound through the effect called `name`.
    pub fn remove_effect(&self, name: &str) {
        self.mixer().remove_send(self.id, name);
    }

    /// Returns the names of the effects the sound is sent through.
    pub fn active_effects(&self) -> Vec<String> {
        self.mixer().sends(self.id)
    }
}

impl Clone for Source {
    /// Makes an independent source for the same sound, stopped at the
    /// start, with the same settings, such as volume, pitch, looping,
    /// position, filter and effects.
    fn clone(&self) -> Self {
        let id = self
            .mixer()
            .duplicate(self.id)
            .expect("a sound that loaded once can be opened again");
        Source {
            mixer: self.mixer.clone(),
            id,
            kind: self.kind,
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.mixer().release(self.id);
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("kind", &self.kind)
            .field("playing", &self.is_playing())
            .finish()
    }
}

/// Why a sound could not be loaded.
#[non_exhaustive]
#[derive(Debug)]
pub enum AudioError {
    /// The file could not be read.
    Read {
        /// The file.
        path: PathBuf,
        /// Why it could not be read.
        source: std::io::Error,
    },
    /// The data is not audio Smitten can decode.
    Decode(String),
    /// A recording device could not be opened.
    Device(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Read { path, source } => {
                write!(f, "could not read sound {}: {source}", path.display())
            }
            AudioError::Decode(message) => write!(f, "could not load sound: {message}"),
            AudioError::Device(message) => write!(f, "could not record sound: {message}"),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Read { source, .. } => Some(source),
            AudioError::Decode(_) | AudioError::Device(_) => None,
        }
    }
}

impl Source {
    /// Loads a sound file.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes, which
    /// normally means inside the game's `assets` folder.
    pub fn load(path: impl AsRef<Path>, kind: SourceType) -> Result<Source, AudioError> {
        let path = path.as_ref();
        let bytes = crate::filesystem::read(path).map_err(|source| AudioError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let source = Source::from_bytes(bytes, kind)?;
        let file = crate::filesystem::resolve(path);
        context::with(|ctx| {
            ctx.reload.watch(
                file,
                crate::reload::Asset::Sound(Sound {
                    mixer: source.mixer.clone(),
                    group: source.id,
                }),
            );
        });
        Ok(source)
    }
}

/// A source that plays sound the game makes as it runs, the equivalent of
/// a source from `love.audio.newQueueableSource`. Hand it buffers with
/// [`queue`](Self::queue) and call [`play`](Source::play).
///
/// It derefs to [`Source`] for playing, volume, position and the rest. It
/// cannot seek or loop.
///
/// ```no_run
/// use smitten::audio::QueueableSource;
/// use smitten::sound::SoundData;
///
/// let source = QueueableSource::new(44_100, 1, 4);
/// while source.free_buffer_count() > 0 {
///     let mut tone = SoundData::new(1_024, 44_100, 1);
///     for (i, sample) in tone.samples_mut().iter_mut().enumerate() {
///         *sample = (i as f32 * 0.06).sin() * 0.2;
///     }
///     source.queue(&tone);
/// }
/// source.play();
/// ```
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct QueueableSource {
    source: Source,
}

impl QueueableSource {
    /// Makes a queueable source. Every buffer must have `rate` samples per
    /// second and `channels` channels. At most `buffer_count` buffers wait
    /// at once; more than one lets the game queue the next before the
    /// current one runs out.
    ///
    /// Panics if `rate` is zero or `channels` is not 1 or 2.
    pub fn new(rate: u32, channels: u16, buffer_count: usize) -> QueueableSource {
        assert!(rate > 0, "a sound's sample rate must be above zero");
        assert!(
            channels == 1 || channels == 2,
            "a sound must have 1 or 2 channels, not {channels}"
        );
        let data = Data::Queue(Queue::new(rate, channels, buffer_count));
        let mixer = shared_mixer();
        let id = lock(&mixer).add(Voice::new(data));
        QueueableSource {
            source: Source {
                mixer,
                id,
                kind: SourceType::Queue,
            },
        }
    }

    /// Adds sound to the end of a queueable source, to play once what is
    /// already queued has played. The samples are copied.
    ///
    /// Returns `false`, and queues nothing, if the source already holds as
    /// many buffers as it was made with; wait for
    /// [`free_buffer_count`](Self::free_buffer_count) to rise. A source that ran out of
    /// sound has stopped, so call [`Source::play`] again after queueing.
    ///
    /// Panics if `data` does not have the source's sample rate and
    /// channel count.
    pub fn queue(&self, data: &crate::sound::SoundData) -> bool {
        let frames = data.to_decoded().frames;
        let queued = self
            .mixer()
            .queue(self.source.id, frames, data.rate(), data.channel_count());
        queued.unwrap_or_else(|message| panic!("{message}"))
    }

    /// How many more buffers a queueable source can take before
    /// [`queue`](Self::queue) refuses them.
    pub fn free_buffer_count(&self) -> usize {
        self.source.mixer().free_buffers(self.source.id)
    }
}

impl std::ops::Deref for QueueableSource {
    type Target = Source;

    fn deref(&self) -> &Source {
        &self.source
    }
}

/// A sound loaded from a file, with every copy made of it, for
/// [`crate::reload`].
#[derive(Clone)]
pub(crate) struct Sound {
    mixer: Arc<Mutex<Mixer>>,
    group: u64,
}

impl Sound {
    /// Whether the game still holds a source playing this sound.
    pub(crate) fn in_use(&self) -> bool {
        !lock(&self.mixer).group(self.group).is_empty()
    }

    /// Gives every source of this sound a newer version of its file. A
    /// source that is playing carries on from the start of the new sound.
    pub(crate) fn reload(&self, bytes: Vec<u8>) -> Result<(), String> {
        let bytes: Arc<[u8]> = bytes.into();
        // Decoding can take a while, so it is done without holding up the
        // audio thread, and nothing changes unless all of it succeeds.
        let voices = lock(&self.mixer).group(self.group);
        let mut decoded: Option<Arc<decode::Decoded>> = None;
        let mut replacements = Vec::with_capacity(voices.len());
        for (id, streamed) in voices {
            let data = if streamed {
                Data::Stream(Stream::new(bytes.clone())?)
            } else {
                let sound = match &decoded {
                    Some(sound) => sound.clone(),
                    None => {
                        let sound = Arc::new(Decoder::open(bytes.clone())?.decode_all());
                        decoded.insert(sound).clone()
                    }
                };
                Data::Static(sound)
            };
            replacements.push((id, data));
        }
        let mut mixer = lock(&self.mixer);
        for (id, data) in replacements {
            mixer.replace_data(id, data);
        }
        Ok(())
    }
}

/// Sets the volume of everything, where 1.0 is normal and 0.0 is silent.
pub fn set_volume(volume: f32) {
    lock(&shared_mixer()).set_master_volume(volume);
}

/// Returns the volume of everything.
pub fn volume() -> f32 {
    lock(&shared_mixer()).master_volume()
}

/// Starts or resumes every source in `sources` at the same moment, the
/// equivalent of `love.audio.play` with several sources. Calling
/// [`Source::play`] on each in turn could let the audio thread mix a
/// buffer between two of them, putting them a few milliseconds apart.
#[doc(alias = "love.audio.play")]
pub fn play(sources: &[&Source]) {
    let mixer = shared_mixer();
    let mut mixer = lock(&mixer);
    for source in sources {
        mixer.play(source.id);
    }
}

/// Pauses every playing source. Each resumes from where it was when its
/// `play` is called.
pub fn pause() {
    lock(&shared_mixer()).pause_all();
}

/// Stops and rewinds every source.
pub fn stop() {
    lock(&shared_mixer()).stop_all();
}

/// Returns how many sources are playing right now.
pub fn active_source_count() -> usize {
    lock(&shared_mixer()).playing_count()
}

fn change_listener(change: impl FnOnce(&mut spatial::Listener)) {
    lock(&shared_mixer()).change_listener(change);
}

fn listener() -> spatial::Listener {
    lock(&shared_mixer()).listener()
}

/// Places the listener, who hears every positioned source. The listener
/// starts at the origin.
pub fn set_position(position: impl Into<Vec3>) {
    let Vec3 { x, y, z } = position.into();
    change_listener(|listener| listener.position = [x, y, z]);
}

/// Returns where the listener is.
pub fn position() -> Vec3 {
    let [x, y, z] = listener().position;
    (x, y, z).into()
}

/// Sets how fast the listener is moving, which shifts the pitch of sounds
/// as [`Source::set_velocity`] describes. It does not move the listener.
pub fn set_velocity(velocity: impl Into<Vec3>) {
    let Vec3 { x, y, z } = velocity.into();
    change_listener(|listener| listener.velocity = [x, y, z]);
}

/// Returns the listener's velocity.
pub fn velocity() -> Vec3 {
    let [x, y, z] = listener().velocity;
    (x, y, z).into()
}

/// Turns the listener to face `forward`, with the top of their head
/// toward `up`. They start facing negative z with positive y up, which
/// puts positive x on their right.
pub fn set_orientation(forward: impl Into<Vec3>, up: impl Into<Vec3>) {
    let (forward, up) = (forward.into(), up.into());
    change_listener(|listener| {
        listener.forward = forward.to_array();
        listener.up = up.to_array();
    });
}

/// Returns the directions the listener faces and their head points.
pub fn orientation() -> (Vec3, Vec3) {
    let listener = listener();
    let [fx, fy, fz] = listener.forward;
    let [ux, uy, uz] = listener.up;
    ((fx, fy, fz).into(), (ux, uy, uz).into())
}

/// Chooses how sounds get quieter with distance.
pub fn set_distance_model(model: DistanceModel) {
    change_listener(|listener| listener.model = model);
}

/// Returns how sounds get quieter with distance.
pub fn distance_model() -> DistanceModel {
    listener().model
}

/// Scales how much moving sounds change pitch, where 1.0 is real life and
/// 0.0 turns the effect off. The speed of sound is taken to be 343.3
/// distance units per second, so a game measuring in pixels may want a
/// much larger value.
pub fn set_doppler_scale(scale: f32) {
    change_listener(|listener| listener.doppler_scale = scale.max(0.0));
}

/// Returns the Doppler scale.
pub fn doppler_scale() -> f32 {
    listener().doppler_scale
}

/// Defines an effect that sources can be sent through with
/// [`Source::set_effect`], or changes one already defined. Changing the
/// settings of an effect keeps it going, so a reverb's tail is not cut
/// off.
///
/// ```no_run
/// use smitten::audio::{self, Reverb};
///
/// audio::set_effect("cave", Reverb { decay_time: 4.0, ..Reverb::default() });
/// ```
pub fn set_effect(name: &str, effect: impl Into<Effect>) {
    lock(&shared_mixer()).set_effect(name, effect.into());
}

/// Returns the settings of the effect called `name`.
pub fn effect(name: &str) -> Option<Effect> {
    lock(&shared_mixer()).effect(name)
}

/// Removes the effect called `name`, and stops every source sending to it.
/// Returns `false` if there was no such effect.
pub fn remove_effect(name: &str) -> bool {
    lock(&shared_mixer()).remove_effect(name)
}

/// Whether effects work, the equivalent of `love.audio.isEffectsSupported`.
/// Smitten mixes effects itself, so they always do.
#[doc(alias = "love.audio.isEffectsSupported")]
pub fn is_effects_supported() -> bool {
    true
}

/// How many effects can be defined at once, the equivalent of
/// `love.audio.getMaxSceneEffects`. Smitten's own mixer has no limit
/// other than the time it takes to run them, so this is `usize::MAX`.
/// LÖVE's OpenAL usually allows a few dozen.
#[doc(alias = "love.audio.getMaxSceneEffects")]
pub fn max_scene_effects() -> usize {
    usize::MAX
}

/// How many effects one source can be sent through at once, the
/// equivalent of `love.audio.getMaxSourceEffects`. There is no limit, as
/// with [`max_scene_effects`]; OpenAL usually allows 2 to 4.
#[doc(alias = "love.audio.getMaxSourceEffects")]
pub fn max_source_effects() -> usize {
    usize::MAX
}

/// Asks to mix with music other apps are playing, the equivalent of
/// `love.audio.setMixWithSystem`. Only phones and tablets can choose, and
/// Smitten does not run on them yet, so this returns `false` and changes
/// nothing, as LÖVE does on desktop systems.
#[doc(alias = "love.audio.setMixWithSystem")]
pub fn set_mix_with_system(mix: bool) -> bool {
    let _ = mix;
    false
}

/// Returns the names of every effect defined, in alphabetical order.
pub fn active_effects() -> Vec<String> {
    lock(&shared_mixer()).effect_names()
}

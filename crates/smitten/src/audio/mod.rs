//! Sound, the equivalent of `love.audio`.
//!
//! Load a sound with [`new_source`] and call [`Source::play`]. Short sound
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

pub(crate) mod decode;
mod effects;
mod mixer;
mod spatial;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::context;
use decode::Decoder;
use mixer::{Data, Mixer, Stream, Voice};

pub use effects::{
    Chorus, Compressor, Distortion, Echo, Effect, Equalizer, Filter, FilterType, Flanger, Reverb,
    RingModulator, Waveform,
};
pub use spatial::DistanceModel;

/// How a sound is held in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceType {
    /// Decoded completely when loaded. Starts instantly and is cheap to
    /// play many times, but a long sound uses a lot of memory.
    Static,
    /// Decoded as it plays. Uses little memory however long the sound is.
    Stream,
}

/// The mixer shared by the game and the audio thread.
#[derive(Clone, Default)]
pub(crate) struct AudioState {
    mixer: Arc<Mutex<Mixer>>,
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

    /// Sets whether the sound starts over when it reaches the end.
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

    /// Moves playback to `seconds` from the start.
    pub fn seek(&self, seconds: f64) {
        self.mixer().seek(self.id, seconds);
    }

    /// Returns the playback position in seconds from the start.
    pub fn tell(&self) -> f64 {
        self.mixer().tell(self.id)
    }

    /// Returns the sound's length in seconds. A few streamed formats only
    /// learn their length by playing to the end, and give `None` until then.
    pub fn duration(&self) -> Option<f64> {
        self.mixer().duration(self.id)
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
    pub fn set_position(&self, x: f32, y: f32, z: f32) {
        self.change_placement(|placement| placement.position = [x, y, z]);
    }

    /// Returns where the sound is.
    pub fn position(&self) -> (f32, f32, f32) {
        let [x, y, z] = self.placement().position;
        (x, y, z)
    }

    /// Sets how fast the sound is moving, in distance units per second.
    /// This does not move it; it only shifts its pitch, as a passing car
    /// sounds higher coming and lower going.
    pub fn set_velocity(&self, x: f32, y: f32, z: f32) {
        self.change_placement(|placement| placement.velocity = [x, y, z]);
    }

    /// Returns the velocity.
    pub fn velocity(&self) -> (f32, f32, f32) {
        let [x, y, z] = self.placement().velocity;
        (x, y, z)
    }

    /// Points the sound in a direction, for use with [`Source::set_cone`].
    /// A direction of zero, the default, sounds the same all round.
    pub fn set_direction(&self, x: f32, y: f32, z: f32) {
        self.change_placement(|placement| placement.direction = [x, y, z]);
    }

    /// Returns the direction.
    pub fn direction(&self) -> (f32, f32, f32) {
        let [x, y, z] = self.placement().direction;
        (x, y, z)
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
#[derive(Debug)]
pub enum AudioError {
    /// The file could not be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The data is not audio Smitten can decode.
    Decode(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Read { path, source } => {
                write!(f, "could not read sound {}: {source}", path.display())
            }
            AudioError::Decode(message) => write!(f, "could not load sound: {message}"),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Read { source, .. } => Some(source),
            AudioError::Decode(_) => None,
        }
    }
}

/// Loads a sound file.
///
/// A relative path is looked up as [`crate::filesystem`] describes, which
/// normally means inside the game's `assets` folder.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_source`] to handle the failure instead.
pub fn new_source(path: impl AsRef<Path>, kind: SourceType) -> Source {
    let path = path.as_ref();
    try_new_source(path, kind).unwrap_or_else(|error| match error {
        AudioError::Decode(message) => {
            panic!("could not load sound {}: {message}", path.display())
        }
        error => panic!("{error}"),
    })
}

/// Loads a sound file, returning an error if it fails.
pub fn try_new_source(path: impl AsRef<Path>, kind: SourceType) -> Result<Source, AudioError> {
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
pub fn set_position(x: f32, y: f32, z: f32) {
    change_listener(|listener| listener.position = [x, y, z]);
}

/// Returns where the listener is.
pub fn position() -> (f32, f32, f32) {
    let [x, y, z] = listener().position;
    (x, y, z)
}

/// Sets how fast the listener is moving, which shifts the pitch of sounds
/// as [`Source::set_velocity`] describes. It does not move the listener.
pub fn set_velocity(x: f32, y: f32, z: f32) {
    change_listener(|listener| listener.velocity = [x, y, z]);
}

/// Returns the listener's velocity.
pub fn velocity() -> (f32, f32, f32) {
    let [x, y, z] = listener().velocity;
    (x, y, z)
}

/// Turns the listener to face `forward`, with the top of their head
/// toward `up`. They start facing negative z with positive y up, which
/// puts positive x on their right.
pub fn set_orientation(forward: (f32, f32, f32), up: (f32, f32, f32)) {
    change_listener(|listener| {
        listener.forward = [forward.0, forward.1, forward.2];
        listener.up = [up.0, up.1, up.2];
    });
}

/// Returns the directions the listener faces and their head points.
pub fn orientation() -> ((f32, f32, f32), (f32, f32, f32)) {
    let listener = listener();
    let [fx, fy, fz] = listener.forward;
    let [ux, uy, uz] = listener.up;
    ((fx, fy, fz), (ux, uy, uz))
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

/// Returns the names of every effect defined, in alphabetical order.
pub fn active_effects() -> Vec<String> {
    lock(&shared_mixer()).effect_names()
}

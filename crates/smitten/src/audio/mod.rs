//! Sound, the equivalent of `love.audio`.
//!
//! Load a sound with [`new_source`] and call [`Source::play`]. Short sound
//! effects should be [`SourceType::Static`]; music and other long sounds
//! should be [`SourceType::Stream`].
//!
//! WAV, Ogg Vorbis, MP3 and FLAC files are supported.

mod decode;
mod mixer;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::context;
use decode::Decoder;
use mixer::{Data, Mixer, Stream, Voice};

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

    fn mixer(&self) -> MutexGuard<'_, Mixer> {
        lock(&self.mixer)
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
}

impl Clone for Source {
    /// Makes an independent source for the same sound, stopped at the
    /// start, with the same volume, pitch and looping.
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
        let mut decoded: Option<Arc<decode::SoundData>> = None;
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

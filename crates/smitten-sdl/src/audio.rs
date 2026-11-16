//! Audio output through an SDL audio stream.
//!
//! SDL calls back on its own audio thread whenever the device needs more
//! samples. The callback asks Smitten's mixer to fill a buffer and hands
//! the result to SDL, which converts it to whatever the device plays.

use std::ffi::{c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use sdl3_sys::everything::*;
use smitten_core::backend::{AUDIO_SAMPLE_RATE, AudioFill};

use crate::sdl_error;

const CHANNELS: usize = 2;
const BYTES_PER_FRAME: usize = CHANNELS * size_of::<f32>();

/// What the audio thread owns: the mixer callback and a reusable buffer.
struct Shared {
    fill: AudioFill,
    scratch: Vec<f32>,
}

pub(crate) struct AudioOutput {
    stream: *mut SDL_AudioStream,
    /// Handed to SDL as the callback's userdata and reclaimed in `drop`.
    shared: *mut Shared,
}

impl AudioOutput {
    pub fn open(fill: AudioFill) -> Result<Self, String> {
        let shared = Box::into_raw(Box::new(Shared {
            fill,
            scratch: Vec::new(),
        }));
        // SAFETY: `shared` stays valid until `drop` destroys the stream,
        // after which SDL makes no further callbacks. The spec is valid for
        // the call. On failure the box is reclaimed before returning.
        unsafe {
            if !SDL_InitSubSystem(SDL_INIT_AUDIO) {
                let message = format!("could not start audio: {}", sdl_error());
                drop(Box::from_raw(shared));
                return Err(message);
            }
            let spec = SDL_AudioSpec {
                format: SDL_AUDIO_F32,
                channels: CHANNELS as c_int,
                freq: AUDIO_SAMPLE_RATE as c_int,
            };
            let stream = SDL_OpenAudioDeviceStream(
                SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &spec,
                Some(callback),
                shared.cast::<c_void>(),
            );
            if stream.is_null() {
                let message = format!("could not open an audio device: {}", sdl_error());
                drop(Box::from_raw(shared));
                SDL_QuitSubSystem(SDL_INIT_AUDIO);
                return Err(message);
            }
            // Streams opened this way start paused.
            SDL_ResumeAudioStreamDevice(stream);
            Ok(Self { stream, shared })
        }
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        // SAFETY: destroying the stream waits for any running callback and
        // prevents new ones, so nothing else can touch `shared` afterwards.
        unsafe {
            SDL_DestroyAudioStream(self.stream);
            drop(Box::from_raw(self.shared));
            SDL_QuitSubSystem(SDL_INIT_AUDIO);
        }
    }
}

/// Called by SDL on the audio thread when the stream wants more data.
unsafe extern "C" fn callback(
    userdata: *mut c_void,
    stream: *mut SDL_AudioStream,
    additional_amount: c_int,
    _total_amount: c_int,
) {
    let Ok(bytes) = usize::try_from(additional_amount) else {
        return;
    };
    let frames = bytes / BYTES_PER_FRAME;
    if frames == 0 {
        return;
    }
    // SAFETY: `userdata` is the `Shared` passed to SDL in `open`. SDL runs
    // one callback at a time per stream, so this is the only reference.
    let shared = unsafe { &mut *userdata.cast::<Shared>() };
    shared.scratch.clear();
    shared.scratch.resize(frames * CHANNELS, 0.0);

    // A panic must not unwind into SDL's C code. Play silence instead.
    let filled = catch_unwind(AssertUnwindSafe(|| (shared.fill)(&mut shared.scratch)));
    if filled.is_err() {
        shared.scratch.fill(0.0);
    }

    // SAFETY: the pointer and length describe `scratch`, which SDL copies
    // before returning.
    unsafe {
        SDL_PutAudioStreamData(
            stream,
            shared.scratch.as_ptr().cast::<c_void>(),
            (frames * BYTES_PER_FRAME) as c_int,
        );
    }
}

//! Audio output through an SDL audio stream.
//!
//! SDL calls back on its own audio thread whenever the device needs more
//! samples. The callback asks Smitten's mixer to fill a buffer and hands
//! the result to SDL, which converts it to whatever the device plays.
//!
//! Recording works the other way round: SDL calls back when a recording
//! device has captured more, and the samples are handed to a sink.

use std::ffi::{c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use sdl3_sys::everything::*;
use smitten_core::backend::{AUDIO_SAMPLE_RATE, AudioFill, RecordingDeviceId, RecordingSink};

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

/// Keeps the audio subsystem running while it is alive. Device ids are
/// only good while the subsystem stays up, so one is held from the first
/// time the devices are listed.
pub(crate) struct AudioSubsystem(());

impl AudioSubsystem {
    pub fn start() -> Result<Self, String> {
        // SAFETY: balanced by the `SDL_QuitSubSystem` in `drop`.
        if unsafe { SDL_InitSubSystem(SDL_INIT_AUDIO) } {
            Ok(Self(()))
        } else {
            Err(format!("could not start audio: {}", sdl_error()))
        }
    }
}

impl Drop for AudioSubsystem {
    fn drop(&mut self) {
        // SAFETY: the subsystem was started in `start`.
        unsafe { SDL_QuitSubSystem(SDL_INIT_AUDIO) };
    }
}

/// Lists the recording devices. The audio subsystem must be running, as
/// an [`AudioSubsystem`] keeps it.
pub(crate) fn recording_devices(_running: &AudioSubsystem) -> Vec<(RecordingDeviceId, String)> {
    // SAFETY: the list SDL returns holds `count` ids and is freed once.
    // Device names are copied while the subsystem is still running.
    unsafe {
        let mut count: c_int = 0;
        let ids = SDL_GetAudioRecordingDevices(&mut count);
        let mut devices = Vec::new();
        if !ids.is_null() {
            let ids_slice = std::slice::from_raw_parts(ids, usize::try_from(count).unwrap_or(0));
            for &id in ids_slice {
                let name = SDL_GetAudioDeviceName(id);
                let name = if name.is_null() {
                    String::from("Recording device")
                } else {
                    std::ffi::CStr::from_ptr(name)
                        .to_string_lossy()
                        .into_owned()
                };
                devices.push((RecordingDeviceId(id.0), name));
            }
            SDL_free(ids.cast());
        }
        devices
    }
}

/// What a recording's audio thread owns.
struct RecordingShared {
    sink: RecordingSink,
    scratch: Vec<f32>,
}

/// A recording device that is capturing. Dropping it stops the capture.
pub(crate) struct AudioInput {
    stream: *mut SDL_AudioStream,
    shared: *mut RecordingShared,
}

impl AudioInput {
    pub fn open(
        device: RecordingDeviceId,
        rate: u32,
        channels: u16,
        sink: RecordingSink,
    ) -> Result<Self, String> {
        if !(1..=2).contains(&channels) {
            return Err(format!("cannot record {channels} channels; use 1 or 2"));
        }
        let rate = c_int::try_from(rate)
            .ok()
            .filter(|&rate| rate > 0)
            .ok_or_else(|| format!("cannot record at {rate} samples a second"))?;
        let shared = Box::into_raw(Box::new(RecordingShared {
            sink,
            scratch: Vec::new(),
        }));
        // SAFETY: as for `AudioOutput::open`: `shared` outlives the stream,
        // and is reclaimed if opening fails.
        unsafe {
            if !SDL_InitSubSystem(SDL_INIT_AUDIO) {
                let message = format!("could not start audio: {}", sdl_error());
                drop(Box::from_raw(shared));
                return Err(message);
            }
            let spec = SDL_AudioSpec {
                format: SDL_AUDIO_F32,
                channels: c_int::from(channels),
                freq: rate,
            };
            let stream = SDL_OpenAudioDeviceStream(
                SDL_AudioDeviceID(device.0),
                &spec,
                Some(recording_callback),
                shared.cast::<c_void>(),
            );
            if stream.is_null() {
                let message = format!("could not open the recording device: {}", sdl_error());
                drop(Box::from_raw(shared));
                SDL_QuitSubSystem(SDL_INIT_AUDIO);
                return Err(message);
            }
            SDL_ResumeAudioStreamDevice(stream);
            Ok(Self { stream, shared })
        }
    }
}

impl Drop for AudioInput {
    fn drop(&mut self) {
        // SAFETY: as for `AudioOutput`, destroying the stream ends its
        // callbacks before `shared` is freed.
        unsafe {
            SDL_DestroyAudioStream(self.stream);
            drop(Box::from_raw(self.shared));
            SDL_QuitSubSystem(SDL_INIT_AUDIO);
        }
    }
}

/// Called by SDL on the audio thread when a recording has more to read.
unsafe extern "C" fn recording_callback(
    userdata: *mut c_void,
    stream: *mut SDL_AudioStream,
    additional_amount: c_int,
    _total_amount: c_int,
) {
    let Ok(bytes) = usize::try_from(additional_amount) else {
        return;
    };
    let samples = bytes / size_of::<f32>();
    if samples == 0 {
        return;
    }
    // SAFETY: `userdata` is the `RecordingShared` given to SDL in `open`,
    // and SDL runs one callback at a time per stream.
    let shared = unsafe { &mut *userdata.cast::<RecordingShared>() };
    shared.scratch.resize(samples, 0.0);
    // SAFETY: the buffer holds `samples` floats, which is what is asked for.
    let read = unsafe {
        SDL_GetAudioStreamData(
            stream,
            shared.scratch.as_mut_ptr().cast::<c_void>(),
            (samples * size_of::<f32>()) as c_int,
        )
    };
    let Ok(read) = usize::try_from(read) else {
        return;
    };
    let got = &shared.scratch[..read / size_of::<f32>()];
    if !got.is_empty() {
        // A panic must not unwind into SDL's C code; the samples are lost.
        let _ = catch_unwind(AssertUnwindSafe(|| (shared.sink)(got)));
    }
}

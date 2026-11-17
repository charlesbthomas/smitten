//! Recording from microphones, the equivalent of LÖVE's `RecordingDevice`.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use smitten_core::backend::RecordingDeviceId;

use super::AudioError;
use crate::context;
use crate::sound::SoundData;

/// What a device has captured and not yet been read, shared with the
/// audio thread that captures it.
#[derive(Debug)]
pub(crate) struct Capture {
    /// Interleaved for stereo.
    samples: VecDeque<f32>,
    /// The most samples per channel kept before the oldest are dropped.
    capacity: usize,
    rate: u32,
    channels: u16,
    recording: bool,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            capacity: 0,
            rate: 8_000,
            channels: 1,
            recording: false,
        }
    }
}

impl Capture {
    /// Gets ready for a new recording, forgetting anything not read.
    fn restart(&mut self, capacity: usize, rate: u32, channels: u16) {
        self.samples.clear();
        self.capacity = capacity;
        self.rate = rate;
        self.channels = channels;
        self.recording = true;
    }

    /// Adds what the device heard. Once the buffer is full, the oldest
    /// samples make way, so a reading is always of the latest sound.
    fn push(&mut self, samples: &[f32]) {
        if !self.recording {
            return;
        }
        self.samples.extend(samples);
        let limit = self.capacity * usize::from(self.channels);
        let excess = self.samples.len().saturating_sub(limit);
        // Whole frames go, so left and right stay paired.
        let excess = excess.div_ceil(usize::from(self.channels)) * usize::from(self.channels);
        self.samples.drain(..excess.min(self.samples.len()));
    }

    /// Takes everything captured so far, or `None` if there is nothing.
    fn take(&mut self) -> Option<SoundData> {
        let channels = usize::from(self.channels);
        let whole = self.samples.len() / channels * channels;
        if whole == 0 {
            return None;
        }
        let samples: Vec<f32> = self.samples.drain(..whole).collect();
        Some(SoundData::from_samples(samples, self.rate, self.channels))
    }

    fn sample_count(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }
}

fn lock(capture: &Mutex<Capture>) -> MutexGuard<'_, Capture> {
    capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A microphone or other device that can record sound.
///
/// Find one with [`recording_devices`], call [`RecordingDevice::start`],
/// and collect what it has heard with [`RecordingDevice::data`] as the
/// game runs, or all at once with [`RecordingDevice::stop`].
///
/// Every handle to the same device shares one recording.
#[derive(Clone)]
pub struct RecordingDevice {
    id: RecordingDeviceId,
    name: String,
    capture: Arc<Mutex<Capture>>,
}

impl RecordingDevice {
    /// The device's name, as the system gives it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Starts recording `sample_count` samples per channel at `rate`
    /// samples per second, with 1 or 2 channels. The device keeps that
    /// many and, if they are not read in time, drops the oldest to make
    /// room for new ones. LÖVE's bit depth is left out, since samples are
    /// always floating point.
    ///
    /// Starting a device that is recording starts it over, losing what it
    /// had not handed back.
    pub fn start(&self, sample_count: usize, rate: u32, channels: u16) -> Result<(), AudioError> {
        if !(1..=2).contains(&channels) {
            return Err(AudioError::Device(format!(
                "a recording must have 1 or 2 channels, not {channels}"
            )));
        }
        if rate == 0 || sample_count == 0 {
            return Err(AudioError::Device(
                "a recording needs a sample rate and a sample count above zero".to_owned(),
            ));
        }
        lock(&self.capture).restart(sample_count, rate, channels);
        let capture = self.capture.clone();
        let sink = Box::new(move |samples: &[f32]| lock(&capture).push(samples));
        let started =
            context::with(|ctx| ctx.backend.start_recording(self.id, rate, channels, sink));
        started.map_err(|message| {
            lock(&self.capture).recording = false;
            AudioError::Device(message)
        })
    }

    /// Stops recording and returns whatever had not been read yet.
    pub fn stop(&self) -> Option<SoundData> {
        context::with(|ctx| ctx.backend.stop_recording(self.id));
        let mut capture = lock(&self.capture);
        capture.recording = false;
        capture.take()
    }

    /// Returns what the device has recorded since the last call, and
    /// keeps recording. `None` if nothing new has arrived.
    pub fn data(&self) -> Option<SoundData> {
        lock(&self.capture).take()
    }

    /// How many samples per channel are waiting to be read.
    pub fn sample_count(&self) -> usize {
        lock(&self.capture).sample_count()
    }

    /// Whether the device is recording.
    pub fn is_recording(&self) -> bool {
        lock(&self.capture).recording
    }

    /// The sample rate of the current or last recording.
    pub fn rate(&self) -> u32 {
        lock(&self.capture).rate
    }

    /// The channel count of the current or last recording.
    pub fn channel_count(&self) -> u16 {
        lock(&self.capture).channels
    }
}

impl fmt::Debug for RecordingDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordingDevice")
            .field("name", &self.name)
            .field("recording", &self.is_recording())
            .finish()
    }
}

/// Returns the microphones and other devices that can record, the
/// equivalent of `love.audio.getRecordingDevices`. The list is empty if
/// there are none or the system does not allow recording.
///
/// Some systems ask the player for permission the first time a game
/// records.
pub fn recording_devices() -> Vec<RecordingDevice> {
    context::with(|ctx| {
        let devices = ctx.backend.recording_devices();
        devices
            .into_iter()
            .map(|(id, name)| RecordingDevice {
                id,
                name,
                capture: ctx.audio.recordings.entry(id).or_default().clone(),
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_kept_until_recording_starts() {
        let mut capture = Capture::default();
        capture.push(&[0.5; 10]);
        assert_eq!(capture.take(), None);
    }

    #[test]
    fn captured_sound_is_read_once() {
        let mut capture = Capture::default();
        capture.restart(100, 16_000, 1);
        capture.push(&[0.25, 0.5]);
        capture.push(&[0.75]);
        assert_eq!(capture.sample_count(), 3);
        let data = capture.take().unwrap();
        assert_eq!(data.rate(), 16_000);
        assert_eq!(data.samples(), [0.25, 0.5, 0.75]);
        assert_eq!(capture.take(), None);
    }

    #[test]
    fn a_full_recording_drops_its_oldest_frames() {
        let mut capture = Capture::default();
        capture.restart(2, 22_050, 2);
        capture.push(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
        assert_eq!(capture.sample_count(), 2);
        assert_eq!(capture.take().unwrap().samples(), [2.0, -2.0, 3.0, -3.0]);
    }

    #[test]
    fn starting_again_forgets_what_was_not_read() {
        let mut capture = Capture::default();
        capture.restart(10, 8_000, 1);
        capture.push(&[0.5; 4]);
        capture.restart(10, 44_100, 2);
        assert_eq!(capture.sample_count(), 0);
        assert_eq!(capture.channels, 2);
    }
}

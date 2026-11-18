//! Filters and effects applied to sounds as they are mixed, the equivalent
//! of LÖVE's `Source:setFilter` and `love.audio.setEffect`.
//!
//! The settings follow the OpenAL effects extension that LÖVE uses: the
//! same names, the same defaults and the same ranges. Values outside a
//! range are clamped to it rather than being an error.

use std::f32::consts::{FRAC_1_SQRT_2, LN_2, PI, TAU};
use std::iter::zip;

use smitten_core::backend::AUDIO_SAMPLE_RATE;

const RATE: f32 = AUDIO_SAMPLE_RATE as f32;

/// The frequency above which a filter's `high_gain` applies, in hertz.
const HIGH_REFERENCE: f32 = 5_000.0;
/// The frequency below which a filter's `low_gain` applies, in hertz.
const LOW_REFERENCE: f32 = 250.0;

/// Which frequencies a [`Filter`] turns down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterType {
    /// Turns down high frequencies by `high_gain`, muffling the sound.
    LowPass,
    /// Turns down low frequencies by `low_gain`, making the sound thin.
    HighPass,
    /// Turns down both, by `low_gain` and `high_gain`.
    BandPass,
}

/// A filter on a sound, the equivalent of the table LÖVE's
/// `Source:setFilter` takes.
///
/// Every gain is from 0.0 to 1.0, where 1.0 leaves that part of the sound
/// alone. High frequencies are those above about 5 kHz and low frequencies
/// those below about 250 Hz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
    pub kind: FilterType,
    /// The volume of the whole filtered sound.
    pub volume: f32,
    /// How much of the low frequencies is kept. Unused by a low-pass.
    pub low_gain: f32,
    /// How much of the high frequencies is kept. Unused by a high-pass.
    pub high_gain: f32,
}

impl Filter {
    /// A low-pass filter keeping `high_gain` of the high frequencies.
    pub fn low_pass(high_gain: f32) -> Self {
        Self {
            kind: FilterType::LowPass,
            volume: 1.0,
            low_gain: 1.0,
            high_gain,
        }
    }

    /// A high-pass filter keeping `low_gain` of the low frequencies.
    pub fn high_pass(low_gain: f32) -> Self {
        Self {
            kind: FilterType::HighPass,
            volume: 1.0,
            low_gain,
            high_gain: 1.0,
        }
    }

    /// A band-pass filter keeping `low_gain` of the low frequencies and
    /// `high_gain` of the high ones.
    pub fn band_pass(low_gain: f32, high_gain: f32) -> Self {
        Self {
            kind: FilterType::BandPass,
            volume: 1.0,
            low_gain,
            high_gain,
        }
    }

    pub(crate) fn clamped(self) -> Self {
        Self {
            kind: self.kind,
            volume: self.volume.clamp(0.0, 1.0),
            low_gain: self.low_gain.clamp(0.0, 1.0),
            high_gain: self.high_gain.clamp(0.0, 1.0),
        }
    }
}

/// The smoothing factor of a one-pole low-pass filter at `frequency`.
fn one_pole(frequency: f32) -> f32 {
    1.0 - (-TAU * frequency / RATE).exp()
}

/// The simplest low-pass filter: each output moves part of the way toward
/// the input.
#[derive(Debug, Clone, Copy, Default)]
struct OnePole {
    value: f32,
}

impl OnePole {
    fn next(&mut self, input: f32, factor: f32) -> f32 {
        self.value += factor * (input - self.value);
        self.value
    }
}

/// What a [`Filter`] remembers between one buffer and the next.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FilterState {
    below_low: [Biquad; 2],
    below_high: [Biquad; 2],
}

impl Default for FilterState {
    fn default() -> Self {
        let mut low = Biquad::default();
        low.set(Response::LowPass, LOW_REFERENCE);
        let mut high = Biquad::default();
        high.set(Response::LowPass, HIGH_REFERENCE);
        Self {
            below_low: [low; 2],
            below_high: [high; 2],
        }
    }
}

impl FilterState {
    /// Filters interleaved stereo in place.
    ///
    /// The sound is split at the two reference frequencies into low, middle
    /// and high parts, and each is scaled by its gain, so a filter with
    /// every gain at 1.0 changes nothing.
    pub fn process(&mut self, filter: &Filter, samples: &mut [f32]) {
        for frame in samples.as_chunks_mut::<2>().0 {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let input = *sample;
                let below_low = self.below_low[channel].next(input);
                let below_high = self.below_high[channel].next(input);
                let output = match filter.kind {
                    FilterType::LowPass => below_high + filter.high_gain * (input - below_high),
                    FilterType::HighPass => filter.low_gain * below_low + (input - below_low),
                    FilterType::BandPass => {
                        filter.low_gain * below_low
                            + (below_high - below_low)
                            + filter.high_gain * (input - below_high)
                    }
                };
                *sample = output * filter.volume;
            }
        }
    }
}

/// The shape of an oscillator in a [`Chorus`], [`Flanger`] or
/// [`RingModulator`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Waveform {
    Sine,
    /// For chorus and flanger only.
    Triangle,
    /// For the ring modulator only.
    Sawtooth,
    /// For the ring modulator only.
    Square,
}

impl Waveform {
    /// The oscillator's value at `phase`, from 0.0 to 1.0 through one cycle,
    /// between -1.0 and 1.0.
    fn at(self, phase: f32) -> f32 {
        match self {
            Waveform::Sine => (TAU * phase).sin(),
            Waveform::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
            Waveform::Sawtooth => 2.0 * phase - 1.0,
            Waveform::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

/// An effect that sources can be sent through with
/// [`Source::set_effect`](super::Source::set_effect), defined with
/// [`set_effect`](super::set_effect).
///
/// An effect adds to a sound rather than replacing it: the source still
/// plays as it would, and the effect's output, such as a reverb's echoes,
/// plays as well. Each kind of effect is its own struct with LÖVE's
/// defaults, so `Reverb { decay_time: 3.0, ..Default::default() }` changes
/// one setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    Reverb(Reverb),
    Chorus(Chorus),
    Distortion(Distortion),
    Echo(Echo),
    Flanger(Flanger),
    Compressor(Compressor),
    Equalizer(Equalizer),
    RingModulator(RingModulator),
}

macro_rules! into_effect {
    ($($kind:ident),*) => {
        $(impl From<$kind> for Effect {
            fn from(settings: $kind) -> Self {
                Effect::$kind(settings)
            }
        })*
    };
}

into_effect!(
    Reverb,
    Chorus,
    Distortion,
    Echo,
    Flanger,
    Compressor,
    Equalizer,
    RingModulator
);

/// The sound of a room: a burst of early reflections, then a tail that
/// dies away.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reverb {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// The overall level of the reverb, from 0.0 to 1.0.
    pub gain: f32,
    /// How much of the high frequencies reach the reverb, from 0.0 to 1.0.
    pub high_gain: f32,
    /// How closely packed the echoes are, from 0.0 to 1.0. Lower sounds
    /// like a smaller room.
    pub density: f32,
    /// How smeared together the echoes are, from 0.0 to 1.0.
    pub diffusion: f32,
    /// Seconds for the tail to die away, from 0.1 to 20.0.
    pub decay_time: f32,
    /// How long high frequencies last compared with low ones, from 0.1 to
    /// 2.0. Below 1.0 the tail grows duller as it fades.
    pub decay_high_ratio: f32,
    /// The level of the first reflections, from 0.0 to 3.16.
    pub early_gain: f32,
    /// Seconds before the first reflections, from 0.0 to 0.3.
    pub early_delay: f32,
    /// The level of the tail, from 0.0 to 10.0.
    pub late_gain: f32,
    /// Seconds from the first reflections to the tail, from 0.0 to 0.1.
    pub late_delay: f32,
}

impl Default for Reverb {
    fn default() -> Self {
        Self {
            volume: 1.0,
            gain: 0.32,
            high_gain: 0.89,
            density: 1.0,
            diffusion: 1.0,
            decay_time: 1.49,
            decay_high_ratio: 0.83,
            early_gain: 0.05,
            early_delay: 0.007,
            late_gain: 1.26,
            late_delay: 0.011,
        }
    }
}

/// Copies of the sound slightly delayed and wavering in pitch, making one
/// voice sound like several.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chorus {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// [`Waveform::Sine`] or [`Waveform::Triangle`].
    pub waveform: Waveform,
    /// Degrees between the left and right wavering, from -180 to 180.
    pub phase: f32,
    /// How fast the delay wavers, in hertz, from 0.0 to 10.0.
    pub rate: f32,
    /// How far the delay wavers, from 0.0 to 1.0.
    pub depth: f32,
    /// How much of the output is fed back in, from -1.0 to 1.0.
    pub feedback: f32,
    /// The average delay in seconds, from 0.0 to 0.016.
    pub delay: f32,
}

impl Default for Chorus {
    fn default() -> Self {
        Self {
            volume: 1.0,
            waveform: Waveform::Triangle,
            phase: 90.0,
            rate: 1.1,
            depth: 0.1,
            feedback: 0.25,
            delay: 0.016,
        }
    }
}

/// Like [`Chorus`] with a much shorter delay, giving a sweeping, jet-plane
/// sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flanger {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// [`Waveform::Sine`] or [`Waveform::Triangle`].
    pub waveform: Waveform,
    /// Degrees between the left and right sweep, from -180 to 180.
    pub phase: f32,
    /// How fast the sweep goes, in hertz, from 0.0 to 10.0.
    pub rate: f32,
    /// How far the sweep goes, from 0.0 to 1.0.
    pub depth: f32,
    /// How much of the output is fed back in, from -1.0 to 1.0.
    pub feedback: f32,
    /// The average delay in seconds, from 0.0 to 0.004.
    pub delay: f32,
}

impl Default for Flanger {
    fn default() -> Self {
        Self {
            volume: 1.0,
            waveform: Waveform::Triangle,
            phase: 0.0,
            rate: 0.27,
            depth: 1.0,
            feedback: -0.5,
            delay: 0.002,
        }
    }
}

/// A gritty, overdriven version of the sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Distortion {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// The level of the distorted sound, from 0.01 to 1.0.
    pub gain: f32,
    /// How hard the sound is clipped, from 0.0 to 1.0.
    pub edge: f32,
    /// Frequencies above this many hertz are removed before distorting,
    /// from 80 to 24000.
    pub low_cut: f32,
    /// The middle of the band kept after distorting, in hertz, from 80 to
    /// 24000.
    pub center: f32,
    /// The width of that band in hertz, from 80 to 24000.
    pub bandwidth: f32,
}

impl Default for Distortion {
    fn default() -> Self {
        Self {
            volume: 1.0,
            gain: 0.05,
            edge: 0.2,
            low_cut: 8_000.0,
            center: 3_600.0,
            bandwidth: 3_600.0,
        }
    }
}

/// Repeats of the sound that fade away, alternating between the left and
/// right speakers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Echo {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// Seconds before the first repeat, from 0.0 to 0.207.
    pub delay: f32,
    /// Seconds from the first repeat to the second, from 0.0 to 0.404.
    pub tap_delay: f32,
    /// How much duller each repeat is, from 0.0 to 0.99.
    pub damping: f32,
    /// How loud each round of repeats is compared with the last, from 0.0
    /// to 1.0.
    pub feedback: f32,
    /// From -1.0 to 1.0. At -1.0 the first repeat is on the left and the
    /// second on the right, at 1.0 the other way round, and at 0.0 both
    /// are in the middle.
    pub spread: f32,
}

impl Default for Echo {
    fn default() -> Self {
        Self {
            volume: 1.0,
            delay: 0.1,
            tap_delay: 0.1,
            damping: 0.5,
            feedback: 0.5,
            spread: -1.0,
        }
    }
}

/// Evens out the volume, turning down the loud parts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compressor {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// Whether the compressor does anything. When off, the sound passes
    /// through unchanged.
    pub enable: bool,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            volume: 1.0,
            enable: true,
        }
    }
}

/// Turns four ranges of frequencies up or down.
///
/// Gains are from 0.126 to 7.943, where 1.0 leaves that range alone.
/// Bandwidths are in octaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equalizer {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// The gain below `low_cut`.
    pub low_gain: f32,
    /// In hertz, from 50 to 800.
    pub low_cut: f32,
    pub low_mid_gain: f32,
    /// In hertz, from 200 to 3000.
    pub low_mid_frequency: f32,
    /// From 0.01 to 1.0.
    pub low_mid_bandwidth: f32,
    pub high_mid_gain: f32,
    /// In hertz, from 1000 to 8000.
    pub high_mid_frequency: f32,
    /// From 0.01 to 1.0.
    pub high_mid_bandwidth: f32,
    /// The gain above `high_cut`.
    pub high_gain: f32,
    /// In hertz, from 4000 to 16000.
    pub high_cut: f32,
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            volume: 1.0,
            low_gain: 1.0,
            low_cut: 200.0,
            low_mid_gain: 1.0,
            low_mid_frequency: 500.0,
            low_mid_bandwidth: 1.0,
            high_mid_gain: 1.0,
            high_mid_frequency: 3_000.0,
            high_mid_bandwidth: 1.0,
            high_gain: 1.0,
            high_cut: 6_000.0,
        }
    }
}

/// Multiplies the sound by a tone, for robotic and metallic voices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingModulator {
    /// The volume of the effect's output, from 0.0 to 1.0.
    pub volume: f32,
    /// [`Waveform::Sine`], [`Waveform::Sawtooth`] or [`Waveform::Square`].
    pub waveform: Waveform,
    /// The tone's frequency in hertz, from 0 to 8000.
    pub frequency: f32,
    /// Frequencies below this many hertz are removed first, from 0 to
    /// 24000. At 0 nothing is removed.
    pub high_cut: f32,
}

impl Default for RingModulator {
    fn default() -> Self {
        Self {
            volume: 1.0,
            waveform: Waveform::Sine,
            frequency: 440.0,
            high_cut: 800.0,
        }
    }
}

/// The kinds of filter a [`Biquad`] can be.
#[derive(Debug, Clone, Copy)]
enum Response {
    LowPass,
    HighPass,
    /// Unity gain at the center frequency.
    BandPass {
        q: f32,
    },
    /// A boost or cut around the center frequency, `octaves` wide.
    Peaking {
        gain: f32,
        octaves: f32,
    },
    LowShelf {
        gain: f32,
    },
    HighShelf {
        gain: f32,
    },
}

/// A second-order filter, from Robert Bristow-Johnson's audio EQ cookbook.
#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// Changes the response, keeping what the filter remembers so that the
    /// change does not click.
    fn set(&mut self, response: Response, frequency: f32) {
        let frequency = frequency.clamp(1.0, RATE * 0.49);
        let w0 = TAU * frequency / RATE;
        let (sin, cos) = w0.sin_cos();
        let butterworth = sin / (2.0 * FRAC_1_SQRT_2);
        let (b0, b1, b2, a0, a1, a2) = match response {
            Response::LowPass => {
                let alpha = butterworth;
                let b = (1.0 - cos) / 2.0;
                (b, 1.0 - cos, b, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Response::HighPass => {
                let alpha = butterworth;
                let b = (1.0 + cos) / 2.0;
                (b, -(1.0 + cos), b, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Response::BandPass { q } => {
                let alpha = sin / (2.0 * q.max(0.01));
                (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Response::Peaking { gain, octaves } => {
                let a = gain.max(1e-4).sqrt();
                let alpha = sin * (LN_2 / 2.0 * octaves * w0 / sin).sinh();
                (
                    1.0 + alpha * a,
                    -2.0 * cos,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos,
                    1.0 - alpha / a,
                )
            }
            Response::LowShelf { gain } => {
                let a = gain.max(1e-4).sqrt();
                let root = 2.0 * a.sqrt() * butterworth;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + root),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - root),
                    (a + 1.0) + (a - 1.0) * cos + root,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - root,
                )
            }
            Response::HighShelf { gain } => {
                let a = gain.max(1e-4).sqrt();
                let root = 2.0 * a.sqrt() * butterworth;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + root),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - root),
                    (a + 1.0) - (a - 1.0) * cos + root,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - root,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn next(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }
}

/// A ring buffer holding the last few seconds of a signal.
#[derive(Debug, Clone)]
struct DelayLine {
    samples: Vec<f32>,
    /// Where the next sample is written.
    next: usize,
}

impl DelayLine {
    fn new(seconds: f32) -> Self {
        Self {
            samples: vec![0.0; (seconds * RATE).ceil() as usize + 2],
            next: 0,
        }
    }

    /// The longest delay [`DelayLine::tap`] accepts, in samples.
    fn longest(&self) -> f32 {
        (self.samples.len() - 2) as f32
    }

    fn write(&mut self, sample: f32) {
        self.samples[self.next] = sample;
        self.next = (self.next + 1) % self.samples.len();
    }

    /// The sample written `delay` samples before the next write, which may
    /// fall between two samples. A delay of 1.0 is the latest sample.
    fn tap(&self, delay: f32) -> f32 {
        let delay = delay.clamp(1.0, self.longest());
        let whole = delay as usize;
        let blend = delay - whole as f32;
        let len = self.samples.len();
        let at = |back: usize| self.samples[(self.next + len - back) % len];
        let newer = at(whole);
        newer + (at(whole + 1) - newer) * blend
    }
}

/// One of the parallel echoes that build a reverb's tail.
#[derive(Debug, Clone)]
struct Comb {
    buffer: Vec<f32>,
    /// How much of the buffer is in use, which density shortens.
    len: usize,
    position: usize,
    damped: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len],
            len,
            position: 0,
            damped: 0.0,
        }
    }

    fn next(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let output = self.buffer[self.position];
        self.damped = output * (1.0 - damping) + self.damped * damping;
        self.buffer[self.position] = input + self.damped * feedback;
        self.position = (self.position + 1) % self.len;
        output
    }
}

/// Smears a reverb's echoes together.
#[derive(Debug, Clone)]
struct Allpass {
    buffer: Vec<f32>,
    position: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len],
            position: 0,
        }
    }

    fn next(&mut self, input: f32, feedback: f32) -> f32 {
        let delayed = self.buffer[self.position];
        self.buffer[self.position] = input + delayed * feedback;
        self.position = (self.position + 1) % self.buffer.len();
        delayed - input
    }
}

/// Freeverb's delay lengths in samples at 44.1 kHz, which sound good
/// because none is a multiple of another.
const COMB_LENGTHS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; 4] = [556, 441, 341, 225];
/// How much longer the right channel's delays are, for a wide sound.
const STEREO_SPREAD: usize = 23;
/// Early reflections arrive at these multiples of `early_delay`.
const EARLY_TAPS: [f32; 4] = [1.0, 1.31, 1.73, 2.11];

fn scaled_length(length: usize) -> usize {
    length * AUDIO_SAMPLE_RATE as usize / 44_100
}

#[derive(Debug, Clone)]
struct ReverbState {
    settings: Reverb,
    high_cut: OnePole,
    /// Holds the input for the early reflections and the start of the tail.
    delay: DelayLine,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl ReverbState {
    fn new(settings: Reverb) -> Self {
        let channel = |spread: usize| {
            let combs = COMB_LENGTHS
                .iter()
                .map(|&len| Comb::new(scaled_length(len + spread)))
                .collect();
            let allpasses = ALLPASS_LENGTHS
                .iter()
                .map(|&len| Allpass::new(scaled_length(len + spread)))
                .collect();
            (combs, allpasses)
        };
        let (left_combs, left_allpasses) = channel(0);
        let (right_combs, right_allpasses) = channel(STEREO_SPREAD);
        let mut state = Self {
            settings,
            high_cut: OnePole::default(),
            delay: DelayLine::new(0.3 * EARLY_TAPS[3] + 0.1),
            combs: [left_combs, right_combs],
            allpasses: [left_allpasses, right_allpasses],
        };
        state.configure(settings);
        state
    }

    fn configure(&mut self, settings: Reverb) {
        let settings = Reverb {
            volume: settings.volume.clamp(0.0, 1.0),
            gain: settings.gain.clamp(0.0, 1.0),
            high_gain: settings.high_gain.clamp(0.0, 1.0),
            density: settings.density.clamp(0.0, 1.0),
            diffusion: settings.diffusion.clamp(0.0, 1.0),
            decay_time: settings.decay_time.clamp(0.1, 20.0),
            decay_high_ratio: settings.decay_high_ratio.clamp(0.1, 2.0),
            early_gain: settings.early_gain.clamp(0.0, 3.16),
            early_delay: settings.early_delay.clamp(0.0, 0.3),
            late_gain: settings.late_gain.clamp(0.0, 10.0),
            late_delay: settings.late_delay.clamp(0.0, 0.1),
        };
        self.settings = settings;
        // A lower density means a smaller room, so shorter delays.
        let size = 0.4 + 0.6 * settings.density;
        for comb in self.combs.iter_mut().flatten() {
            comb.len = ((comb.buffer.len() as f32 * size) as usize).max(1);
            comb.position %= comb.len;
        }
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let settings = self.settings;
        let high_factor = one_pole(HIGH_REFERENCE);
        // Freeverb's scaling, which keeps eight combs from getting too loud.
        let input_scale = 0.015;
        let output_scale = 3.0 * settings.late_gain * settings.volume;
        let early_scale = settings.early_gain * settings.volume / EARLY_TAPS.len() as f32;
        let damping = ((1.0 - settings.decay_high_ratio).max(0.0) * 0.66).min(0.6);
        let diffusion = 0.5 * settings.diffusion;
        let early = settings.early_delay * RATE;
        let late = early + settings.late_delay * RATE;

        let feedback: [Vec<f32>; 2] = std::array::from_fn(|channel| {
            self.combs[channel]
                .iter()
                .map(|comb| 10f32.powf(-3.0 * comb.len as f32 / RATE / settings.decay_time))
                .collect()
        });

        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            let mono = (frame[0] + frame[1]) * 0.5 * settings.gain;
            let below = self.high_cut.next(mono, high_factor);
            self.delay
                .write(below + settings.high_gain * (mono - below));

            let reflections: f32 = EARLY_TAPS
                .iter()
                .map(|&tap| self.delay.tap(early * tap))
                .sum();
            let tail_input = self.delay.tap(late) * input_scale;
            for channel in 0..2 {
                let mut tail = 0.0;
                for (comb, &feedback) in self.combs[channel].iter_mut().zip(&feedback[channel]) {
                    tail += comb.next(tail_input, feedback, damping);
                }
                for allpass in &mut self.allpasses[channel] {
                    tail = allpass.next(tail, diffusion);
                }
                output[channel] += reflections * early_scale + tail * output_scale;
            }
        }
    }
}

/// The shared workings of [`Chorus`] and [`Flanger`].
#[derive(Debug, Clone)]
struct ModulatedDelay {
    volume: f32,
    waveform: Waveform,
    phase: f32,
    rate: f32,
    depth: f32,
    feedback: f32,
    delay: f32,
    lines: [DelayLine; 2],
    /// Where the oscillator is in its cycle, from 0.0 to 1.0.
    position: f32,
}

impl ModulatedDelay {
    fn new(longest_delay: f32) -> Self {
        Self {
            volume: 1.0,
            waveform: Waveform::Triangle,
            phase: 0.0,
            rate: 0.0,
            depth: 0.0,
            feedback: 0.0,
            delay: 0.0,
            lines: std::array::from_fn(|_| DelayLine::new(longest_delay * 2.0 + 0.001)),
            position: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn configure(
        &mut self,
        volume: f32,
        waveform: Waveform,
        phase: f32,
        rate: f32,
        depth: f32,
        feedback: f32,
        delay: f32,
        longest_delay: f32,
    ) {
        self.volume = volume.clamp(0.0, 1.0);
        self.waveform = match waveform {
            Waveform::Sine => Waveform::Sine,
            _ => Waveform::Triangle,
        };
        self.phase = phase.clamp(-180.0, 180.0) / 360.0;
        self.rate = rate.clamp(0.0, 10.0);
        self.depth = depth.clamp(0.0, 1.0);
        // Feedback of exactly 1.0 would ring forever.
        self.feedback = feedback.clamp(-0.99, 0.99);
        self.delay = delay.clamp(0.0, longest_delay);
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let step = self.rate / RATE;
        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            let mono = (frame[0] + frame[1]) * 0.5;
            let lines = self.lines.iter_mut().zip(output.iter_mut());
            for (channel, (line, output)) in lines.enumerate() {
                let offset = if channel == 1 { self.phase } else { 0.0 };
                let wobble = self.waveform.at((self.position + offset).rem_euclid(1.0));
                let delay = self.delay * (1.0 + self.depth * wobble) * RATE;
                let delayed = line.tap(delay);
                line.write(mono + delayed * self.feedback);
                *output += delayed * self.volume;
            }
            self.position = (self.position + step).fract();
        }
    }
}

#[derive(Debug, Clone)]
struct EchoState {
    settings: Echo,
    line: DelayLine,
    damped: f32,
}

impl EchoState {
    fn new(settings: Echo) -> Self {
        let mut state = Self {
            settings,
            line: DelayLine::new(0.207 + 0.404),
            damped: 0.0,
        };
        state.configure(settings);
        state
    }

    fn configure(&mut self, settings: Echo) {
        self.settings = Echo {
            volume: settings.volume.clamp(0.0, 1.0),
            delay: settings.delay.clamp(0.0, 0.207),
            tap_delay: settings.tap_delay.clamp(0.0, 0.404),
            damping: settings.damping.clamp(0.0, 0.99),
            feedback: settings.feedback.clamp(0.0, 1.0),
            spread: settings.spread.clamp(-1.0, 1.0),
        };
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let settings = self.settings;
        let first = (settings.delay * RATE).max(1.0);
        let second = first + settings.tap_delay * RATE;
        // At a spread of -1 the first repeat is all left and the second all
        // right.
        let near = (1.0 - settings.spread) / 2.0 * settings.volume;
        let far = (1.0 + settings.spread) / 2.0 * settings.volume;
        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            let mono = (frame[0] + frame[1]) * 0.5;
            let tap1 = self.line.tap(first);
            let tap2 = self.line.tap(second);
            self.damped = tap2 * (1.0 - settings.damping) + self.damped * settings.damping;
            self.line.write(mono + self.damped * settings.feedback);
            output[0] += tap1 * near + tap2 * far;
            output[1] += tap1 * far + tap2 * near;
        }
    }
}

#[derive(Debug, Clone)]
struct DistortionState {
    settings: Distortion,
    low_cut: Biquad,
    band: Biquad,
}

impl DistortionState {
    fn new(settings: Distortion) -> Self {
        let mut state = Self {
            settings,
            low_cut: Biquad::default(),
            band: Biquad::default(),
        };
        state.configure(settings);
        state
    }

    fn configure(&mut self, settings: Distortion) {
        let settings = Distortion {
            volume: settings.volume.clamp(0.0, 1.0),
            gain: settings.gain.clamp(0.01, 1.0),
            edge: settings.edge.clamp(0.0, 1.0),
            low_cut: settings.low_cut.clamp(80.0, 24_000.0),
            center: settings.center.clamp(80.0, 24_000.0),
            bandwidth: settings.bandwidth.clamp(80.0, 24_000.0),
        };
        self.settings = settings;
        self.low_cut.set(Response::LowPass, settings.low_cut);
        self.band.set(
            Response::BandPass {
                q: settings.center / settings.bandwidth,
            },
            settings.center,
        );
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let settings = self.settings;
        let edge = (settings.edge * PI / 2.0).sin().min(0.99);
        let hardness = 2.0 * edge / (1.0 - edge);
        let level = settings.gain * settings.volume;
        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            let mono = (frame[0] + frame[1]) * 0.5;
            // Overdrive the input so that even quiet sounds are clipped.
            let driven = self.low_cut.next(mono) * 4.0;
            let clipped = (1.0 + hardness) * driven / (1.0 + hardness * driven.abs());
            let shaped = self.band.next(clipped.clamp(-1.0, 1.0)) * level;
            output[0] += shaped;
            output[1] += shaped;
        }
    }
}

#[derive(Debug, Clone)]
struct RingModulatorState {
    settings: RingModulator,
    high_cut: [Biquad; 2],
    position: f32,
}

impl RingModulatorState {
    fn new(settings: RingModulator) -> Self {
        let mut state = Self {
            settings,
            high_cut: [Biquad::default(); 2],
            position: 0.0,
        };
        state.configure(settings);
        state
    }

    fn configure(&mut self, settings: RingModulator) {
        let settings = RingModulator {
            volume: settings.volume.clamp(0.0, 1.0),
            waveform: match settings.waveform {
                Waveform::Triangle => Waveform::Sine,
                waveform => waveform,
            },
            frequency: settings.frequency.clamp(0.0, 8_000.0),
            high_cut: settings.high_cut.clamp(0.0, 24_000.0),
        };
        self.settings = settings;
        for filter in &mut self.high_cut {
            filter.set(Response::HighPass, settings.high_cut);
        }
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let settings = self.settings;
        let step = settings.frequency / RATE;
        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            let tone = settings.waveform.at(self.position) * settings.volume;
            for channel in 0..2 {
                let input = if settings.high_cut < 1.0 {
                    frame[channel]
                } else {
                    self.high_cut[channel].next(frame[channel])
                };
                output[channel] += input * tone;
            }
            self.position = (self.position + step).fract();
        }
    }
}

/// Above this level the compressor turns the sound down.
const COMPRESSOR_THRESHOLD: f32 = 0.25;
/// How much the compressor squeezes sound above the threshold: four times
/// as much input over the threshold gives only one times as much output.
const COMPRESSOR_RATIO: f32 = 4.0;

#[derive(Debug, Clone)]
struct CompressorState {
    settings: Compressor,
    envelope: f32,
}

impl CompressorState {
    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let volume = self.settings.volume.clamp(0.0, 1.0);
        let attack = 1.0 - (-1.0 / (0.001 * RATE)).exp();
        let release = 1.0 - (-1.0 / (0.2 * RATE)).exp();
        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            let gain = if self.settings.enable {
                let peak = frame[0].abs().max(frame[1].abs());
                let speed = if peak > self.envelope {
                    attack
                } else {
                    release
                };
                self.envelope += (peak - self.envelope) * speed;
                if self.envelope > COMPRESSOR_THRESHOLD {
                    (COMPRESSOR_THRESHOLD / self.envelope).powf(1.0 - 1.0 / COMPRESSOR_RATIO)
                } else {
                    1.0
                }
            } else {
                1.0
            };
            output[0] += frame[0] * gain * volume;
            output[1] += frame[1] * gain * volume;
        }
    }
}

#[derive(Debug, Clone)]
struct EqualizerState {
    settings: Equalizer,
    bands: [[Biquad; 4]; 2],
}

impl EqualizerState {
    fn new(settings: Equalizer) -> Self {
        let mut state = Self {
            settings,
            bands: [[Biquad::default(); 4]; 2],
        };
        state.configure(settings);
        state
    }

    fn configure(&mut self, settings: Equalizer) {
        let gain = |gain: f32| gain.clamp(0.126, 7.943);
        let settings = Equalizer {
            volume: settings.volume.clamp(0.0, 1.0),
            low_gain: gain(settings.low_gain),
            low_cut: settings.low_cut.clamp(50.0, 800.0),
            low_mid_gain: gain(settings.low_mid_gain),
            low_mid_frequency: settings.low_mid_frequency.clamp(200.0, 3_000.0),
            low_mid_bandwidth: settings.low_mid_bandwidth.clamp(0.01, 1.0),
            high_mid_gain: gain(settings.high_mid_gain),
            high_mid_frequency: settings.high_mid_frequency.clamp(1_000.0, 8_000.0),
            high_mid_bandwidth: settings.high_mid_bandwidth.clamp(0.01, 1.0),
            high_gain: gain(settings.high_gain),
            high_cut: settings.high_cut.clamp(4_000.0, 16_000.0),
        };
        self.settings = settings;
        for bands in &mut self.bands {
            bands[0].set(
                Response::LowShelf {
                    gain: settings.low_gain,
                },
                settings.low_cut,
            );
            bands[1].set(
                Response::Peaking {
                    gain: settings.low_mid_gain,
                    octaves: settings.low_mid_bandwidth,
                },
                settings.low_mid_frequency,
            );
            bands[2].set(
                Response::Peaking {
                    gain: settings.high_mid_gain,
                    octaves: settings.high_mid_bandwidth,
                },
                settings.high_mid_frequency,
            );
            bands[3].set(
                Response::HighShelf {
                    gain: settings.high_gain,
                },
                settings.high_cut,
            );
        }
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let volume = self.settings.volume;
        for (frame, output) in zip(input.as_chunks::<2>().0, out.as_chunks_mut::<2>().0) {
            for channel in 0..2 {
                let shaped = self.bands[channel]
                    .iter_mut()
                    .fold(frame[channel], |sample, band| band.next(sample));
                output[channel] += shaped * volume;
            }
        }
    }
}

/// An effect's settings together with what it remembers, such as a
/// reverb's echoes.
#[derive(Debug, Clone)]
pub(crate) struct EffectState(State);

#[derive(Debug, Clone)]
enum State {
    Reverb(Box<ReverbState>),
    Chorus(ModulatedDelay),
    Flanger(ModulatedDelay),
    Echo(EchoState),
    Distortion(DistortionState),
    RingModulator(RingModulatorState),
    Compressor(CompressorState),
    Equalizer(EqualizerState),
}

const LONGEST_CHORUS: f32 = 0.016;
const LONGEST_FLANGER: f32 = 0.004;

impl EffectState {
    pub fn new(effect: &Effect) -> Self {
        let mut state = EffectState(match *effect {
            Effect::Reverb(settings) => State::Reverb(Box::new(ReverbState::new(settings))),
            Effect::Chorus(_) => State::Chorus(ModulatedDelay::new(LONGEST_CHORUS)),
            Effect::Flanger(_) => State::Flanger(ModulatedDelay::new(LONGEST_FLANGER)),
            Effect::Echo(settings) => State::Echo(EchoState::new(settings)),
            Effect::Distortion(settings) => State::Distortion(DistortionState::new(settings)),
            Effect::RingModulator(settings) => {
                State::RingModulator(RingModulatorState::new(settings))
            }
            Effect::Compressor(settings) => State::Compressor(CompressorState {
                settings,
                envelope: 0.0,
            }),
            Effect::Equalizer(settings) => State::Equalizer(EqualizerState::new(settings)),
        });
        state.configure(effect);
        state
    }

    /// Changes the settings. An effect of the same kind keeps what it
    /// remembers, so a reverb's tail carries on; a different kind starts
    /// afresh.
    pub fn configure(&mut self, effect: &Effect) {
        match (&mut self.0, *effect) {
            (State::Reverb(state), Effect::Reverb(settings)) => state.configure(settings),
            (State::Chorus(state), Effect::Chorus(s)) => state.configure(
                s.volume,
                s.waveform,
                s.phase,
                s.rate,
                s.depth,
                s.feedback,
                s.delay,
                LONGEST_CHORUS,
            ),
            (State::Flanger(state), Effect::Flanger(s)) => state.configure(
                s.volume,
                s.waveform,
                s.phase,
                s.rate,
                s.depth,
                s.feedback,
                s.delay,
                LONGEST_FLANGER,
            ),
            (State::Echo(state), Effect::Echo(settings)) => state.configure(settings),
            (State::Distortion(state), Effect::Distortion(settings)) => state.configure(settings),
            (State::RingModulator(state), Effect::RingModulator(settings)) => {
                state.configure(settings)
            }
            (State::Compressor(state), Effect::Compressor(settings)) => state.settings = settings,
            (State::Equalizer(state), Effect::Equalizer(settings)) => state.configure(settings),
            _ => *self = EffectState::new(effect),
        }
    }

    /// Runs the effect on interleaved stereo `input` and adds its output
    /// to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        match &mut self.0 {
            State::Reverb(state) => state.process(input, out),
            State::Chorus(state) | State::Flanger(state) => state.process(input, out),
            State::Echo(state) => state.process(input, out),
            State::Distortion(state) => state.process(input, out),
            State::RingModulator(state) => state.process(input, out),
            State::Compressor(state) => state.process(input, out),
            State::Equalizer(state) => state.process(input, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a sine wave at `frequency`, as interleaved stereo.
    fn tone(frequency: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let value = (TAU * frequency * i as f32 / RATE).sin() * 0.5;
                [value, value]
            })
            .collect()
    }

    /// The loudness of the second half of the left channel, after any
    /// filter has settled.
    fn loudness(samples: &[f32]) -> f32 {
        let left: Vec<f32> = samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        let settled = &left[left.len() / 2..];
        (settled.iter().map(|s| s * s).sum::<f32>() / settled.len() as f32).sqrt()
    }

    fn filtered(filter: Filter, frequency: f32) -> f32 {
        let mut samples = tone(frequency, 4_800);
        FilterState::default().process(&filter, &mut samples);
        loudness(&samples) / loudness(&tone(frequency, 4_800))
    }

    /// An impulse followed by silence, run through `effect`.
    fn impulse_response(effect: Effect, frames: usize) -> Vec<f32> {
        let mut input = vec![0.0; frames * 2];
        input[0] = 1.0;
        input[1] = 1.0;
        let mut out = vec![0.0; frames * 2];
        EffectState::new(&effect).process(&input, &mut out);
        out
    }

    #[test]
    fn a_filter_with_every_gain_at_one_changes_nothing() {
        for filter in [
            Filter::low_pass(1.0),
            Filter::high_pass(1.0),
            Filter::band_pass(1.0, 1.0),
        ] {
            let mut samples = tone(3_000.0, 1_000);
            let original = samples.clone();
            FilterState::default().process(&filter, &mut samples);
            for (a, b) in samples.iter().zip(&original) {
                assert!((a - b).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn a_low_pass_quiets_high_notes_but_not_low_ones() {
        let filter = Filter::low_pass(0.0);
        assert!(filtered(filter, 100.0) > 0.95);
        assert!(filtered(filter, 15_000.0) < 0.3);
    }

    #[test]
    fn a_high_pass_quiets_low_notes_but_not_high_ones() {
        let filter = Filter::high_pass(0.0);
        assert!(filtered(filter, 30.0) < 0.3);
        assert!(filtered(filter, 10_000.0) > 0.95);
    }

    #[test]
    fn a_band_pass_keeps_the_middle() {
        let filter = Filter::band_pass(0.0, 0.0);
        assert!(filtered(filter, 30.0) < 0.3);
        assert!(filtered(filter, 1_200.0) > 0.7);
        assert!(filtered(filter, 15_000.0) < 0.3);
    }

    #[test]
    fn a_filter_volume_scales_everything() {
        let filter = Filter {
            volume: 0.5,
            ..Filter::low_pass(1.0)
        };
        assert!((filtered(filter, 440.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn an_echo_repeats_after_its_delay() {
        let echo = Echo {
            delay: 0.1,
            tap_delay: 0.05,
            feedback: 0.0,
            spread: -1.0,
            ..Echo::default()
        };
        let out = impulse_response(echo.into(), 12_000);
        let first = (0.1 * RATE) as usize;
        let second = first + (0.05 * RATE) as usize;
        // The first repeat is on the left, the second on the right.
        assert!(out[first * 2] > 0.9 && out[first * 2 + 1].abs() < 1e-6);
        assert!(out[second * 2].abs() < 1e-6 && out[second * 2 + 1] > 0.4);
        let elsewhere: f32 = out
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != first && i != second)
            .map(|(_, frame)| frame[0].abs() + frame[1].abs())
            .sum();
        assert!(elsewhere < 1e-6);
    }

    #[test]
    fn an_echo_with_feedback_keeps_repeating_more_quietly() {
        let echo = Echo {
            delay: 0.01,
            tap_delay: 0.0,
            damping: 0.0,
            feedback: 0.5,
            spread: 0.0,
            ..Echo::default()
        };
        let out = impulse_response(echo.into(), 2_000);
        let gap = (0.01 * RATE) as usize;
        let repeats: Vec<f32> = (1..4).map(|n| out[n * gap * 2]).collect();
        assert!(repeats[0] > repeats[1] && repeats[1] > repeats[2] && repeats[2] > 0.0);
    }

    #[test]
    fn a_reverb_rings_on_after_the_sound_and_then_dies_away() {
        let out = impulse_response(Reverb::default().into(), RATE as usize * 3);
        let energy = |from: f32, to: f32| -> f32 {
            out[(from * RATE) as usize * 2..(to * RATE) as usize * 2]
                .iter()
                .map(|s| s * s)
                .sum()
        };
        assert!(energy(0.05, 0.3) > 1e-4, "there should be a tail");
        assert!(energy(2.7, 3.0) < energy(0.05, 0.3) / 100.0);
        assert!(out.iter().all(|s| s.is_finite() && s.abs() < 1.0));
    }

    #[test]
    fn a_longer_decay_time_rings_for_longer() {
        let late_energy = |decay_time: f32| -> f32 {
            let reverb = Reverb {
                decay_time,
                ..Reverb::default()
            };
            impulse_response(reverb.into(), RATE as usize)[RATE as usize..]
                .iter()
                .map(|s| s * s)
                .sum()
        };
        assert!(late_energy(5.0) > late_energy(0.5) * 10.0);
    }

    #[test]
    fn changing_settings_keeps_the_tail() {
        let mut state = EffectState::new(&Reverb::default().into());
        let mut input = vec![0.0; 9_600];
        input[0] = 1.0;
        let mut out = vec![0.0; 9_600];
        state.process(&input, &mut out);

        state.configure(
            &Reverb {
                gain: 0.0,
                ..Reverb::default()
            }
            .into(),
        );
        let mut later = vec![0.0; 9_600];
        state.process(&vec![0.0; 9_600], &mut later);
        assert!(later.iter().any(|&s| s.abs() > 1e-5));
    }

    #[test]
    fn every_effect_stays_finite_and_reasonably_quiet() {
        let effects: [Effect; 8] = [
            Reverb::default().into(),
            Chorus::default().into(),
            Distortion::default().into(),
            Echo::default().into(),
            Flanger::default().into(),
            Compressor::default().into(),
            Equalizer::default().into(),
            RingModulator::default().into(),
        ];
        let input = tone(440.0, 48_000);
        for effect in effects {
            let mut out = vec![0.0; input.len()];
            EffectState::new(&effect).process(&input, &mut out);
            assert!(
                out.iter().all(|s| s.is_finite() && s.abs() < 4.0),
                "{effect:?} misbehaved"
            );
            assert!(out.iter().any(|&s| s != 0.0), "{effect:?} was silent");
        }
    }

    #[test]
    fn a_flat_equalizer_changes_nothing() {
        let input = tone(1_000.0, 4_800);
        let mut out = vec![0.0; input.len()];
        EffectState::new(&Equalizer::default().into()).process(&input, &mut out);
        for (a, b) in out.iter().zip(&input) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn an_equalizer_boosts_the_band_it_is_told_to() {
        let eq = Equalizer {
            high_gain: 4.0,
            ..Equalizer::default()
        };
        let louder = |frequency: f32| {
            let input = tone(frequency, 9_600);
            let mut out = vec![0.0; input.len()];
            EffectState::new(&eq.into()).process(&input, &mut out);
            loudness(&out) / loudness(&input)
        };
        assert!(louder(15_000.0) > 3.0);
        assert!((louder(200.0) - 1.0).abs() < 0.1);
    }

    #[test]
    fn a_compressor_turns_loud_sound_down_and_leaves_quiet_sound() {
        let run = |level: f32, enable: bool| {
            let input: Vec<f32> = tone(440.0, 9_600).iter().map(|s| s * level).collect();
            let mut out = vec![0.0; input.len()];
            let compressor = Compressor {
                enable,
                ..Compressor::default()
            };
            EffectState::new(&compressor.into()).process(&input, &mut out);
            loudness(&out) / loudness(&input)
        };
        assert!(run(1.8, true) < 0.7);
        assert!((run(0.2, true) - 1.0).abs() < 1e-3);
        assert!((run(1.8, false) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_ring_modulator_with_a_square_wave_flips_the_sound() {
        let ring = RingModulator {
            waveform: Waveform::Square,
            frequency: 100.0,
            high_cut: 0.0,
            ..RingModulator::default()
        };
        let input = vec![0.5; 2_000];
        let mut out = vec![0.0; input.len()];
        EffectState::new(&ring.into()).process(&input, &mut out);
        // Half a cycle at 100 Hz is 240 frames.
        assert!((out[100] - 0.5).abs() < 0.01);
        assert!((out[2 * 300] + 0.5).abs() < 0.01);
    }
}

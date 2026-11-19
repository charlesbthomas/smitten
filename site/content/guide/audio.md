+++
title = "Audio"
description = "Sound effects, music, positional sound, filters, effects and sounds made in code."
weight = 7
+++

The {{ api(path="audio/index.html", text="audio") }} module is the equivalent
of `love.audio`. It plays WAV, Ogg Vorbis, MP3 and FLAC files.

## Sound effects and music

Load a {{ api(path="audio/struct.Source.html", text="Source") }} and play it.
Short sounds should be `SourceType::Static`, which decodes them into memory
once. Music and other long sounds should be `SourceType::Stream`, which
decodes as it plays:

```rust
use smitten::audio::{Source, SourceType};

let jump = Source::load("jump.wav", SourceType::Static)?;
let music = Source::load("theme.ogg", SourceType::Stream)?;
music.set_looping(true);
music.set_volume(0.6);
music.play();
```

A source plays one copy of its sound at a time. For a sound effect that may
overlap itself, use `play_once`, which fires off a copy and cleans up after
it. `play_once_with(volume, pitch)` varies it, so a sound that repeats
doesn't sound mechanical:

```rust
fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
    if key == Key::Space {
        let pitch = smitten::math::random_range(0.9, 1.1);
        self.jump.play_once_with(1.0, pitch);
    }
}
```

`pause`, `stop`, `seek` and `tell` control playback, with positions as
`std::time::Duration`. `audio::set_volume` sets the master volume.

## Positional sound

A mono source can be placed in the world with `set_position`, and the
listener with `audio::set_position`. The sound then pans toward the side it
is on, gets quieter with distance, and changes pitch as it moves if it has
a velocity. Stereo sources are never positioned, so music isn't affected.

Positions are 3D {{ api(path="math/struct.Vec3.html", text="Vec3") }} values,
in whatever unit the game uses. With pixels, give sources a larger
reference distance, since the default of 1.0 makes a sound half as loud two
pixels away:

```rust
use smitten::audio;

audio::set_position((400.0, 300.0, 100.0));
growl.set_position((monster.x, monster.y, 0.0));
growl.set_attenuation_distances(150.0, f32::MAX);
```

A listener level with the sounds in a 2D game hears them panned fully to
one side. Placing it a little in front of the screen, as the `100.0` above
does, softens that. `audio::set_distance_model` picks how volume falls off
with distance.

## Filters and effects

`set_filter` muffles or thins a single source, such as a sound behind a
wall. `audio::set_effect` defines a named effect, such as a
{{ api(path="audio/struct.Reverb.html", text="Reverb") }} or an
{{ api(path="audio/struct.Echo.html", text="Echo") }}, and `source.set_effect`
sends a source through it:

```rust
use smitten::audio::{self, Filter, Reverb};

audio::set_effect(
    "cave",
    Reverb {
        decay_time: 2.5,
        ..Reverb::default()
    },
);
footsteps.set_effect("cave", None);
footsteps.set_filter(Some(Filter::low_pass(0.1)));
```

The settings follow the OpenAL effects LÖVE uses, with the same names,
defaults and ranges. Chorus, distortion, flanger, compressor and
equalizer effects are there too.

{{ figure(src="spatial-audio-example.png", alt="A ring for the listener in the middle of the window and a dot for the sound that follows the mouse.", caption="The spatial audio example: a hum made in code follows the mouse, with reverb and echo to switch on.") }}

## Sounds made in code

A {{ api(path="sound/struct.SoundData.html", text="SoundData") }} holds samples
you can read and write. Fill one and play it with `Source::from_sound_data`:

```rust
use std::f32::consts::TAU;
use smitten::audio::Source;
use smitten::sound::SoundData;

let rate = 44_100;
let mut beep = SoundData::new(rate as usize / 4, rate, 1);
for i in 0..beep.sample_count() {
    let t = i as f32 / rate as f32;
    beep.set_sample(i, 0, (TAU * 440.0 * t).sin() * 0.3);
}
let beep = Source::from_sound_data(&beep);
beep.play();
```

For sound generated while the game runs, such as a synthesizer, a
{{ api(path="audio/struct.QueueableSource.html", text="QueueableSource") }}
plays buffers as you queue them. A
{{ api(path="sound/struct.Decoder.html", text="Decoder") }} reads a long file a
piece at a time, and microphones can be recorded with
`audio::recording_devices`.

## Video

Videos are Ogg Theora files with an optional Vorbis sound track, as in
LÖVE. Load one with `graphics::Video::load`, call `play`, and draw it like an
image. FFmpeg makes one from almost any video:

```sh
ffmpeg -i input.mp4 -c:v libtheora -q:v 7 -c:a libvorbis -q:a 4 output.ogv
```

The `video` Cargo feature, on by default, can be turned off to leave the
decoder out of a game that plays no video.

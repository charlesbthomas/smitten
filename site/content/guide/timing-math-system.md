+++
title = "Timing, math, events and the system"
description = "The timer, random numbers and noise, custom events, and the computer the game runs on."
weight = 11
[extra]
nav = "Timing, math and system"
+++

## Timer

{{ api(path="timer/index.html", text="timer") }} measures time. The `dt` passed to `update` is usually all a game needs, but:

- `timer::time()` is the seconds since the game started, as an `f64`. Use
  it for animations that depend on time alone.
- `timer::fps()` is the frame rate, and `average_delta()` the average frame
  time over the last second.
- `timer::delta()` is the last frame's `dt`, from anywhere.
- `timer::sleep(Duration)` pauses the thread.

```rust
use smitten::{graphics, timer, window};

let bob = (timer::time() as f32 * 3.0).sin() * 4.0;
graphics::draw(&coin, (x, y + bob));
window::set_title(&format!("My game ({} FPS)", timer::fps()));
```

## Random numbers and noise

{{ api(path="math/index.html", text="math") }} has random numbers,
noise and a little geometry. It works without a running game, so it can be used in tests
and tools:

```rust
use smitten::math;

math::set_random_seed(42);
let roll = math::random_int(1, 6);
let speed = math::random_range(100.0, 200.0);
let jitter = math::random_normal(0.0, 1.0);
if math::chance(0.1) {
    // one time in ten
}
let loot = math::choose(&["sword", "shield", "potion"]);

// Smooth noise from 0 to 1, for terrain and wobble.
let height = math::noise2(x * 0.05, y * 0.05);
```

`shuffle`, `noise` in one to four dimensions, `is_convex` and
`triangulate` are there too, and `math::random_state` and
`set_random_state` save and restore the generator, for replays.
`Vec2` and `Vec3` come from [glam](https://docs.rs/glam) and have the
vector arithmetic built in.

## Events

{{ api(path="event/index.html", text="event") }} handles game-level
events. `event::quit()` asks the game to close. `event::push(value)`
sends the game an event of any type, which arrives in the `custom_event`
callback at the start of the next frame:

```rust
use smitten::event::{self, CustomEvent};

struct LevelComplete(u32);

// Anywhere:
event::push(LevelComplete(3));

// In the game:
fn custom_event(&mut self, event: CustomEvent) {
    if let Some(LevelComplete(level)) = event.downcast_ref::<LevelComplete>() {
        self.show_results(*level);
    }
}
```

`event::poll`, `pump` and `wait` are for games that
[write their own main loop](@/guide/game-loop.md#writing-your-own-main-loop).

## The system

{{ api(path="system/index.html", text="system") }} describes the
computer the game runs on: `system::os()`, `processor_count()`, `clipboard_text()` and
`set_clipboard_text`, `open_url`, `power_info()` for the battery, and
`preferred_locales()` for the player's languages.

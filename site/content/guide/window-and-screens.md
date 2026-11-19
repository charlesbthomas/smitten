+++
title = "Windows, screens and high-DPI"
description = "Units and pixels, fixed design sizes, fullscreen and the window module."
weight = 10
[extra]
nav = "Windows and screens"
+++

## Units and pixels

Sizes and positions are measured in units. On an ordinary
display a unit is one pixel. On a high-DPI display, such as a Retina screen
or Windows set to 150%, a unit covers more pixels, so an 800 by 600 game
looks the same size everywhere and is simply drawn sharper. Text and
canvases are made as sharp as the screen automatically.

{{ figure(src="text-at-150-percent.png", alt="The text example on a display scaled to 150 percent, with crisp text.", caption="The text example on Windows at 150% scaling.") }}

`graphics::dimensions()` gives the size in units, and
`graphics::pixel_dimensions()` and `window::dpi_scale()` give the real
pixels when you need them. `Config { dpi_scaling: false, .. }` switches to
raw pixels.

## A fixed design size

A game laid out for one size can set a design size. It then always draws in
a space of exactly that size, and Smitten scales the picture to the window,
with black bars where the shapes differ. The mouse position arrives in the
same space.

```rust
use smitten::{Config, Scaling};

smitten::run::<MyGame>(Config {
    resizable: true,
    design_size: Some((320, 180)),
    design_scaling: Scaling::PixelPerfect,
    ..Config::default()
});
```

- `Scaling::Fit`, the default, scales smoothly and draws at the window's
  full resolution. It suits high-resolution art.
- `Scaling::PixelPerfect` draws at exactly the design size and enlarges by
  whole numbers with hard pixel edges, so every game pixel stays square. It
  suits pixel art.

{{ figure(src="design-pixel-perfect.png", alt="A 320 by 180 pixel art scene enlarged three times in a 960 by 600 window, with black bars above and below.", caption="The design example: 320 by 180, enlarged three times with Scaling::PixelPerfect.") }}

[Pong](@/examples/_index.md#pong) uses a design size of 800 by 600 so its
court fills any window.

## Fullscreen

```rust
use smitten::{Key, window};

fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
    if key == Key::F11 {
        window::set_fullscreen(!window::fullscreen());
    }
}
```

Fullscreen is the desktop kind by default, which covers the screen without
changing its resolution. `window::set_fullscreen_with(true,
FullscreenType::Exclusive)` changes the display mode instead, and
`window::fullscreen_modes(display)` lists the modes it can use.

## The window module

{{ api(path="window/index.html", text="window") }} controls the window:

- `set_title`, `set_size`, `set_position`, `minimize`, `maximize` and
  `restore` change the window.
- `set_mode(width, height, mode)` and `update_mode` change several settings
  at once.
- `set_icon(path)` sets the icon, as `Config::icon` does at startup.
- `set_vsync` and `set_msaa` change presentation.
- `display_count`, `desktop_dimensions`, `display_name` and
  `display_dpi_scale` describe the displays.
- `show_message_box` and `show_message_box_with_buttons` ask the player a
  question with a system dialog.

The `resized` callback reports size changes, and `focus` and `visible`
report the player switching away or minimizing, which is a good moment to
pause.

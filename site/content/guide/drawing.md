+++
title = "Drawing"
description = "Shapes, colors, images, DrawParams, transforms and text."
weight = 3
+++

Everything here is in the {{ api(path="graphics/index.html", text="graphics") }}
module, the equivalent of `love.graphics`. Positions are in units from the
top-left corner of the window, with y pointing down. Everything you draw is
tinted by the current color and moved by the current transform.

A pair of numbers that belong together, such as a position or a size, is
one argument. Anything that converts into a
{{ api(path="math/struct.Vec2.html", text="Vec2") }} works: a tuple, an
array or a `Vec2` itself.

## Shapes

```rust
use smitten::graphics::{self, ArcType, DrawMode};

graphics::rectangle(DrawMode::Fill, (20.0, 20.0), (120.0, 80.0));
graphics::rounded_rectangle(DrawMode::Line, (160.0, 20.0), (120.0, 80.0), (12.0, 12.0));
graphics::circle(DrawMode::Fill, (360.0, 60.0), 40.0);
graphics::ellipse(DrawMode::Line, (480.0, 60.0), (60.0, 30.0));
graphics::arc(DrawMode::Fill, ArcType::Pie, (600.0, 60.0), 40.0, 0.0, 4.0);
graphics::polygon(DrawMode::Fill, &[(20.0, 200.0), (120.0, 160.0), (140.0, 240.0)]);
graphics::line((200.0, 160.0), (300.0, 240.0));
graphics::polyline(&[(320.0, 240.0), (360.0, 160.0), (400.0, 240.0)]);
graphics::points(&[(440.0, 200.0), (460.0, 200.0)]);
```

`DrawMode::Fill` fills a shape and `DrawMode::Line` outlines it. As in
LÖVE, a filled polygon must be convex; use
{{ api(path="math/fn.triangulate.html", text="math::triangulate") }} to split
a concave one into triangles.

{{ figure(src="shapes-example.png", alt="Rectangles, circles, ellipses, arcs and polygons, filled on the top row and outlined below.", caption="Every shape Smitten draws, from the shapes example.") }}

Lines are shaped by `set_line_width`, `set_line_join`
(`LineJoin::Miter`, `Bevel` or `None`) and `set_line_style`
(`LineStyle::Smooth` or `Rough`). `dashed_line` and `dashed_polyline`
draw dashes.

{{ figure(src="line-joins.png", alt="Thick lines drawn with miter, bevel and no joins, smooth on the left and rough on the right.", caption="Line joins and styles.") }}

## Colors

{{ api(path="struct.Color.html", text="Color") }} components go from 0 to 1,
as in LÖVE 11. `set_color` changes the color everything after it is drawn
in, until it is changed again:

```rust
use smitten::{graphics, Color};

graphics::set_color(Color::rgb(1.0, 0.6, 0.2));
graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.5));
graphics::set_color(Color::from_rgba8(255, 128, 0, 255));
graphics::set_color(Color::WHITE);
```

`Color::rgb` and `Color::rgba` are `const`, so colors can be constants.
`set_background_color` sets the color each frame starts from, and
`clear(color)` fills the screen, or the current canvas, right away.

## Images

Load an image in `load` and draw it in `draw`:

```rust
use smitten::graphics::{self, Image};

let player = Image::load("player.png")?;
graphics::draw(&player, (100.0, 200.0));
```

Images load from PNG, JPEG and TGA files, and compressed DDS and KTX files.
For pixel art, keep images crisp when they are scaled with
`graphics::set_default_filter(Filter::Nearest)` before loading, or
`image.set_filter(Filter::Nearest)` on one image.

To build an image from pixels, use `Image::from_rgba(width, height, &bytes)`,
or fill an {{ api(path="graphics/struct.ImageData.html", text="ImageData") }}
and call `Image::from_image_data`.

## Placing things with DrawParams

{{ api(path="graphics/fn.draw.html", text="graphics::draw") }} takes anything
{{ api(path="graphics/trait.Drawable.html", text="Drawable") }} (images,
canvases, sprite batches, meshes, particle systems, text and video) and
where to put it. A position is enough for most things. For more, build a
{{ api(path="graphics/struct.DrawParams.html", text="DrawParams") }}, which
replaces LÖVE's `x, y, r, sx, sy, ox, oy, kx, ky` arguments:

```rust
use smitten::graphics::{self, DrawParams};

// Spin a sprite around its own center, at twice its size.
graphics::draw(
    &player,
    DrawParams::at((x, y))
        .rotation(angle)
        .scale(2.0)
        .origin(player.size() / 2.0),
);
```

`scale_xy((sx, sy))` scales each axis separately and `shear((kx, ky))`
shears. A {{ api(path="graphics/struct.Transform.html", text="Transform") }}
can be passed instead of a `DrawParams` too.

{{ figure(src="sprites-example.png", alt="Tinted heart sprites bouncing and spinning over a floor and a sun.", caption="The sprites example: tinting, rotation around a sprite's center, and the transform stack.") }}

### Part of an image

A {{ api(path="graphics/struct.Quad.html", text="Quad") }} picks out part of
an image, for sprite sheets and tile sets. Draw it with `image.region`:

```rust
use smitten::graphics::{self, Quad};

// The second 16 by 16 tile of a 64 by 64 sheet.
let tile = Quad::new((16.0, 0.0), (16.0, 16.0), sheet.size());
graphics::draw(&sheet.region(&tile), (x, y));
```

## The transform stack

The current transform moves, turns and scales everything drawn after it.
`push` saves it and `pop` brings it back:

```rust
use smitten::graphics::{self, DrawMode};

graphics::push();
graphics::translate((400.0, 300.0));
graphics::rotate(angle);
graphics::scale(2.0, 2.0);
graphics::rectangle(DrawMode::Fill, (-10.0, -10.0), (20.0, 20.0));
graphics::pop();
```

`graphics::scoped` does the same and pops for you, even on an early
return:

```rust
graphics::scoped(|| {
    graphics::translate((camera_x, camera_y));
    draw_world();
});
```

`push_all` (or `scoped_all`) saves the color, font, blend mode and other
settings as well as the transform. `transform_point` and
`inverse_transform_point` convert between screen and local coordinates,
which is handy for turning a mouse position into world coordinates.

## Text

`print` draws text in the current font, and `printf` wraps it to a width
and aligns it:

```rust
use smitten::graphics::{self, Align, Font};

let title = Font::default_at(48.0);
graphics::set_font(&title);
graphics::print("Score: 10", (20.0, 20.0));
graphics::printf("Game over", (0.0, 280.0), 800.0, Align::Center);
```

Smitten has a built-in font, which `Font::default_at(size)` gives at any
size. Load TrueType and OpenType fonts with `Font::load(path, size)`,
bitmap fonts with `Font::load_image_font(path, glyphs)`, and BMFont files
with `Font::load_bmfont(path)`. `font.width(text)` and `font.height()`
measure text.

`print` and `printf` take a `DrawParams` as well as a position, so text can
be rotated and scaled. They also take colored parts in place of a string:

```rust
graphics::print(
    &[(Color::WHITE, "Press "), (Color::rgb(1.0, 0.8, 0.3), "Space")],
    (20.0, 60.0),
);
```

Text you draw every frame but rarely change can be laid out once in a
{{ api(path="graphics/struct.Text.html", text="Text") }} object:

```rust
use smitten::graphics::{self, Text};

let label = Text::new(&font, "Laid out once");
graphics::draw(&label, (20.0, 100.0));
```

{{ figure(src="text-example.png", alt="Text at several sizes, wrapped paragraphs in four alignments, rotated text and a bitmap font.", caption="The text example.") }}

Text and canvases are drawn as sharp as the screen allows on high-DPI
displays, so text stays crisp at 150% scaling. See
[Windows and screens](@/guide/window-and-screens.md).

## Blend modes

`set_blend_mode` decides how what you draw combines with what is already
there. `BlendMode::Alpha` is the default. `Add` brightens, which suits
glows and fire, and `Multiply` darkens, which suits light maps:

```rust
use smitten::{graphics, BlendMode};

graphics::set_blend_mode(BlendMode::Add);
draw_glow();
graphics::set_blend_mode(BlendMode::Alpha);
```

The others are `Subtract`, `Replace`, `Screen`, `Lighten`, `Darken` and
`Premultiplied`, which is the one to use when drawing a canvas. Add an
alpha mode with `mode.with_alpha(alpha)`, as LÖVE's second argument to
`setBlendMode` does.

## Screenshots

`graphics::save_screenshot(path)` saves the next frame to a PNG in the save
directory, and `capture_screenshot` hands it to a closure as
{{ api(path="graphics/struct.ImageData.html", text="ImageData") }}.

+++
title = "Smitten"
+++

<div class="hero">
<div>
<h1>Smitten</h1>
<p class="lead">A LÖVE-style 2D game framework for Rust. Write <code>load</code>, <code>update</code> and <code>draw</code>, and call <code>graphics</code>, <code>audio</code> and <code>keyboard</code> from anywhere, just as you would in LÖVE.</p>
<div class="buttons">
<a class="button primary" href="guide/getting-started/">Get started</a>
<a class="button" href="examples/">See the examples</a>
<a class="button" href="api/smitten/index.html">API reference</a>
</div>
</div>
<img src="images/lantern.png" alt="Lantern, an example game: a lantern beam lights a dark room of tiles, with glowing embers and a ghostly shade." width="800" height="600">
</div>

```rust
use smitten::{graphics, Color, Config, Game};

struct Hello;

impl Game for Hello {
    fn load() -> smitten::Result<Self> {
        Ok(Hello)
    }

    fn draw(&mut self) {
        graphics::clear(Color::rgb(0.1, 0.1, 0.2));
        graphics::print("Hello from smitten", (40.0, 40.0));
    }
}

fn main() {
    smitten::run::<Hello>(Config::default());
}
```

<div class="features">
<div><h3>Everything LÖVE has</h3><p>Every LÖVE 11.5 function that means something in Rust on the desktop has a Smitten equivalent: graphics, audio, input, physics, files, video and more.</p></div>
<div><h3>Feels like Rust</h3><p>Loaders return <code>Result</code>, points are <code>glam::Vec2</code>, drawing takes a <code>DrawParams</code> builder, and LÖVE's names are search aliases in the API docs.</p></div>
<div><h3>Built on SDL3</h3><p>Rendering uses SDL3's GPU API on Vulkan, on Windows and Linux. SDL is built from source on the first build, so <code>cargo run</code> just works.</p></div>
<div><h3>Kind when things break</h3><p>A panic shows LÖVE's blue error screen instead of a vanishing window, and saved images, shaders and sounds reload while the game runs.</p></div>
<div><h3>Fast where it counts</h3><p>Drawing is batched automatically. Ten thousand moving, turning, tinted sprites cost Smitten under a millisecond a frame.</p></div>
<div><h3>Ready to ship</h3><p><code>cargo smitten package</code> builds a release with SDL linked in and gathers your assets into a zip for players.</p></div>
</div>

## Where to go next

- [Getting started](guide/getting-started/) sets up the tools and runs your first game.
- [The guide](guide/) walks through each part of the framework.
- [Coming from LÖVE](guide/coming-from-love/) shows how LÖVE code translates, and [LÖVE name lookup](guide/love-names/) finds the Smitten name for a LÖVE function.
- [The API reference](api/smitten/index.html) documents every type and function.

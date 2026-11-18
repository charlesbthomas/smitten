# Plan for a more idiomatic Rust API

Smitten covers all of LÖVE 11.5 that makes sense in Rust (see
[love-parity.md](love-parity.md)). Getting there meant mirroring LÖVE's
names and argument lists closely, because that made each gap easy to find
and check. Now that the gaps are closed, that mirroring is the main thing
that makes the API feel like a Lua binding rather than a Rust crate.

This plan covers what to change, as of `main` at 1b34675 (after #95). It
groups the changes into workstreams, puts them in dependency order,
estimates their size and marks the decisions to make first. Sizes use the
same terms as [love-parity-plan.md](love-parity-plan.md): **S** is under
about 500 changed lines, **M** is 500 to 1,500, **L** is more than 1,500.

Nearly every workstream breaks the public API. That is called out per
workstream, and decision 1 is about how to handle it.

## What the code looks like today

These numbers come from `crates/smitten/src` unless noted.

- **1,300 public functions.** None start with `get_`, so the most common
  binding smell is already gone. 219 start with `set_`, which is fine for
  global state.
- **34 `_ex` functions** (`draw_ex`, `print_ex`, `clear_ex`, `push_ex`,
  `stencil_ex`, `SpriteBatch::add_quad_ex`, ...) and **34
  `#[allow(clippy::too_many_arguments)]`**, almost all "Mirrors
  love.graphics.draw". Drawing an image comes in up to four shapes:
  `draw`, `draw_ex`, `draw_with`, and the `_quad` versions of each. The
  same triplet is repeated for videos, sprite batches, meshes, instanced
  meshes, particles, text, array layers and `print`/`printf`.
- **26 `try_new_*` functions**, each twinned with a `new_*` that panics.
  About 50 `new_*` free functions in `graphics`, `font`, `audio`, `math`
  and `physics::World` construct types that also have (or should have)
  associated constructors. `graphics::new_sprite_batch` just calls
  `SpriteBatch::new`; `math::new_transform` just calls
  `Transform::from_transformation`.
- **Errors are mixed.** There are good typed errors (`ImageError`,
  `FontError`, `AudioError`, `ShaderError`, `VideoError`, `DataError`), but
  39 public and private functions return `Result<_, String>`
  (`Canvas::new`, `Canvas::with_msaa`, `graphics::try_set_canvases`,
  `joystick::load_gamepad_mappings_from_str`, `Image::set_depth_sample_mode`
  and the mixer), and `MeshError` wraps a `String`. There are 55 `panic!`,
  87 `.expect(` and 285 `.unwrap()` across `smitten`, `smitten-core` and
  `smitten-sdl` (tests excluded). Many are fine invariants; some are
  validation that should be an error.
- **Points are tuples.** `(f32, f32)` appears 127 times, and most
  positions are passed as separate `x, y` arguments. There is no `Vec2` or
  `Rect` type. The scissor is `(i32, i32, u32, u32)`, the color mask is
  four `bool`s, `Source::cone` is `(f32, f32, f32)`, `Joint::anchors` is
  `((f32, f32), (f32, f32))`.
- **One type, many kinds, runtime panics.** `physics::Joint` is one type
  for eleven joint kinds; its ~60 methods panic when called on the wrong
  kind. `physics::Shape` is the same (`radius` panics unless it is a
  circle). `audio::Source::queue` panics unless the source is queueable.
  `SpriteBatch::image()` returns `Option` because a batch might hold an
  `ArrayImage` instead.
- **Time is `f32`/`f64` seconds** everywhere, including where `std` uses
  `Duration` (`timer::sleep`, `Source::seek`/`tell`/`duration`).
- **LÖVE-only no-ops are public API**: `graphics::discard`,
  `graphics::flush_batch`, `SpriteBatch::flush`. There are LÖVE-order
  arguments such as `math::random_normal(deviation, mean)`, and LÖVE-style
  helpers that belong on a type, such as `math::color_from_bytes` and
  `math::gamma_to_linear`.
- **Two paths to one type.** `font::Rasterizer` and `graphics::Rasterizer`
  are the same item, as are `math::Transform` and `graphics::Transform`.
- **`graphics/mod.rs` is 3,037 lines**, holding every free function of the
  module.
- **Trait coverage is thin.** Handles like `Image` derive only `Clone`
  (no `Debug`, `PartialEq`). Only 9 `Display` impls exist; `Key` and
  `PixelFormat` have `name`/`from_name` instead of `Display`/`FromStr`.
  `#[non_exhaustive]` is used 4 times. Clippy's pedantic lints flag 38
  missing `# Errors` sections and 32 missing `#[must_use]` in
  `smitten-core` alone.
- **`unsafe` is nearly contained.** The README says `smitten-sdl` is the
  only crate with `unsafe`, but `smitten/src/lib.rs` has one block
  (`AttachConsole` on Windows). In `smitten-sdl`, `renderer.rs` has 71
  `unsafe` uses and 54 `SAFETY` comments.
- **There is no CI.** Nothing runs `cargo fmt`, `clippy`, the 471 tests or
  `cargo doc` on pull requests.

What is already idiomatic and should stay: the `Game` trait with default
methods, `Config` as a plain struct with `Default`, enums rather than
strings for modes (`DrawMode`, `BlendMode`, `FileMode`...), `io::Result`
in `filesystem`, `File` implementing `Read`/`BufRead`/`Write`/`Seek`,
`Transform` implementing `Mul`, closures for `stencil`, `CustomEvent`'s
`Any`-based downcast, and `std::thread` standing in for `love.thread`.

## Decisions to make first

1. **How much LÖVE to keep.** Options: (a) Rust-first names and shapes,
   with LÖVE names searchable through `#[doc(alias = "...")]` and a
   "coming from LÖVE" mapping page; (b) the same, plus a `smitten::compat`
   module that keeps the old shapes; (c) keep LÖVE shapes and only tidy
   errors and traits. *Recommended: (a).* The crate is `publish = false`
   at 0.0.1, so there is no one to stay compatible with except the
   examples, and `doc(alias)` keeps LÖVE users' search working
   (rustdoc finds `draw_ex` searches under `draw`, and `newImage` under
   `Image::load`). The free-function, global-context feel stays; only the
   argument shapes change.
2. **Break freely or deprecate.** *Recommended: break freely, one
   workstream per PR, with every example updated in the same PR.*
   Deprecation shims would double the surface during exactly the period
   we are trying to shrink it.
3. **Vector type.** Options: depend on `glam` and re-export its `Vec2`
   (and use `Affine2` inside `Transform`); write a small `smitten::Vec2`;
   or keep tuples. *Recommended: `glam`.* It is what Bevy, macroquad and
   most of the Rust game ecosystem use, it is fast, and it gives `Vec2`
   arithmetic, `Rect`-free helpers and `mint` interop for free. Every
   argument takes `impl Into<Vec2>`, so `(x, y)` tuples and `[x, y]`
   arrays still work at call sites.
4. **How draw arguments look.** Options: a `DrawParams` builder passed as
   `impl Into<DrawParams>`; keyword-like structs with `..Default::default()`;
   or keeping `_ex`. *Recommended: a builder that tuples and `Transform`
   convert into.* See workstream C.
5. **Fallible loading in `Game::load`.** Once loaders return `Result`,
   `load` either stays `fn load() -> Self` (games call `.expect()`) or
   becomes `fn load() -> Result<Self, smitten::Error>` so games can use
   `?`. *Recommended: return `Result`*, and show the error on the existing
   error screen. This needs a crate-level `smitten::Error` that every
   module error converts into.
6. **The global context.** It could move to an explicit `&mut Context`
   passed into every callback (as ggez does). *Recommended: keep the
   thread-local.* It is the core of the LÖVE feel, it is sound as written,
   and changing it would touch every line of every game.
7. **Physics user data.** `u64` (an index into the game's own storage) or
   `Box<dyn Any>`. *Recommended: keep `u64`*, perhaps renamed to `tag`. It
   is `Copy`, cannot fail to downcast, and fits ECS-style games.

## Workstreams

In dependency order. Each is one PR unless it says otherwise.

### A. Lints, CI and the safety boundary (S, not breaking)

Do this first so every later PR is checked.

- Add a GitHub Actions workflow running `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`
  and `cargo doc` on Linux, with SDL built from source (cache `target/`).
- Add `#![forbid(unsafe_code)]` to `smitten` and `smitten-core`. Move the
  Windows console call into `smitten-sdl` (SDL builds already link
  `kernel32`) or a `Backend` method, so the README's claim holds.
- Add `#![deny(clippy::undocumented_unsafe_blocks)]` to `smitten-sdl` and
  write the missing `SAFETY` comments in `renderer.rs`.
- Turn on `missing_docs`, `missing_debug_implementations` and a chosen
  subset of pedantic (`must_use_candidate`, `missing_errors_doc`,
  `missing_panics_doc`) in `[workspace.lints]`, and fix what they find.

### B. Typed errors and fallible constructors (M, breaking)

- Replace every public `Result<_, String>` with a typed error. Canvas
  creation gets `CanvasError` (or joins `ImageError`); canvas targets get
  `TargetError`; gamepad mappings get `MappingError`. `MeshError` becomes
  an enum.
- Add `smitten::Error`, an enum over the module errors with `From` impls,
  and `smitten::Result<T>`.
- Make loaders return `Result` and remove the panicking twins: the 26
  `try_new_*` functions take over the plain names (see workstream D for
  where they live).
- Mark error enums `#[non_exhaustive]`, and give each a `source()` where
  it wraps an `io::Error` or decoder error.
- Audit `panic!`/`expect` in public paths. Keep panics for programmer
  errors that LÖVE also raises (pushing past the transform stack limit,
  using a destroyed body), document them under `# Panics`, and turn
  input-dependent failures into errors.
- Apply decision 5 to `Game::load`.

### C. Geometry types and draw parameters (L, breaking; two PRs)

**C1, geometry.** Add `glam` (decision 3) and use `Vec2` for every
position, size, velocity and point list (`&[Vec2]` instead of
`&[(f32, f32)]`). Add small structs where tuples carry meaning:
`Rect` for the scissor and `fixtures_in_box`, `ColorMask { r, g, b, a }`,
`Cone { inner, outer, outer_volume }`, and use the existing
`smitten_core::DepthMode` in `graphics::set_depth_mode`. Back `Transform`
with `glam::Affine2` (keeping its own type so it can keep LÖVE's
shear and 4x4 `matrix()`).

**C2, drawing.** Replace the `draw`/`draw_ex`/`draw_with` triplets with
one function and a trait:

```rust
pub trait Drawable { /* sealed */ }
pub fn draw(item: &impl Drawable, params: impl Into<DrawParams>);

graphics::draw(&image, (x, y));
graphics::draw(&image, DrawParams::at(x, y).rotation(r).scale(2.0).origin(16.0, 16.0));
graphics::draw(&image, &transform);
graphics::draw(&image.region(&quad), (x, y));
graphics::draw(&array_image.layer(3), (x, y));
graphics::draw(&sprite_batch, Vec2::ZERO);
graphics::draw_instanced(&mesh, 100, (x, y));
```

`Image`, `Canvas`, `Video`, `Mesh`, `SpriteBatch`, `ParticleSystem` and
`Text` implement `Drawable`, as do the borrowed views `ImageRegion` (an
image plus a `Quad`) and `Layer` (an array image plus a layer index and
optional quad). `DrawParams` converts from a `Vec2`, a tuple, an array and
`&Transform`. This removes `draw_ex`, `draw_with`, `draw_quad*`,
`draw_video*`, `draw_mesh*`, `draw_sprite_batch*`, `draw_particles*`,
`draw_text*` and `draw_layer*`: about 25 functions and 20
`too_many_arguments` allows.

The same `DrawParams` replaces `SpriteBatch::add`/`add_ex`/`add_quad`/
`add_quad_ex`/`add_layer*`/`set*` (one `add(region, params)` and
`set(index, region, params)`), and `print`/`print_ex`/`print_with`. Text
layout takes `print(text, params)` and `printf(text, params, Wrap { limit,
align })`, and `print_colored` folds in by taking `impl Into<ColoredText>`,
which `&str` and `&[(Color, &str)]` convert into.

The other `_ex` functions get the same treatment in this PR:
`clear_ex` becomes `clear(impl Into<ClearValues>)` (a `Color`, or a
struct with optional stencil and depth); `push_ex(kind)` becomes
`push(StackType)`; `stencil_ex` becomes `stencil(StencilParams, draw)`;
`set_blend_mode_ex`/`blend_mode_ex` become a `Blend { mode, alpha }`
value; `new_font_ex` and friends take a `FontSettings`.

### D. One way to construct each type (M, breaking)

- Constructors become associated functions: `Image::load(path)`,
  `Image::from_data(&ImageData)`, `Font::load(path, size)`,
  `Font::default_sized(size)`, `Font::bmfont(path)`, `Canvas::new(w, h)`,
  `Mesh::new`, `SpriteBatch::new`, `ParticleSystem::new`,
  `Source::load(path, kind)`, `Shader::new(source)`, `Rasterizer::load`.
  Settings that pile up as `with_*` constructors (`Canvas::with_msaa`,
  `with_mipmaps`, `with_dpi_scale`) move to the existing `CanvasSettings`
  via `Canvas::with_settings(w, h, CanvasSettings { msaa: 4,
  ..Default::default() })`.
- Remove the `graphics::new_*`, `font::new_*`, `math::new_*` and
  `audio::new_*` free functions that duplicate them, each with a
  `#[doc(alias = "newImage")]` on its replacement. `World::new_body`,
  `World::new_*_joint` stay methods on `World`, which is idiomatic.
- Give each type a single public path (drop `math::Transform` and the
  `font` re-exports of `graphics` items, or make `font` the home of
  `Rasterizer`, `GlyphData` and `FontError`).

### E. Types that make wrong calls impossible (L, breaking; physics and audio PRs)

- **Joints.** One type per kind (`RevoluteJoint`, `PrismaticJoint`,
  `DistanceJoint`, ...) with only its own methods, sharing a `Joint`
  trait for what all joints have (`bodies`, `anchors`, `reaction_force`,
  `destroy`, `tag`). `World::joints()` returns `Vec<AnyJoint>`, an enum a
  game can `match`. The `new_*_joint` methods return the specific type.
- **Shapes.** The same, with `CircleShape`, `PolygonShape`, `EdgeShape`,
  `ChainShape` and a `Shape` enum.
- **Sources.** Split `QueueableSource` off `Source`, so `queue` and
  `free_buffer_count` only exist where they work.
- **SpriteBatch.** Replace `image() -> Option<&Image>` with
  `texture() -> BatchTexture<'_>`, an enum of `&Image` and `&ArrayImage`.
- Replace `Option<Option<Filter>>` in `Source::effect` with an enum
  (`EffectSend::Off`, `Unfiltered`, `Filtered(Filter)`) or a struct.

### F. Std types and Rust conventions (M, breaking)

- `Duration` for `timer::sleep`, `Source::seek`/`tell`/`duration` and
  the video equivalents. Keep `dt: f32` in `Game::update`, which is the
  conventional game-loop shape.
- `Display` and `FromStr` for `Key`, `PixelFormat` and the other enums
  that have `name`/`from_name`, replacing those methods.
- Conventional argument order and names: `random_normal(mean, std_dev)`;
  `Color::from_rgba8`/`to_rgba8` and `Color::to_linear`/`to_gamma` in
  place of the `math::color_*` and `gamma_*` free functions.
- Common traits on every public type: `Debug` everywhere; `PartialEq`,
  `Eq` and `Hash` by identity on the `Rc` handles (`Image`, `Font`,
  `Shader`, `Canvas`), as the physics handles already do; `Default` where
  there is an obvious default; `Copy` on small value types; `#[must_use]`
  on getters and builders.
- `#[non_exhaustive]` on enums likely to grow: `Key`, `event::Event`,
  `GraphicsFeature`, the error enums.
- RAII guards for paired calls: `graphics::push()` returns a
  `#[must_use]` guard that pops on drop, alongside a closure form
  `graphics::scoped(|| ...)`. `set_canvas` gets the same closure form
  (`graphics::with_canvas(&canvas, || ...)`). The bare `pop` stays for
  code that needs to pop in another function.

### G. Callbacks (S, breaking)

Callbacks with more than three positional arguments take a small struct
instead: `touch_pressed(&mut self, touch: Touch)` with `id`, `position`,
`delta` and `pressure`; `mouse_pressed(&mut self, press: MouseClick)`
with `position`, `button` and `presses`. The `event::Event` variants
reuse the same structs, so a game can forward from either. The rest of
the `Game` trait stays as it is.

### H. Remove LÖVE-only leftovers (S, breaking)

- Delete the no-ops `graphics::discard`, `graphics::flush_batch` and
  `SpriteBatch::flush`. Move their explanations to the LÖVE mapping page.
- Delete thin wrappers left over from parity, such as
  `graphics::set_new_font` and `set_new_default_font` (which are
  `set_font(&Font::load(...))`).
- Update [love-parity.md](love-parity.md) so it maps LÖVE names to the
  new API instead of listing gaps.

### I. Layout and docs (M, not breaking)

- Split `graphics/mod.rs` into private submodules by topic (targets,
  state, shapes, images, text, capabilities), re-exported from
  `graphics`, so no file holds the whole module.
- Make `smitten_core::backend` `#[doc(hidden)]` or move the types games
  use (`CompressedFormat`, `VertexDataType`, `RendererInfo`...) out of it,
  so games never see the `Backend` trait.
- Rewrite item docs to lead with what the item does in Rust terms. The
  216 "the equivalent of `love.*`" sentences become `#[doc(alias)]`
  attributes plus one "Coming from LÖVE" guide in `docs/`, which can
  include the name table from love-parity.md.
- Replace `docs/api-sketch.md`, which predates most of the API, with that
  guide, and update the README example and status.

## Suggested order

1. **A** (lints, CI). Unblocks everything and is not breaking.
2. **B** (errors). Changes the most signatures that later PRs build on.
3. **C1** then **C2** (geometry, then drawing). The largest change and the
   most visible improvement to game code.
4. **D** (constructors) and **F** (std types) in either order.
5. **E** (typed joints, shapes and sources), physics then audio.
6. **G** and **H** (callbacks, leftovers).
7. **I** (layout, docs) last, so docs describe the final API.

Roughly 9 to 11 PRs. The examples are the test of each: every PR updates
them, and Pong and Topple should read noticeably shorter after C2.

## Before and after

```rust
// Today
let player = graphics::new_image("player.png");
let font = graphics::try_new_font_ex("font.ttf", 24.0, Hinting::Light, 1.0)?;
graphics::draw_quad_ex(&sheet, &frame, x, y, angle, 2.0, 2.0, 8.0, 8.0);
graphics::print_ex("Score", 10.0, 10.0, 0.0, 2.0, 2.0, 0.0, 0.0);
graphics::set_color_mask(true, true, true, false);
if let Some((x, y, w, h)) = graphics::scissor() { /* ... */ }

// After
let player = Image::load("player.png")?;
let font = Font::load_with("font.ttf", 24.0, FontSettings { hinting: Hinting::Light, ..Default::default() })?;
graphics::draw(&sheet.region(&frame), DrawParams::at(x, y).rotation(angle).scale(2.0).origin(8.0, 8.0));
graphics::print("Score", DrawParams::at(10.0, 10.0).scale(2.0));
graphics::set_color_mask(ColorMask { a: false, ..ColorMask::ALL });
if let Some(rect) = graphics::scissor() { /* ... */ }
```

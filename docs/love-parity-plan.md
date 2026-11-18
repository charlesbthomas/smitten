# Plan for the rest of LÖVE parity

This plan covers everything still listed in [love-parity.md](love-parity.md)
as of `main` at 6efed54, except what that file lists as left out on
purpose. It groups the gaps into workstreams, puts them in dependency
order, estimates their size, and marks the decisions to make first.

Sizes are in the terms of recent PRs. **S** is under about 500 changed
lines, like #63 (stencil actions). **M** is 500 to 1,500, like #61
(ParticleSystem). **L** is more than 1,500, like #66 (physics).

## How the renderer works today

These facts shape most of the plan.

- **There is one vertex type.** `Vertex` in `smitten-core/src/backend.rs`
  is a 2D position, a UV and a color. Every draw writes into one shared
  vertex stream per frame, which `renderer.rs` uploads once and draws as
  runs of `DrawCall`s, pass by pass.
- **Transforms are applied on the CPU.** `graphics::draw_*` and
  `push_mesh` transform vertices into screen pixels before they reach the
  backend. The built-in vertex shader, `shaders/sprite.vert` (GLSL compiled
  ahead of time to SPIR-V), only maps pixels to clip space with a single
  `pixel_to_clip` uniform. Meshes' vertex maps are expanded on the CPU too.
- **Custom shaders are fragment-only.** `smitten-sdl/src/shader.rs` wraps a
  game's WGSL `effect` function in a fixed entry point with fixed bindings,
  and naga translates it to SPIR-V.
- **Every texture and render target is RGBA8.** `TEXTURE_FORMAT` in
  `renderer.rs` is a constant, and `read_texture` assumes four bytes a
  pixel. The swapchain uses `SDL_GPU_SWAPCHAINCOMPOSITION_SDR`.
- **A pass draws to one target.** `Pass` has a single `target`.
- **A depth buffer already exists.** Render targets get a
  `D24_UNORM_S8_UINT` (or `D32_FLOAT_S8_UINT`) texture for the stencil, so
  the depth half is there but never tested or written.
- **`PipelineKey` is the meeting point.** Shader, blend, stencil, color
  mask, wireframe, samples and cull all live in it. Almost every
  workstream below adds a field to it.
- **SDL's GPU API has no border-color address mode.** Its samplers offer
  clamp to edge, repeat and mirrored repeat only, so `clampzero` cannot map
  straight onto the hardware.

The big consequence is that vertex shaders, custom vertex formats,
instancing and depth all need geometry to reach the GPU untransformed,
with its own layout. That is the one large refactor in this plan.

## Decisions to make first

1. **Where transforms run.** Either move every draw to GPU-side
   transforms, or keep the current CPU path for ordinary draws and add a
   GPU-transformed path only for meshes with custom formats, custom vertex
   shaders and instanced draws. *Recommended: add a second path.* The
   batched CPU path is what makes sprites and shapes cheap, and it already
   works.
2. **How a WGSL vertex shader looks.** LÖVE's is
   `vec4 position(mat4 transform_projection, vec4 vertex_position)`.
   *Recommended:* `fn position(transform_projection: mat4x4<f32>,
   vertex_position: vec4<f32>) -> vec4<f32>`. Extra attributes are named in
   a game-declared `struct Attributes`, matched to mesh attribute names, and
   values passed to `effect` go in a `struct Varyings`. The shader.rs
   wrapper generates the bindings, as it does for `Params` today.
3. **How a shader writes to several canvases.** LÖVE uses
   `void effect()` with `love_Canvases[i]`. *Recommended:* a second entry
   point name, `fn effects(...) -> Outputs`, where `Outputs` is a
   game-declared struct with one field per canvas. `effect` stays as it is
   for the single-canvas case.
4. **What to do about `clampzero`.** The choices are to emulate it in the
   built-in and wrapped shaders, or to report it unsupported through
   `get_supported` and fall back to clamp, as LÖVE does on hardware without
   it. *Recommended: report it unsupported and fall back.* Emulating it
   adds a branch to every texture sample, for a mode few games use.
5. **The shape of a custom main loop.** LÖVE lets a game replace
   `love.run`. *Recommended:* a `smitten::run_loop(config, |frame| …)`
   function. It hands the game a value with `poll`, `step`, `clear`,
   `present` and `is_active`, built from `event::poll`, `timer::step` and
   the existing frame code. `Game` stays the normal way in.
6. **The `love.conf` fields that only matter on mobile or for Lua.**
   `t.externalstorage`, `t.accelerometerjoystick` and `t.audio.mic` only do
   anything on Android and iOS, which smitten doesn't run on. `t.modules.*`
   overlaps with the Cargo features that already gate physics and video.
   *Recommended:* add `t.appendidentity` and `t.console`, map
   `t.modules.*` to the existing features, and move the mobile-only fields
   to the "left out on purpose" table until there is a mobile backend.
7. **Ghost vertices.** Box2D's ghost vertices have no direct Rapier
   equivalent that I could confirm, so this needs a short spike.
   *Recommended:* first check whether parry's internal-edge handling covers
   polylines in the Rapier version in use. If it doesn't, store each
   edge's neighbors and reject or fix contact normals in the existing
   contact-modification hook (`modify_solver_contacts`, which pre-solve
   already uses).

## Workstreams

### 0. Pipeline and target refactor (S, first)

This changes no behavior. Split `PipelineKey` into a target description
(color formats, sample count, depth and stencil format) and a vertex
layout id, and give `Texture` a format field in place of the
`TEXTURE_FORMAT` constant. Workstreams A, E, F, G, H and J all touch these
two structs, so landing this first turns most of their conflicts into
additions.

### A. GPU-transformed draws (L, after 0)

This is the foundation for B, C, D and E.

- A second kind of draw in `DrawCall`: geometry in its own GPU vertex
  buffer (and optional index buffer), owned by the `Mesh`, with a layout
  id and a per-draw transform and projection uniform.
- A built-in WGSL or GLSL vertex shader for that path, which applies the
  transform on the GPU.
- `Mesh` uploads its vertices when they change, not every frame, which
  gives `Mesh:flush` real meaning.
- Ordinary draws keep using the shared stream.

### B. Custom vertex formats and Mesh attributes (M, after A)

- `newMesh` with a vertex format (name, type, component count per
  attribute), and `Mesh:getVertexFormat`.
- `Mesh:getVertexAttribute`, `setVertexAttribute`.
- `Mesh:attachAttribute`, `detachAttribute`, `setAttributeEnabled`,
  `isAttributeEnabled`. Attaching binds another mesh's buffer as a second
  vertex buffer.
- `IndexDataType` `uint16` for vertex maps (a real index buffer now).
- `SpriteBatch:attachAttribute` (S, after this).

### C. Vertex shaders (L, after A, alongside B)

- Decision 2 above.
- shader.rs generates a vertex entry point when the source has a
  `position` function, and the built-in one otherwise.
- `Attributes` and `Varyings` structs, matched against the mesh's vertex
  format when the pipeline is built, with an error naming any missing
  attribute.
- Vertex-stage uniforms: `Params` shared between the stages, or a
  separate binding for the vertex stage.
- `Shader:getWarnings`. naga mostly reports errors, not warnings, so this
  may honestly return an empty string. Decide in the PR.

### D. Instancing (M, after B and C)

- `graphics::draw_instanced(mesh, count, …)`.
- Per-instance attributes (`VertexAttributeStep` `perinstance`), through
  `attachAttribute` with a step.
- An instance index available to vertex shaders (LÖVE's
  `love_InstanceID`).

### E. Depth testing (M, after A; depth canvases after F)

- `setDepthMode`, `getDepthMode`, with `CompareMode` and a write flag in
  the pipeline key's depth-stencil state.
- Depth on the screen (`t.window.depth`), and on canvases.
- 3-component positions, which come from B's custom formats.
- Depth canvas formats and `Texture:setDepthSampleMode`,
  `getDepthSampleMode` (comparison samplers for shadow maps) once F
  exists.

### F. Texture and canvas formats (M to L, after 0, alongside A)

- A `PixelFormat` enum in smitten-core covering the formats SDL's GPU API
  can create and sample (r8, rg8, rgba8, rgba16f, rgba32f, r16f, r32f,
  rgb10a2, rg11b10f and the depth and stencil formats).
- `newCanvas` with `format` and `readable`.
- `read_texture` and `Canvas:newImageData` for each format.
- `ImageData` in other formats, and `ImageData:getFormat`. This part is
  CPU-only, so it **can start at once**, before 0.
- `Texture:getFormat`, `getTextureType`, `getDepth`, `isReadable`.
- `getCanvasFormats`, `getImageFormats`, `getTextureTypes`, asked of the
  device with `SDL_GPUTextureSupportsFormat`.

### G. Gamma-correct rendering (M, after F)

- `t.gammacorrect`. sRGB formats for images and canvases, and an sRGB
  swapchain (`SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR`), when it's set.
- The `linear` setting on `newImage`, `Image:isFormatLinear`.
- `is_gamma_correct` reports the real setting.
- Colors set through `set_color` converted the way LÖVE converts them.

### H. Several canvases and layered targets (M, after F)

- `setCanvas` with several canvases, and decision 3 for shaders.
- `Pass` gets a list of targets, and the pipeline key gets one format per
  target.
- Drawing to one layer of an array canvas, one face of a cube canvas, one
  mipmap level, or one slice of a volume canvas.
- `newCanvas` with `type` array, cube and volume (volume after I).

### I. Volume textures (S to M, can start now)

- `TextureKind::Volume`, `newVolumeImage` from a list of `ImageData`
  slices, and `texture_3d<f32>` in custom shaders through the existing
  declared-texture mechanism in shader.rs.
- LÖVE can't draw a volume image directly either, only sample it, so no
  draw path is needed.

### J. `clampzero` (S, after 0)

- Decision 4. With the recommended option, this means adding the enum
  value, falling back to clamp, and having `get_supported` report it as
  unsupported.

### K. Capability queries (S, last)

- `getSupported` (`GraphicsFeature`) and `getSystemLimits`
  (`GraphicsLimit`).
- These come last because their answers depend on what D, E, F, H, I and
  J add. A first version can land early and grow with each workstream.

### L. Custom main loop (M, can start now)

- Decision 5.
- `graphics::present`, `is_active`, `flush_batch`, and a public
  `timer::step`.

### M. Small independent items (S each, can start now)

- `SpriteBatch:addLayer`, `setLayer`. `Batch::push_layer` already draws
  layers of array textures, so this is mostly API.
- `Video:setSource`, `Video:getStream`. `Video` already owns an
  `Option<Source>` and follows it for sync.
- `Contact:resetFriction`, `resetRestitution`.

### N. Ghost vertices (M, can start now)

- Decision 7, then `ChainShape` and `EdgeShape` `setNextVertex`,
  `setPreviousVertex`, `getNextVertex`, `getPreviousVertex`.

### O. `love.conf` leftovers (S, can start now)

- Decision 6, then `t.appendidentity`, `t.console`, and `t.modules.*`.

## Order and parallel tracks

```
Track 1, geometry:  0 ──► A ──► B ──► D
                              └─► C ──┘
                              └─► E (depth canvases wait for F)

Track 2, formats:   F (ImageData part now) ──► 0 ──► F ──► G
                                                    └─► H (volume canvases wait for I)

Track 3, anytime:   I, J (after 0), L, M, N, O

Last:               K, then update love-parity.md
```

- **Can run in parallel from the start:** workstream 0, the ImageData
  half of F, I, L, M, N and O. None of them touch the same code, apart from
  I and C both touching shader.rs's texture declarations, which is a small
  overlap.
- **Can run in parallel after 0:** A and the GPU half of F. They both add
  to `PipelineKey` and `Texture`, which 0 prepares for.
- **Can run in parallel after A:** B, C and E.
- **Must be serial:** D needs both B and C. G and H need F. K is last.

Rough total: one L refactor (A), one more L (C), about seven M and a
handful of S PRs. That is about 15 to 18 PRs, comparable to the #52 to #70
round.

## After this

When these land, the only gaps left are the ones in love-parity.md's "left
out on purpose" table: KTX, ASTC and ETC textures, `drawLayer` with a
custom shader, recording bit depth, macOS and web, and the Lua-only and
Rust-covered APIs.

# LÖVE parity: what is left

This is every part of the LÖVE 11.5 API that smitten does not have yet, as
of `main` at commit 5998012 (after #52 to #70). It was made by matching the
function list in [love-api](https://github.com/love2d-community/love-api)
(900 module functions and methods, plus callbacks, enums and `love.conf`
fields) against smitten's public API by hand.

Names are LÖVE's, so each item can be looked up on the LÖVE wiki. Items
covered by a differently shaped smitten API are counted as done and not
listed. For example, `beginContact` is covered by `World::contacts`,
`love.thread.newThread` by `std::thread::spawn`, and `newDataView` by
slicing a `ByteData`.

## Summary

About 45 of LÖVE's 900 functions and methods are still missing, plus 9
that only make sense in Lua. That is about **95% of the full API**, and
everything a typical 2D game uses. What is left is almost all lower-level
GPU features: depth buffers, instancing, custom vertex formats, vertex
shaders, volume textures and canvas formats.

| Module | LÖVE functions and methods | Missing | Coverage |
|---|---|---|---|
| audio | 76 | 1 (on purpose) | 99% |
| data | 11 | 0 | 100% |
| event | 6 | 0 | 100% |
| filesystem | 48 | 9 Lua-only | 100% of what applies |
| font | 5, plus Rasterizer and GlyphData | 0 | 100% |
| graphics | 268 | 32 | 88% |
| image | 17 | 1 | 94% |
| joystick | 26 | 0 | 100% |
| keyboard | 9 | 0 | 100% |
| math | 50 | 0 | 100% |
| mouse | 19 | 0 | 100% |
| physics | 272 | 10 | 96% |
| sound | 16 | 0 | 100% |
| system | 8 | 0 | 100% |
| thread | 16 | 0 (Rust threads plus `Channel`) | 100% |
| timer | 6 | 1 | 83% |
| touch | 3 | 0 | 100% |
| video | 8 | 0 | 100% |
| window | 36 | 0 | 100% |
| callbacks | 35 | 1 | 97% |

## graphics

### Module functions

- `getSupported` (`GraphicsFeature`), `getSystemLimits` (`GraphicsLimit`),
  `getCanvasFormats`, `getImageFormats`, `getTextureTypes`.
  `renderer_info` exists.
- `setDepthMode`, `getDepthMode`, and depth and stencil buffers on canvases
  (`t.window.depth`, `t.window.stencil`, depth canvas formats).
- `drawInstanced`, with per-instance vertex attributes
  (`VertexAttributeStep` `perinstance`).
- `newVolumeImage` (`TextureType` `volume`).
- `setCanvas` with several canvases at once (multiple render targets), and
  with a layer, cube face or mipmap of a canvas as the target.
- `present`, `isActive`, `flushBatch`. These only matter with a custom
  `love.run`.
- Gamma-correct rendering (`t.gammacorrect`). `is_gamma_correct` exists and
  always answers `false`.

### Enums

- `WrapMode` `clampzero`.

### Canvas and Texture

- `newCanvas` settings: `format` (float formats and the rest of
  `PixelFormat`), `readable`, and `type` (array, cube and volume canvases).
  MSAA is done.
- `Texture:getFormat`, `getTextureType`, `getDepth`, `isReadable`,
  `getDepthSampleMode`, `setDepthSampleMode`.
- `Image:isFormatLinear`, and the `linear` setting in `newImage`.

### Mesh

- Custom vertex formats in `newMesh`, and `Mesh:getVertexFormat`.
- `Mesh:attachAttribute`, `detachAttribute`, `setAttributeEnabled`,
  `isAttributeEnabled`.
- `Mesh:getVertexAttribute`, `setVertexAttribute`.
- `IndexDataType` `uint16` for vertex maps.

### Shader

- A vertex stage (LÖVE's `position` function). Smitten shaders are
  fragment-only.
- `Shader:getWarnings`. `validate_shader` exists.

### SpriteBatch

- `SpriteBatch:addLayer`, `setLayer`: batches of an `ArrayImage`.
- `SpriteBatch:attachAttribute`.

### Video

- `Video:getStream`, `Video:setSource`.

## physics

- Ghost vertices on chains and edges: `ChainShape` and `EdgeShape`
  `setNextVertex`, `setPreviousVertex`, `getNextVertex`,
  `getPreviousVertex`. These stop bodies catching on the joins between
  edges.
- `Contact:resetFriction`, `resetRestitution`. They only matter if a change
  made in pre-solve should outlast the step.

## timer

- `love.timer.step` as a public function. It only matters with a custom
  run loop.

## Callbacks and `love.conf`

- `love.run`: a custom main loop.
- `love.conf` fields with no `Config` equivalent: `t.gammacorrect`,
  `t.window.depth`, `t.window.stencil`.

## Left out on purpose

| Item | Why |
|---|---|
| KTX, ASTC and ETC textures | Only DDS files load. BC6H and PVRTC are missing too (`CompressedFormat` has BC1 to BC5 and BC7). |
| `drawLayer` with a custom shader | The array-texture pipeline has its own shader. |
| `RecordingDevice:getBitDepth` | Recorded samples are floats. |
| macOS and web | The renderer is Vulkan only. |
| Lua-only filesystem functions | `load`, `getRequirePath`, `setRequirePath`, `getCRequirePath`, `setCRequirePath`, `init`, `setSource` and the like have no Rust meaning. |
| `love.threaderror`, the `Thread` type | Rust threads report panics through `JoinHandle`. |
| `Object`, `Data` base types | Rust ownership and slices do their job. |
| `setDeprecationOutput`, `hasDeprecationOutput` | Smitten has no deprecated LÖVE names to warn about. |
| LÖVE's GLSL shader dialect | Smitten shaders are WGSL. |
| `t.externalstorage`, `t.accelerometerjoystick`, `t.audio.mic` | They only do anything on Android and iOS, which Smitten doesn't run on. |
| `t.modules.*` | The `physics` and `video` Cargo features leave those modules out; the rest are always built in. |

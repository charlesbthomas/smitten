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

About 34 of LÖVE's 900 functions and methods are still missing, plus 9
that only make sense in Lua. That is about **95% of the full API**, and
everything a typical 2D game uses. What is left is almost all lower-level
GPU features: depth buffers, instancing, custom vertex formats, vertex
shaders and canvas formats.

| Module | LÖVE functions and methods | Missing | Coverage |
|---|---|---|---|
| audio | 76 | 1 (on purpose) | 99% |
| data | 11 | 0 | 100% |
| event | 6 | 0 | 100% |
| filesystem | 48 | 9 Lua-only | 100% of what applies |
| font | 5, plus Rasterizer and GlyphData | 0 | 100% |
| graphics | 268 | 24 | 91% |
| image | 17 | 0 | 100% |
| joystick | 26 | 0 | 100% |
| keyboard | 9 | 0 | 100% |
| math | 50 | 0 | 100% |
| mouse | 19 | 0 | 100% |
| physics | 272 | 8 | 97% |
| sound | 16 | 0 | 100% |
| system | 8 | 0 | 100% |
| thread | 16 | 0 (Rust threads plus `Channel`) | 100% |
| timer | 6 | 0 | 100% |
| touch | 3 | 0 | 100% |
| video | 8 | 0 | 100% |
| window | 36 | 0 | 100% |
| callbacks | 35 | 0 | 100% |

## graphics

### Module functions

- `getSystemLimits` (`GraphicsLimit`), `getTextureTypes`. `renderer_info`,
  `is_supported` (`GraphicsFeature`, a first version) and the format lists
  exist.
- `setDepthMode`, `getDepthMode`, and depth and stencil buffers on canvases
  (`t.window.depth`, `t.window.stencil`, depth canvas formats).
- `drawInstanced`, with per-instance vertex attributes
  (`VertexAttributeStep` `perinstance`).
- `setCanvas` with several canvases at once (multiple render targets), and
  with a layer, cube face or mipmap of a canvas as the target.

### Canvas and Texture

- `newCanvas` settings: depth formats, and `type` (array, cube and volume
  canvases). Color formats, `readable` and MSAA are done.
- `Texture:getTextureType`, `getDepth`, `getDepthSampleMode`,
  `setDepthSampleMode`.

### Shader

- A vertex stage (LÖVE's `position` function). Smitten shaders are
  fragment-only.
- `Shader:getWarnings`. `validate_shader` exists.

### SpriteBatch

- `SpriteBatch:attachAttribute`.

## physics

- Ghost vertices on chains and edges: `ChainShape` and `EdgeShape`
  `setNextVertex`, `setPreviousVertex`, `getNextVertex`,
  `getPreviousVertex`. These stop bodies catching on the joins between
  edges.

## Callbacks and `love.conf`

- `love.conf` fields with no `Config` equivalent: `t.window.depth`,
  `t.window.stencil`.

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

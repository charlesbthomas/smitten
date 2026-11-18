# LÖVE parity: what is left

This compares the LÖVE 11.5 API with smitten's, as of `main` after #94.
It was first made at commit 5998012 (after #52 to #70) by matching the
function list in [love-api](https://github.com/love2d-community/love-api)
(900 module functions and methods, plus callbacks, enums and `love.conf`
fields) against smitten's public API by hand, and has been kept up to
date as the gaps closed. [love-parity-plan.md](love-parity-plan.md) is
the plan they were closed by.

Names are LÖVE's, so each item can be looked up on the LÖVE wiki.
[coming-from-love.md](coming-from-love.md) shows how LÖVE calls
translate into smitten's API, and each LÖVE name is a search alias in
the API docs. Items
covered by a differently shaped smitten API are counted as done and not
listed. For example, `beginContact` is covered by `World::contacts`,
`love.thread.newThread` by `std::thread::spawn`, `newDataView` by
slicing a `ByteData`, and `setCanvas{canvas, depthstencil = depth}` by
`graphics::set_canvas_with_depth`.

## Summary

Every LÖVE function, method, callback and `love.conf` field that has a
meaning in Rust on the desktop is covered. What is left is the table
below: things left out on purpose, almost all because they only make
sense in Lua or on platforms smitten doesn't run on.

| Module | LÖVE functions and methods | Missing | Coverage |
|---|---|---|---|
| audio | 76 | 0 | 100% |
| data | 11 | 0 | 100% |
| event | 6 | 0 | 100% |
| filesystem | 48 | 9 Lua-only | 100% of what applies |
| font | 5, plus Rasterizer and GlyphData | 0 | 100% |
| graphics | 268 | 0 | 100% |
| image | 17 | 0 | 100% |
| joystick | 26 | 0 | 100% |
| keyboard | 9 | 0 | 100% |
| math | 50 | 0 | 100% |
| mouse | 19 | 0 | 100% |
| physics | 272 | 0 | 100% |
| sound | 16 | 0 | 100% |
| system | 8 | 0 | 100% |
| thread | 16 | 0 (Rust threads plus `Channel`) | 100% |
| timer | 6 | 0 | 100% |
| touch | 3 | 0 | 100% |
| video | 8 | 0 | 100% |
| window | 36 | 0 | 100% |
| callbacks | 35 | 0 | 100% |

The last gaps were closed by #90 (depth canvases and
`setDepthSampleMode`), #91 (`SpriteBatch:attachAttribute`), #92
(bitmap-font glyph pages under gamma-correct rendering), #93
(`t.window.depth`, `t.window.stencil` and `getBitDepth`) and #94 (BC6H,
ASTC and KTX textures).

## Left out on purpose

| Item | Why |
|---|---|
| ETC and PVRTC textures | SDL's GPU API has no ETC or PVRTC formats, so such DDS and KTX files are refused with that reason. KTX 2 files are refused too; KTX 1 files load. |
| `drawLayer` with a custom shader | A custom shader's `image` is a `texture_2d<f32>`. Send the `ArrayImage` to the shader with `Shader::send_texture` and pick the layer there instead. |
| macOS and web | The renderer is Vulkan only. |
| Lua-only filesystem functions | `load`, `getRequirePath`, `setRequirePath`, `getCRequirePath`, `setCRequirePath`, `init`, `setSource` and the like have no Rust meaning. |
| `love.threaderror`, the `Thread` type | Rust threads report panics through `JoinHandle`. |
| `Object`, `Data` base types | Rust ownership and slices do their job. |
| `setDeprecationOutput`, `hasDeprecationOutput` | Smitten has no deprecated LÖVE names to warn about. |
| LÖVE's GLSL shader dialect | Smitten shaders are WGSL. |
| `t.externalstorage`, `t.accelerometerjoystick`, `t.audio.mic` | They only do anything on Android and iOS, which Smitten doesn't run on. |
| `t.modules.*` | The `physics` and `video` Cargo features leave those modules out; the rest are always built in. |
| `love.graphics.discard`, `flushBatch`, `SpriteBatch:flush` | Smitten's targets always keep their contents and a frame is sent to the GPU at once, so there is nothing for them to do. |
| `setNewFont` | `set_font(&Font::load(path, size)?)`. |

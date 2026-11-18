# LÖVE parity: what is left

This is every part of the LÖVE 11.5 API that smitten does not have yet, as
of `main` at commit 999426c (after #41 to #50). It was made by matching the
function list in [love-api](https://github.com/love2d-community/love-api)
(900 module functions and methods, plus callbacks and enums) against
smitten's public API by hand.

Names are LÖVE's, so each item can be looked up on the LÖVE wiki. An item
marked **★** is one that ordinary 2D games use. Items that are covered by a
differently shaped smitten API (for example, `beginContact` is covered by
`World::contacts`) are counted as done and not listed.

The last section lists what is left out on purpose and why. Those items are
not needed for 100% of what smitten aims at, but they are listed so that the
gap to LÖVE's full API is complete.

## Summary

| Module | LÖVE functions and methods | Missing | Coverage |
|---|---|---|---|
| audio | 76 | 9 | 88% |
| data | 11 | 11 | 0% |
| event | 6 | 3 (on purpose) | 50% |
| filesystem | 48 | 14, plus 9 Lua-only | 52% (79% without Lua-only) |
| font | 5, plus Rasterizer and GlyphData | all | ~5% |
| graphics | 268 | about 95 | ~65% |
| image | 17 | 4 | 76% |
| joystick | 26 | 1 | 96% |
| keyboard | 9 | 0 functions, 61 key constants | ~95% |
| math | 50 | 7 | 86% |
| mouse | 19 | 0 | 100% |
| physics | 272 | about 70, 15 of them on purpose | ~74% |
| sound | 16 | 3 | 81% |
| system | 8 | 1 | 88% |
| thread | 16 | 16 (Rust threads instead) | 0% |
| timer | 6 | 1 (needs a custom run loop) | 83% |
| touch | 3 | 0 | 100% |
| video | 8 | 1 | 88% |
| window | 36 | 11 | 69% |
| callbacks | 35 | 3 | 91% |

Graphics and physics hold most of what is left. Graphics is mostly the
lower-level GPU features (MSAA, depth, custom vertex formats, instancing,
vertex shaders) plus ParticleSystem.

## Biggest items for a typical 2D game

1. **★ `love.graphics.push("all")`**. `graphics::push` saves only the
   transform. LÖVE's `"all"` also saves color, background color, font,
   blend mode, shader, canvas, scissor, stencil test, line width and
   style, point size, color mask, wireframe and default filter.
2. **★ Keypad and other keys.** `Key` has 83 of LÖVE's 144 key constants.
   The keypad (`kp0` to `kp9`, `kp.`, `kp/`, `kp*`, `kp-`, `kp+`,
   `kpenter`, `kp=`) is the one games hit most.
3. **★ ParticleSystem.** 24 of 60 methods. See the graphics section.
4. **★ Window mode flags.** No borderless, minimum size, MSAA, display
   choice, fullscreen type or `setMode` at run time.
5. **★ Blend modes.** No `screen`, `lighten` or `darken`, and no
   `BlendAlphaMode` (alpha-multiply versus premultiplied for the other
   modes).
6. **★ Font fallbacks** (`Font:setFallbacks`) and `Font:hasGlyphs`.
7. **★ `printf` with `justify`** alignment.
8. **★ Shader uniforms**: matrices, booleans and arrays cannot be sent.

## audio

- `love.audio.play(sources...)`: start several sources at once. `pause`
  and `stop` exist.
- `love.audio.isEffectsSupported`, `getMaxSceneEffects`,
  `getMaxSourceEffects`.
- `love.audio.setMixWithSystem` (iOS and Android only).
- `Source:getEffect`: read back the filter set on a source's effect.
- `Source:setAirAbsorption`, `Source:getAirAbsorption`.
- `Source:seek` and `Source:tell` in samples (`TimeUnit` `samples`).
  Seconds work.
- `RecordingDevice:getBitDepth`, and choosing 8 or 16 bit when recording.
  Smitten records floats.

## data

The whole module is missing. Rust crates cover most of it, but a LÖVE port
reaches for these directly.

- `love.data.compress`, `decompress` (lz4, zlib, gzip, deflate) and
  `CompressedData:getFormat`.
- `love.data.encode`, `decode` (base64, hex).
- `love.data.hash` (md5, sha1, sha224, sha256, sha384, sha512).
- `love.data.pack`, `unpack`, `getPackedSize` (Lua's `string.pack`
  formats).
- `love.data.newByteData`, `newDataView`, and the `ByteData` type.

## event

- `love.event.poll`, `wait`, `pump`. Left out on purpose; see the last
  section.

## filesystem

- `love.filesystem.getAppdataDirectory`, `getUserDirectory`,
  `getWorkingDirectory`.
- `love.filesystem.getSource`, `getSourceBaseDirectory`, `isFused`.
- `love.filesystem.getRealDirectory`: which mounted directory or archive a
  file comes from.
- `love.filesystem.areSymlinksEnabled`, `setSymlinksEnabled`, and
  `FileType` `symlink` in `getInfo`.
- `love.filesystem.newFileData`, and `FileData:getExtension`,
  `FileData:getFilename`. Smitten passes `&[u8]` and paths instead, so this
  may only need a small wrapper.
- `File:open`, `File:close`, `File:isOpen`: reopening a closed `File`.
  Smitten opens on creation and closes on drop.
- `File:setBuffer`, `File:getBuffer` (`BufferMode` none, line, full).

Lua-only, not applicable: `load`, `getRequirePath`, `setRequirePath`,
`getCRequirePath`, `setCRequirePath`, `init`, `setSource`. `File:lines`,
`File:read`, `File:write`, `File:seek` and `File:flush` are covered by
`BufRead`, `Read`, `Write` and `Seek`.

## font

- `love.font.newRasterizer`, `newTrueTypeRasterizer`, `newImageRasterizer`,
  `newBMFontRasterizer`, `newGlyphData`, and the `Rasterizer` and
  `GlyphData` types.
- ★ BMFont (`.fnt`) fonts. `newFont` takes TrueType and image fonts only.
- TrueType hinting modes (`normal`, `light`, `mono`, `none`) in `newFont`.
- `newImageFont`'s `extraspacing` argument.

## graphics

### Module functions

- ★ `push("all")` (`StackType` `all`). See above.
- `getStackDepth`.
- `reset`: put all graphics state back to its defaults.
- ★ `intersectScissor`.
- ★ `setNewFont`.
- `clear` with several colors (one per canvas), and with stencil and depth
  values, or `clear(false, …)` to skip parts.
- `discard`.
- `present`, `isActive`, `flushBatch` (these only matter with a custom
  run loop).
- `getRendererInfo`, `getSupported` (`GraphicsFeature`), `getSystemLimits`
  (`GraphicsLimit`), `getCanvasFormats`, `getImageFormats`,
  `getTextureTypes`.
- `isGammaCorrect`, and gamma-correct rendering (`t.gammacorrect`).
- `setDepthMode`, `getDepthMode`.
- `setMeshCullMode`, `getMeshCullMode` (`CullMode`).
- `setFrontFaceWinding`, `getFrontFaceWinding` (`VertexWinding`).
- `drawInstanced`.
- `newVolumeImage` (3D textures, `TextureType` `volume`).
- `validateShader`.
- `setCanvas` with several canvases at once (multiple render targets), with
  a depth or stencil canvas, and with a layer or mipmap of an array, cube
  or volume canvas.

### Enums

- ★ `BlendMode` `screen`, `lighten`, `darken`, and `BlendAlphaMode`.
- ★ `AlignMode` `justify`.
- `StencilAction` `incrementwrap`, `decrementwrap`.
- `CompareMode` `never`, `always`.
- `MeshDrawMode` `points`.
- `WrapMode` `clampzero`.
- `SpriteBatchUsage` (`dynamic`, `static`, `stream`). Only a hint.

### Canvas

- `newCanvas` settings: `format` (float, depth and stencil formats and the
  rest of `PixelFormat`), `msaa`, `readable`, `type` (array, cube,
  volume canvases), `layers`.
- `Canvas:getMSAA`.

### Texture (Image and Canvas)

- `getFormat`, `getTextureType`, `getDepth`, `isReadable`.
- `getDepthSampleMode`, `setDepthSampleMode`.

### Image

- `Image:isCompressed`, `Image:isFormatLinear`, and the `linear` setting
  in `newImage`.

### Font

- ★ `Font:setFallbacks`.
- ★ `Font:hasGlyphs`.
- `Font:getBaseline`.
- `Font:getKerning`. Kerning is applied, but cannot be read.

### Mesh

- Custom vertex formats in `newMesh`, and `Mesh:getVertexFormat`.
- `Mesh:attachAttribute`, `detachAttribute`, `setAttributeEnabled`,
  `isAttributeEnabled`.
- `Mesh:getVertexAttribute`, `setVertexAttribute`.
- `Mesh:flush`.
- Per-instance attributes (`VertexAttributeStep`) for `drawInstanced`.
- `IndexDataType` `uint16` for vertex maps.

### ParticleSystem

Smitten has `emit`, `start`, `stop`, `pause`, `reset`, `isActive`,
`getCount`, `update`, `getPosition`, `setPosition` and the main setters.
Missing:

- ★ `setSizeVariation`, `getSizeVariation`.
- ★ `setSpinVariation`, `getSpinVariation`.
- ★ `setRadialAcceleration`, `getRadialAcceleration`.
- ★ `setTangentialAcceleration`, `getTangentialAcceleration`.
- ★ `setQuads`, `getQuads`: animated particles from a sprite sheet.
- ★ `setOffset`, `getOffset`.
- ★ `setRelativeRotation`, `hasRelativeRotation`.
- ★ `setEmissionArea` distributions (`normal`, `ellipse`,
  `borderellipse`, `borderrectangle`), its angle, and its
  direction-relative flag. Smitten has the uniform rectangle only.
- ★ `moveTo`: move the emitter and spread new particles along the way.
- `clone`.
- `setInsertMode`, `getInsertMode` (`ParticleInsertMode`).
- `setBufferSize`, `getBufferSize`.
- `setTexture`, `getTexture`.
- `isPaused`, `isStopped`.
- Getters: `getColors`, `getDirection`, `getEmissionArea`,
  `getEmissionRate`, `getEmitterLifetime`, `getLinearAcceleration`,
  `getLinearDamping`, `getParticleLifetime`, `getRotation`, `getSizes`,
  `getSpeed`, `getSpin`, `getSpread`.

### Quad

- `Quad:getViewport`, `setViewport`, `getTextureDimensions`. The fields
  are private, so a quad cannot be read back or moved.

### Shader

- ★ Sending matrices (`mat2`, `mat3`, `mat4`, with `MatrixLayout`),
  booleans, integer vectors and arrays.
- A vertex stage (LÖVE's `position` function). Smitten shaders are
  fragment-only.
- `Shader:getWarnings`.
- LÖVE's GLSL dialect. Smitten shaders are WGSL. Listed for completeness;
  porting this is probably not worth it.

### SpriteBatch

- `SpriteBatch:addLayer`, `setLayer`: batches of an `ArrayImage`.
- `SpriteBatch:setDrawRange`.
- `SpriteBatch:getBufferSize`, and growing past the capacity it was made
  with.
- `SpriteBatch:attachAttribute`, `flush`.

### Video

- `Video:getStream`, `Video:setSource`.

## image

- `ImageData:getFormat`, and `ImageData` in formats other than RGBA8
  (`newImageData` with a `PixelFormat`).
- `ImageData:encode` as TGA. PNG works.
- `love.image.isCompressed`.

## joystick

- `Joystick:getVibration`.

## keyboard

All functions are there. Missing key constants (61):

- ★ Keypad: `kp0` to `kp9`, `kp.`, `kp/`, `kp*`, `kp-`, `kp+`,
  `kpenter`, `kp=`.
- `f13`, `f14`, `f15`.
- `numlock`, `scrolllock`, `lmeta`, `rmeta`, `print`, `sysreq`, `pause`, `break`, `menu`,
  `clear`, `help`, `power`, `mode`, `compose`, `euro`, `undo`.
- Media and app keys: `www`, `mail`, `calculator`, `appsearch`, `apphome`,
  `appback`, `appforward`, `apprefresh`, `appbookmarks`.
- Shifted symbol keys that some layouts have as their own key: `!`, `"`,
  `#`, `$`, `&`, `(`, `)`, `*`, `+`, `:`, `<`, `>`, `?`, `@`, `^`,
  `_`.

The `Scancode` set is the same story: smitten has the same 83, LÖVE has 194,
mostly international and media keys.

## math

- `love.math.colorFromBytes`, `colorToBytes`.
- `love.math.getRandomState`, `setRandomState`, and
  `RandomGenerator:getState`, `setState`: save and restore a generator.
- `Transform:isAffine2DTransform`.
- 4D `noise`. Smitten has 1D, 2D and 3D.

## mouse

Complete.

## physics

`beginContact`, `endContact`, `preSolve` and `postSolve` are covered by
`World::contacts`, `World::set_pre_solve` and `World::touching`, so they
are not listed.

### Module and World

- `love.physics.getDistance`: closest points between two fixtures.
- `World:setContactFilter`, `getContactFilter`: a callback that decides
  which fixtures collide. Category, mask and group filters exist.
- `World:setSleepingAllowed`, `isSleepingAllowed`.
- `World:getContactCount`.
- `World:isLocked`.
- `World:translateOrigin`.

### Shapes

Smitten has no standalone shapes: fixtures are made with
`Body::add_circle` and the rest. That leaves out:

- `newCircleShape`, `newRectangleShape`, `newPolygonShape`,
  `newEdgeShape`, `newChainShape`, `newFixture`, `Fixture:getShape`.
- `Shape:computeAABB`, `computeMass`, `rayCast`, `testPoint`,
  `getChildCount`, `getRadius`, `getType`.
- `CircleShape:getRadius`, `setRadius`, `getPoint`, `setPoint`: resizing a
  circle without remaking the fixture.
- `PolygonShape:getPoints`, `EdgeShape:getPoints`, `ChainShape:getPoints`,
  `getPoint`, `getVertexCount`, `getChildEdge`.
- Ghost vertices: `EdgeShape` and `ChainShape` `setNextVertex`,
  `setPreviousVertex`, `getNextVertex`, `getPreviousVertex`. These stop
  bodies catching on the joins between edges.

### Body

- ★ `getWorldCenter`, `getLocalCenter`.
- ★ `getWorldVector`, `getLocalVector`.
- ★ `getLinearVelocityFromWorldPoint`, `getLinearVelocityFromLocalPoint`.
- `getWorldPoints`, `getLocalPoints` (many points at once; single points
  exist).
- ★ `setMass`, `setInertia`, `getMassData`, `setMassData`,
  `resetMassData`.
- `getContacts`.
- `setActive`, `isActive`: take a body out of the simulation without
  destroying it.
- `setSleepingAllowed`, `isSleepingAllowed`.
- `getTransform`, `setTransform`, `setX`, `setY`.
- `getWorld`.

### Fixture

- `Fixture:rayCast`.
- `Fixture:getMassData`.

### Contact

Smitten's `Contact`, `Touch` and `PreSolve` structs cover reading and
changing contacts. Missing:

- `Contact:getPositions` with two points. Smitten reports one.
- `Contact:getChildren`: which edge of a chain was hit.
- `resetFriction`, `resetRestitution` (only matter if a change outlives
  the step).

### Joints

- `getReferenceAngle` on revolute, prismatic and weld joints.
- `getAxis` on prismatic and wheel joints.
- `setLowerLimit`, `setUpperLimit`, `getLowerLimit`, `getUpperLimit`
  (`setLimits` and `getLimits` exist).
- Gear and pulley joints: left out on purpose.

## sound

- `Decoder:clone`.
- `Decoder:getBitDepth`, `SoundData:getBitDepth`, and 8 or 16 bit
  `SoundData`. Smitten stores floats.

## system

- `love.system.hasBackgroundMusic` (iOS and Android only).

## thread

The whole module (`newThread`, `newChannel`, `getChannel`, `Thread`,
`Channel`) is missing. Rust's `std::thread` and `std::sync::mpsc` do the
same job. A `Channel` lookalike could help ports, but it is optional.

## timer

- `love.timer.step` as a public function. It only matters with a custom
  run loop.

## video

- `VideoStream:getFilename`. Smitten's `VideoStream` pulls frames rather
  than playing, and `Video` covers play, pause, seek and tell.

## window

- ★ `setMode`, `getMode`, `updateMode`, with LÖVE's flags:
  `fullscreentype`, `vsync` (including adaptive), `msaa`, `stencil`,
  `depth`, `resizable`, `borderless`, `centered`, `display`, `minwidth`,
  `minheight`, `highdpi`, `usedpiscale`, `x`, `y`.
- `getFullscreenModes`, and exclusive fullscreen (`FullscreenType`).
  `setFullscreen` takes a bool and uses desktop fullscreen.
- `getDisplayName`.
- A display index for `getDesktopDimensions`, `getPosition`,
  `setPosition` and `getDPIScale`.
- `close`, `isOpen`.
- `setDisplaySleepEnabled`, `isDisplaySleepEnabled`.

### `love.conf` (`Config`)

`Config` has title, size, DPI scaling, resizable, fullscreen, icon,
identity and vsync. Missing:

- `t.window`: `borderless`, `minwidth`, `minheight`, `fullscreentype`,
  `msaa`, `depth`, `stencil`, `display`, `x`, `y`, `centered`.
- `t.gammacorrect`, `t.appendidentity`, `t.externalstorage`,
  `t.accelerometerjoystick`, `t.console`.
- `t.audio.mic`, `t.audio.mixwithsystem`.
- `t.modules.*`: turning modules off.

## Callbacks and top level

- `love.errorhandler`: a custom error screen. The built-in one exists.
- `love.run`: a custom main loop.
- `love.threaderror`: not applicable without `love.thread`.
- `love.getVersion`, `love.isVersionCompatible`,
  `love.setDeprecationOutput`, `love.hasDeprecationOutput`. The crate
  version covers the first two; the deprecation ones do not apply.
- `Object` (`release`, `type`, `typeOf`) and `Data` (`clone`, `getSize`,
  `getString`, `getPointer`, `getFFIPointer`): not applicable to Rust.

## Left out on purpose

| Item | Why |
|---|---|
| `love.event.poll`, `wait`, `pump` | Smitten owns the event loop and delivers events to `Game` callbacks. These only make sense with a custom `love.run`. |
| Gear and pulley joints | Rapier, the physics engine underneath, has no built-in version of either. |
| KTX, ASTC and ETC textures | Only DDS files load. BC6H and PVRTC are missing too (`CompressedFormat` has BC1 to BC5 and BC7). |
| `drawLayer` with a custom shader | The array-texture pipeline has its own shader. |
| macOS and web | The renderer is Vulkan only. |
| Lua-only filesystem functions | `load`, the require paths, `init`, `setSource` have no Rust meaning. |
| `love.thread` | Rust threads and channels. |
| `Object`, `Data` base types | Rust ownership and slices. |
| LÖVE's GLSL shader dialect | Smitten shaders are WGSL. |

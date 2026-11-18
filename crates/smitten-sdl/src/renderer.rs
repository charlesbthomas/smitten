//! Drawing through SDL's GPU API.
//!
//! The renderer draws one kind of thing: textured, vertex-colored
//! triangles. Shapes sample a 1x1 white texture so they can share the
//! shader. Pipelines are built the first time each combination of shader,
//! blend mode, stencil mode and target is drawn with. Each
//! frame's vertices are uploaded once and drawn as a few runs, pass by pass.

use std::collections::HashMap;
use std::ffi::{CStr, c_void};
use std::ptr;

use sdl3_sys::everything::*;
use smitten_core::backend::{
    BoundTexture, ColorMask, CompressedFormat, DrawCall, Frame, RendererInfo, Scissor, ShaderId,
    ShaderInfo, TextureId, TextureKind, Vertex,
};
use smitten_core::{
    BlendAlphaMode, BlendMode, CompareMode, Filter, StencilAction, StencilMode, VertexWinding, Wrap,
};

use crate::{sdl_error, shader};

// Compiled from the GLSL beside them by `shaders/build.sh`.
static VERTEX_SPIRV: &[u8] = include_bytes!("../shaders/sprite.vert.spv");
static FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite.frag.spv");
static ARRAY_FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite_array.frag.spv");

const VERTEX_SIZE: u32 = size_of::<Vertex>() as u32;
const INITIAL_VERTEX_CAPACITY: u32 = 4096;
/// The format of images and render targets. The screen may differ.
const TEXTURE_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;

/// Everything that needs its own pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    /// A custom shader, or `None` for the built-in one.
    shader: Option<u32>,
    /// Whether the built-in shader draws a layer of an array texture.
    layered: bool,
    /// Whether it draws to a render target rather than the screen.
    to_target: bool,
    blend: BlendMode,
    blend_alpha: BlendAlphaMode,
    stencil: StencilMode,
    color_mask: ColorMask,
    wireframe: bool,
    /// How many samples each pixel of the target holds.
    samples: u32,
    /// The on-screen winding of the triangles to leave out.
    cull: Option<VertexWinding>,
}

#[derive(Clone, Copy)]
struct Texture {
    handle: *mut SDL_GPUTexture,
    width: u32,
    height: u32,
    /// Whether it can be drawn to as well as sampled.
    is_target: bool,
    /// The stencil values that go with a render target. Created the first
    /// time the target is drawn to.
    stencil: *mut SDL_GPUTexture,
    /// Whether `stencil` is new and still holds garbage.
    stencil_fresh: bool,
    /// For a multisampled render target, the texture drawn to, which is
    /// resolved into `handle` at the end of every pass. Null otherwise.
    msaa: *mut SDL_GPUTexture,
    /// How many samples each pixel of `msaa` holds, 1 without it.
    samples: u32,
    /// What shows beyond the edges, horizontally and vertically.
    wrap: (Wrap, Wrap),
    /// How much GPU memory the pixels take, for the game's statistics.
    bytes: u64,
    /// How many mipmap levels it has, 1 for none.
    levels: u32,
    /// How mipmaps are sampled, once the texture has them.
    mipmaps: Option<Mipmaps>,
    kind: TextureKind,
    /// Whether the pixels are compressed, which means they cannot be
    /// replaced.
    compressed: bool,
    /// Whether a render target's mipmaps are made again after each run of
    /// passes that draws to it.
    auto_mipmaps: bool,
}

/// How a texture with mipmaps blends between them.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Mipmaps {
    filter: Filter,
    /// The game's sharpness as the bits of an `f32`, so it can be hashed.
    sharpness: u32,
}

/// Everything that tells one sampler from another.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SamplerKey {
    filter: Filter,
    wrap: (Wrap, Wrap),
    mipmaps: Option<Mipmaps>,
}

/// A fragment shader supplied by the game.
struct CustomShader {
    fragment: *mut SDL_GPUShader,
    /// How many bytes of uniform data each draw must supply.
    uniform_size: u32,
    /// The kinds of the textures it declares besides `image`.
    textures: Vec<TextureKind>,
}

pub(crate) struct Renderer {
    device: *mut SDL_GPUDevice,
    screen_format: SDL_GPUTextureFormat,
    /// The vertex shader every pipeline shares.
    vertex_shader: *mut SDL_GPUShader,
    shaders: HashMap<u32, CustomShader>,
    next_shader_id: u32,
    /// The built-in fragment shader.
    fragment_shader: *mut SDL_GPUShader,
    /// The built-in fragment shader for a layer of an array texture.
    array_fragment_shader: *mut SDL_GPUShader,
    /// Built the first time each combination is drawn with.
    pipelines: HashMap<PipelineKey, *mut SDL_GPUGraphicsPipeline>,
    stencil_format: SDL_GPUTextureFormat,
    /// The screen's stencil values, and the size they were made for.
    screen_stencil: *mut SDL_GPUTexture,
    screen_stencil_size: (u32, u32),
    screen_stencil_fresh: bool,
    /// The multisampled picture of the screen, resolved into the window
    /// at the end of every pass, or null without multisampling.
    screen_msaa: *mut SDL_GPUTexture,
    /// How many samples each pixel of the screen holds, 1 for none.
    screen_samples: u32,
    linear: *mut SDL_GPUSampler,
    nearest: *mut SDL_GPUSampler,
    /// Samplers for textures that do not clamp or have mipmaps, made when
    /// first asked for.
    samplers: HashMap<SamplerKey, *mut SDL_GPUSampler>,
    white: *mut SDL_GPUTexture,
    /// Stand-ins for textures a shader declares but was not sent.
    white_array: *mut SDL_GPUTexture,
    white_cube: *mut SDL_GPUTexture,
    textures: HashMap<u32, Texture>,
    next_texture_id: u32,
    vertex_buffer: *mut SDL_GPUBuffer,
    transfer_buffer: *mut SDL_GPUTransferBuffer,
    /// How many vertices the two buffers above can hold.
    vertex_capacity: u32,
}

impl Renderer {
    /// Builds the pipeline for `window`, which `device` must already own.
    pub fn new(device: *mut SDL_GPUDevice, window: *mut SDL_Window) -> Result<Self, String> {
        let mut renderer = Self {
            device,
            screen_format: SDL_GPU_TEXTUREFORMAT_INVALID,
            vertex_shader: ptr::null_mut(),
            shaders: HashMap::new(),
            next_shader_id: 1,
            fragment_shader: ptr::null_mut(),
            array_fragment_shader: ptr::null_mut(),
            pipelines: HashMap::new(),
            stencil_format: SDL_GPU_TEXTUREFORMAT_INVALID,
            screen_stencil: ptr::null_mut(),
            screen_stencil_size: (0, 0),
            screen_stencil_fresh: false,
            screen_msaa: ptr::null_mut(),
            screen_samples: 1,
            linear: ptr::null_mut(),
            samplers: HashMap::new(),
            nearest: ptr::null_mut(),
            white: ptr::null_mut(),
            white_array: ptr::null_mut(),
            white_cube: ptr::null_mut(),
            textures: HashMap::new(),
            next_texture_id: 1,
            vertex_buffer: ptr::null_mut(),
            transfer_buffer: ptr::null_mut(),
            vertex_capacity: 0,
        };
        // On failure `renderer` is dropped, which releases whatever was
        // created so far. SDL ignores null handles on release.
        // SAFETY: `device` and `window` are alive and belong together.
        unsafe {
            let screen_format = SDL_GetGPUSwapchainTextureFormat(device, window);
            renderer.screen_format = screen_format;
            renderer.vertex_shader = create_shader(
                device,
                VERTEX_SPIRV,
                c"main",
                SDL_GPU_SHADERSTAGE_VERTEX,
                0,
                1,
            )?;
            renderer.fragment_shader = create_shader(
                device,
                FRAGMENT_SPIRV,
                c"main",
                SDL_GPU_SHADERSTAGE_FRAGMENT,
                1,
                0,
            )?;
            renderer.array_fragment_shader = create_shader(
                device,
                ARRAY_FRAGMENT_SPIRV,
                c"main",
                SDL_GPU_SHADERSTAGE_FRAGMENT,
                1,
                1,
            )?;
            renderer.stencil_format = [
                SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT,
                SDL_GPU_TEXTUREFORMAT_D32_FLOAT_S8_UINT,
            ]
            .into_iter()
            .find(|&format| {
                SDL_GPUTextureSupportsFormat(
                    device,
                    format,
                    SDL_GPU_TEXTURETYPE_2D,
                    SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET,
                )
            })
            .ok_or("the GPU has no stencil format")?;
            // Build the pipeline nearly every frame uses now, so a broken
            // setup fails at startup rather than drawing nothing.
            let common = PipelineKey {
                shader: None,
                layered: false,
                to_target: false,
                blend: BlendMode::Alpha,
                blend_alpha: BlendAlphaMode::AlphaMultiply,
                stencil: StencilMode::Off,
                color_mask: ColorMask::ALL,
                wireframe: false,
                samples: 1,
                cull: None,
            };
            let pipeline = renderer.create_pipeline(common)?;
            renderer.pipelines.insert(common, pipeline);
            renderer.linear = create_sampler(device, SamplerKey::clamped(Filter::Linear))?;
            renderer.nearest = create_sampler(device, SamplerKey::clamped(Filter::Nearest))?;
            renderer.white = new_texture(device, 1, 1, 1, &[255, 255, 255, 255])?;
            let white: &[u8] = &[255; 4];
            renderer.white_array = new_layered_texture(device, TextureKind::Array, 1, 1, &[white])?;
            renderer.white_cube =
                new_layered_texture(device, TextureKind::Cube, 1, 1, &[white; 6])?;
        }
        renderer.reserve_vertices(INITIAL_VERTEX_CAPACITY)?;
        Ok(renderer)
    }

    pub fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        mipmapped: bool,
    ) -> Result<TextureId, String> {
        if width == 0 || height == 0 {
            return Err("an image must be at least 1x1 pixels".to_string());
        }
        check_pixel_count(width, height, rgba)?;
        let levels = if mipmapped {
            mipmap_levels(width, height)
        } else {
            1
        };
        let mipmaps = mipmapped.then_some(Mipmaps {
            filter: Filter::Linear,
            sharpness: 0f32.to_bits(),
        });
        if let Some(mipmaps) = mipmaps {
            self.ensure_samplers((Wrap::Clamp, Wrap::Clamp), Some(mipmaps))?;
        }
        // SAFETY: the device is alive and `rgba` was just checked to cover
        // the whole texture.
        let handle = unsafe { new_texture(self.device, width, height, levels, rgba)? };
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                width,
                height,
                is_target: false,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa: ptr::null_mut(),
                samples: 1,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: mipmap_bytes(width, height, levels),
                levels,
                mipmaps,
                kind: TextureKind::Flat,
                compressed: false,
                auto_mipmaps: false,
            },
        );
        Ok(TextureId(id))
    }

    pub fn supports_compressed_format(&self, format: CompressedFormat) -> bool {
        // SAFETY: the device is alive.
        unsafe {
            SDL_GPUTextureSupportsFormat(
                self.device,
                compressed_format(format),
                SDL_GPU_TEXTURETYPE_2D,
                SDL_GPU_TEXTUREUSAGE_SAMPLER,
            )
        }
    }

    pub fn create_compressed_texture(
        &mut self,
        format: CompressedFormat,
        width: u32,
        height: u32,
        levels: &[&[u8]],
    ) -> Result<TextureId, String> {
        if width == 0 || height == 0 {
            return Err("an image must be at least 1x1 pixels".to_string());
        }
        if levels.is_empty() || levels.len() as u32 > mipmap_levels(width, height) {
            return Err(format!(
                "a {width}x{height} image cannot have {} mipmap levels",
                levels.len()
            ));
        }
        for (level, data) in levels.iter().enumerate() {
            let (level_width, level_height) = level_size(width, height, level as u32);
            let expected = format.data_size(level_width, level_height);
            if data.len() != expected {
                return Err(format!(
                    "level {level} of a {width}x{height} {format:?} image needs {expected} bytes, got {}",
                    data.len()
                ));
            }
        }
        if !self.supports_compressed_format(format) {
            return Err(format!("this GPU cannot read {format:?} images"));
        }
        let level_count = levels.len() as u32;
        let mipmaps = (level_count > 1).then_some(Mipmaps {
            filter: Filter::Linear,
            sharpness: 0f32.to_bits(),
        });
        if let Some(mipmaps) = mipmaps {
            self.ensure_samplers((Wrap::Clamp, Wrap::Clamp), Some(mipmaps))?;
        }
        // SAFETY: the device is alive, and each level was just checked to
        // hold exactly its size.
        let handle = unsafe {
            let handle = SDL_CreateGPUTexture(
                self.device,
                &SDL_GPUTextureCreateInfo {
                    r#type: SDL_GPU_TEXTURETYPE_2D,
                    format: compressed_format(format),
                    usage: SDL_GPU_TEXTUREUSAGE_SAMPLER,
                    width,
                    height,
                    layer_count_or_depth: 1,
                    num_levels: level_count,
                    sample_count: SDL_GPU_SAMPLECOUNT_1,
                    ..Default::default()
                },
            );
            if handle.is_null() {
                return Err(format!("could not create a texture: {}", sdl_error()));
            }
            for (level, data) in levels.iter().enumerate() {
                let (level_width, level_height) = level_size(width, height, level as u32);
                let region = Region {
                    level: level as u32,
                    width: level_width,
                    height: level_height,
                    ..Region::default()
                };
                if let Err(message) = upload(self.device, handle, region, data) {
                    SDL_ReleaseGPUTexture(self.device, handle);
                    return Err(message);
                }
            }
            handle
        };
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                width,
                height,
                is_target: false,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa: ptr::null_mut(),
                samples: 1,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: levels.iter().map(|level| level.len() as u64).sum(),
                levels: level_count,
                mipmaps,
                kind: TextureKind::Flat,
                compressed: true,
                auto_mipmaps: false,
            },
        );
        Ok(TextureId(id))
    }

    pub fn create_layered_texture(
        &mut self,
        kind: TextureKind,
        width: u32,
        height: u32,
        layers: &[&[u8]],
    ) -> Result<TextureId, String> {
        if kind == TextureKind::Flat {
            let [layer] = layers else {
                return Err("a flat texture has exactly one layer".to_string());
            };
            return self.create_texture(width, height, layer, false);
        }
        if width == 0 || height == 0 {
            return Err("an image must be at least 1x1 pixels".to_string());
        }
        if layers.is_empty() {
            return Err("there must be at least one layer".to_string());
        }
        if kind == TextureKind::Cube && (layers.len() != 6 || width != height) {
            return Err("a cube needs six square faces".to_string());
        }
        for layer in layers {
            check_pixel_count(width, height, layer)?;
        }
        // SAFETY: the device is alive and every layer was just checked to
        // cover the whole texture.
        let handle = unsafe { new_layered_texture(self.device, kind, width, height, layers)? };
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                width,
                height,
                is_target: false,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa: ptr::null_mut(),
                samples: 1,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: u64::from(width) * u64::from(height) * 4 * layers.len() as u64,
                levels: 1,
                mipmaps: None,
                kind,
                compressed: false,
                auto_mipmaps: false,
            },
        );
        Ok(TextureId(id))
    }

    /// Creates a render target, with a full chain of mipmaps unless
    /// `mipmaps` is `None`. `Some(true)` makes them again whenever the
    /// target has been drawn to.
    /// With `samples` above 1 it is multisampled, with as many samples as
    /// the GPU allows up to that.
    pub fn create_render_target(
        &mut self,
        width: u32,
        height: u32,
        mipmaps: Option<bool>,
        samples: u32,
    ) -> Result<TextureId, String> {
        if width == 0 || height == 0 {
            return Err("a canvas must be at least 1x1 pixels".to_string());
        }
        let levels = if mipmaps.is_some() {
            mipmap_levels(width, height)
        } else {
            1
        };
        let blend = mipmaps.is_some().then_some(Mipmaps {
            filter: Filter::Linear,
            sharpness: 0f32.to_bits(),
        });
        if let Some(blend) = blend {
            self.ensure_samplers((Wrap::Clamp, Wrap::Clamp), Some(blend))?;
        }
        // SAFETY: the device is alive, and the passes are begun, ended and
        // submitted in the order SDL requires.
        let handle = unsafe {
            let handle = SDL_CreateGPUTexture(
                self.device,
                &SDL_GPUTextureCreateInfo {
                    r#type: SDL_GPU_TEXTURETYPE_2D,
                    format: TEXTURE_FORMAT,
                    usage: SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                    width,
                    height,
                    layer_count_or_depth: 1,
                    num_levels: levels,
                    sample_count: SDL_GPU_SAMPLECOUNT_1,
                    ..Default::default()
                },
            );
            if handle.is_null() {
                return Err(format!("could not create a canvas: {}", sdl_error()));
            }
            // New GPU memory holds garbage, so start it out transparent.
            let commands = SDL_AcquireGPUCommandBuffer(self.device);
            if !commands.is_null() {
                let target = SDL_GPUColorTargetInfo {
                    texture: handle,
                    load_op: SDL_GPU_LOADOP_CLEAR,
                    store_op: SDL_GPU_STOREOP_STORE,
                    ..Default::default()
                };
                let pass = SDL_BeginGPURenderPass(commands, &target, 1, ptr::null());
                if !pass.is_null() {
                    SDL_EndGPURenderPass(pass);
                }
                // The smaller levels start out transparent too.
                if levels > 1 {
                    SDL_GenerateMipmapsForGPUTexture(commands, handle);
                }
                SDL_SubmitGPUCommandBuffer(commands);
            }
            handle
        };
        let samples = self.supported_samples(TEXTURE_FORMAT, samples);
        let msaa = if samples > 1 {
            // SAFETY: the device is alive.
            let msaa =
                unsafe { new_msaa_texture(self.device, TEXTURE_FORMAT, width, height, samples) };
            if msaa.is_null() {
                let message = format!("could not create a multisampled canvas: {}", sdl_error());
                // SAFETY: the texture came from this device.
                unsafe { SDL_ReleaseGPUTexture(self.device, handle) };
                return Err(message);
            }
            // Its samples start out transparent too.
            // SAFETY: the device and texture are alive, and the pass is
            // begun, ended and submitted in order.
            unsafe {
                let commands = SDL_AcquireGPUCommandBuffer(self.device);
                if !commands.is_null() {
                    let target = SDL_GPUColorTargetInfo {
                        texture: msaa,
                        load_op: SDL_GPU_LOADOP_CLEAR,
                        store_op: SDL_GPU_STOREOP_STORE,
                        ..Default::default()
                    };
                    let pass = SDL_BeginGPURenderPass(commands, &target, 1, ptr::null());
                    if !pass.is_null() {
                        SDL_EndGPURenderPass(pass);
                    }
                    SDL_SubmitGPUCommandBuffer(commands);
                }
            }
            msaa
        } else {
            ptr::null_mut()
        };
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                width,
                height,
                is_target: true,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa,
                samples,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: mipmap_bytes(width, height, levels)
                    + u64::from(width) * u64::from(height) * 4 * u64::from(samples.max(1) - 1),
                levels,
                mipmaps: blend,
                kind: TextureKind::Flat,
                compressed: false,
                auto_mipmaps: mipmaps == Some(true),
            },
        );
        Ok(TextureId(id))
    }

    pub fn update_texture(
        &mut self,
        texture: TextureId,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), String> {
        let Some(&target) = self.textures.get(&texture.0) else {
            return Err("the texture no longer exists".to_string());
        };
        if target.kind != TextureKind::Flat || target.compressed {
            return Err("only a flat, uncompressed texture can be updated".to_string());
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        let fits = x
            .checked_add(width)
            .is_some_and(|right| right <= target.width)
            && y.checked_add(height)
                .is_some_and(|bottom| bottom <= target.height);
        if !fits {
            return Err(format!(
                "a {width}x{height} update at ({x}, {y}) does not fit in a {}x{} texture",
                target.width, target.height
            ));
        }
        check_pixel_count(width, height, rgba)?;
        // SAFETY: the texture is alive, the region was checked to lie inside
        // it, and `rgba` was checked to cover the region.
        unsafe {
            upload_region(self.device, target.handle, 0, x, y, width, height, rgba)?;
            if target.levels > 1 {
                generate_mipmaps(self.device, target.handle)?;
            }
        }
        Ok(())
    }

    /// Sets how a texture with mipmaps blends between them. `None` uses
    /// only the full-size picture. Positive `sharpness` picks larger
    /// mipmaps than the GPU would.
    pub fn set_texture_mipmap_filter(
        &mut self,
        texture: TextureId,
        filter: Option<Filter>,
        sharpness: f32,
    ) {
        let Some(&entry) = self.textures.get(&texture.0) else {
            return;
        };
        if entry.levels <= 1 {
            return;
        }
        let mipmaps = filter.map(|filter| Mipmaps {
            filter,
            sharpness: sharpness.to_bits(),
        });
        if let Err(message) = self.ensure_samplers(entry.wrap, mipmaps) {
            eprintln!("smitten: {message}");
            return;
        }
        if let Some(entry) = self.textures.get_mut(&texture.0) {
            entry.mipmaps = mipmaps;
        }
    }

    /// Makes the samplers a texture with these settings needs, for both
    /// filters, so that drawing never has to stop to make one.
    fn ensure_samplers(
        &mut self,
        wrap: (Wrap, Wrap),
        mipmaps: Option<Mipmaps>,
    ) -> Result<(), String> {
        for filter in [Filter::Linear, Filter::Nearest] {
            let key = SamplerKey {
                filter,
                wrap,
                mipmaps,
            };
            if key == SamplerKey::clamped(filter) || self.samplers.contains_key(&key) {
                continue;
            }
            // SAFETY: the device is alive.
            let sampler = unsafe { create_sampler(self.device, key)? };
            self.samplers.insert(key, sampler);
        }
        Ok(())
    }

    pub fn set_texture_wrap(&mut self, texture: TextureId, horizontal: Wrap, vertical: Wrap) {
        let Some(entry) = self.textures.get(&texture.0) else {
            return;
        };
        let wrap = (horizontal, vertical);
        let mipmaps = entry.mipmaps;
        if let Err(message) = self.ensure_samplers(wrap, mipmaps) {
            eprintln!("smitten: {message}");
            return;
        }
        if let Some(entry) = self.textures.get_mut(&texture.0) {
            entry.wrap = wrap;
        }
    }

    /// The GPU memory taken by every texture the game holds, in bytes.
    /// Makes a render target's mipmaps again from what it holds now.
    pub fn generate_mipmaps(&mut self, texture: TextureId) -> Result<(), String> {
        let Some(&target) = self.textures.get(&texture.0) else {
            return Err("the texture no longer exists".to_string());
        };
        if target.levels <= 1 || !target.is_target {
            return Err("only a canvas made with mipmaps can make them".to_string());
        }
        // SAFETY: the texture is alive and was made to be drawn to.
        unsafe { generate_mipmaps(self.device, target.handle) }
    }

    pub fn texture_memory(&self) -> u64 {
        self.textures.values().map(|texture| texture.bytes).sum()
    }

    pub fn destroy_texture(&mut self, texture: TextureId) {
        if let Some(texture) = self.textures.remove(&texture.0) {
            // SAFETY: the texture came from this device. SDL defers the
            // actual free until no in-flight frame uses it.
            // SDL ignores a null stencil texture.
            unsafe {
                SDL_ReleaseGPUTexture(self.device, texture.handle);
                SDL_ReleaseGPUTexture(self.device, texture.stencil);
                SDL_ReleaseGPUTexture(self.device, texture.msaa);
            }
        }
    }

    /// The most samples up to `wanted` that the GPU can give a target of
    /// `format` and its stencil, or 1.
    fn supported_samples(&self, format: SDL_GPUTextureFormat, wanted: u32) -> u32 {
        [8, 4, 2]
            .into_iter()
            .find(|&count| {
                count <= wanted
                    // SAFETY: the device is alive.
                    && unsafe {
                        SDL_GPUTextureSupportsSampleCount(self.device, format, sample_count(count))
                            && SDL_GPUTextureSupportsSampleCount(
                                self.device,
                                self.stencil_format,
                                sample_count(count),
                            )
                    }
            })
            .unwrap_or(1)
    }

    /// Multisamples the screen with as many samples as the GPU allows up
    /// to `wanted`, or stops for 1 or less. Returns the number used.
    pub fn set_screen_msaa(&mut self, wanted: u32) -> u32 {
        let samples = self.supported_samples(self.screen_format, wanted);
        if samples != self.screen_samples {
            self.screen_samples = samples;
            // Made again at the right size and count before the next draw.
            self.screen_stencil_size = (0, 0);
        }
        samples
    }

    /// How many samples each pixel of the screen holds.
    pub fn screen_msaa(&self) -> u32 {
        self.screen_samples
    }

    /// How many samples each pixel of a canvas holds.
    pub fn texture_msaa(&self, texture: TextureId) -> u32 {
        self.textures
            .get(&texture.0)
            .map_or(1, |texture| texture.samples)
    }

    /// How many samples each pixel of a pass's target holds.
    fn target_samples(&self, target: Option<TextureId>) -> u32 {
        match target {
            None => self.screen_samples,
            Some(id) => self.texture_msaa(id),
        }
    }

    /// The graphics API, driver and GPU, as SDL reports them.
    pub fn renderer_info(&self) -> RendererInfo {
        let text = |pointer: *const std::ffi::c_char| {
            if pointer.is_null() {
                String::new()
            } else {
                // SAFETY: SDL returns a NUL-terminated string that lives
                // as long as the device.
                unsafe { CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .into_owned()
            }
        };
        // SAFETY: the device is alive.
        unsafe {
            let properties = SDL_GetGPUDeviceProperties(self.device);
            let property = |name| text(SDL_GetStringProperty(properties, name, ptr::null()));
            let api = text(SDL_GetGPUDeviceDriver(self.device));
            RendererInfo {
                name: match api.as_str() {
                    "vulkan" => "Vulkan".to_string(),
                    "direct3d12" => "Direct3D 12".to_string(),
                    "metal" => "Metal".to_string(),
                    _ => api,
                },
                // The detailed version is usually the more useful one.
                version: match property(SDL_PROP_GPU_DEVICE_DRIVER_INFO_STRING) {
                    info if info.is_empty() => property(SDL_PROP_GPU_DEVICE_DRIVER_VERSION_STRING),
                    info => info,
                },
                vendor: property(SDL_PROP_GPU_DEVICE_DRIVER_NAME_STRING),
                device: property(SDL_PROP_GPU_DEVICE_NAME_STRING),
            }
        }
    }

    pub fn create_shader(&mut self, source: &str) -> Result<ShaderInfo, String> {
        let compiled = shader::compile(source)?;
        // SAFETY: the device is alive and the SPIR-V outlives the call.
        let fragment = unsafe {
            create_shader(
                self.device,
                &compiled.spirv,
                shader::ENTRY_POINT,
                SDL_GPU_SHADERSTAGE_FRAGMENT,
                1 + compiled.textures.len() as u32,
                u32::from(compiled.uniform_size > 0),
            )?
        };
        let id = self.next_shader_id;
        self.next_shader_id += 1;
        self.shaders.insert(
            id,
            CustomShader {
                fragment,
                uniform_size: compiled.uniform_size,
                textures: compiled
                    .textures
                    .iter()
                    .map(|texture| texture.kind)
                    .collect(),
            },
        );
        Ok(ShaderInfo {
            id: ShaderId(id),
            uniform_size: compiled.uniform_size,
            fields: compiled.fields,
            textures: compiled.textures,
        })
    }

    pub fn destroy_shader(&mut self, shader: ShaderId) {
        let device = self.device;
        self.pipelines.retain(|key, &mut pipeline| {
            let owned = key.shader == Some(shader.0);
            if owned {
                // SAFETY: the pipeline came from this device. SDL defers
                // the free until no in-flight frame uses it.
                unsafe { SDL_ReleaseGPUGraphicsPipeline(device, pipeline) };
            }
            !owned
        });
        if let Some(shader) = self.shaders.remove(&shader.0) {
            // SAFETY: as above.
            unsafe { SDL_ReleaseGPUShader(device, shader.fragment) };
        }
    }

    fn create_pipeline(&self, key: PipelineKey) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
        let fragment = match key.shader {
            None if key.layered => self.array_fragment_shader,
            None => self.fragment_shader,
            Some(id) => {
                self.shaders
                    .get(&id)
                    .ok_or("the shader no longer exists")?
                    .fragment
            }
        };
        let format = if key.to_target {
            TEXTURE_FORMAT
        } else {
            self.screen_format
        };
        // SAFETY: the device and both shaders are alive.
        unsafe {
            create_pipeline(
                self.device,
                self.vertex_shader,
                fragment,
                format,
                self.stencil_format,
                key,
            )
        }
    }

    /// Builds what `frame` needs that does not exist yet: pipelines for
    /// combinations not drawn with before, and stencil textures for render
    /// targets not drawn to before. Anything that cannot be built is left
    /// out, and the draws that need it are skipped.
    fn prepare(&mut self, frame: &Frame) {
        for pass in frame.passes {
            if let Some(id) = pass.target
                && let Some(target) = self.textures.get_mut(&id.0)
                && target.is_target
                && target.stencil.is_null()
            {
                // SAFETY: the device is alive.
                target.stencil = unsafe {
                    new_stencil_texture(
                        self.device,
                        self.stencil_format,
                        target.width,
                        target.height,
                        target.samples,
                    )
                };
                target.stencil_fresh = true;
            }
            let calls = (pass.first_call as usize)
                .checked_add(pass.call_count as usize)
                .and_then(|end| frame.calls.get(pass.first_call as usize..end))
                .unwrap_or(&[]);
            for call in calls {
                let key = self.pipeline_key(call, pass.target);
                if !self.pipelines.contains_key(&key)
                    && let Ok(pipeline) = self.create_pipeline(key)
                {
                    self.pipelines.insert(key, pipeline);
                }
            }
        }
    }

    /// The pipeline `call` draws with.
    fn pipeline_key(&self, call: &DrawCall, target: Option<TextureId>) -> PipelineKey {
        let to_target = target.is_some();
        let layered = call.shader.is_none()
            && call
                .texture
                .and_then(|id| self.textures.get(&id.0))
                .is_some_and(|texture| texture.kind == TextureKind::Array);
        PipelineKey {
            shader: call.shader.map(|id| id.0),
            layered,
            to_target,
            blend: call.blend,
            blend_alpha: call.blend_alpha,
            stencil: call.stencil,
            color_mask: call.color_mask,
            wireframe: call.wireframe,
            samples: self.target_samples(target),
            cull: call.cull,
        }
    }

    /// The texture and sampler for each texture `shader` declares, from
    /// what the game sent, or white where it sent nothing usable.
    fn shader_bindings(
        &self,
        shader: &CustomShader,
        sent: &[Option<BoundTexture>],
    ) -> Vec<SDL_GPUTextureSamplerBinding> {
        shader
            .textures
            .iter()
            .enumerate()
            .map(|(index, &kind)| {
                let found = sent
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|bound| Some((bound, self.textures.get(&bound.texture.0)?)))
                    .filter(|(_, texture)| texture.kind == kind);
                match found {
                    Some((bound, texture)) => SDL_GPUTextureSamplerBinding {
                        texture: texture.handle,
                        sampler: self.sampler(bound.filter, texture.wrap, texture.mipmaps),
                    },
                    None => SDL_GPUTextureSamplerBinding {
                        texture: match kind {
                            TextureKind::Flat => self.white,
                            TextureKind::Array => self.white_array,
                            TextureKind::Cube => self.white_cube,
                        },
                        sampler: self.linear,
                    },
                }
            })
            .collect()
    }

    /// The sampler for these settings, made beforehand by
    /// `ensure_samplers`.
    fn sampler(
        &self,
        filter: Filter,
        wrap: (Wrap, Wrap),
        mipmaps: Option<Mipmaps>,
    ) -> *mut SDL_GPUSampler {
        let clamped = match filter {
            Filter::Linear => self.linear,
            Filter::Nearest => self.nearest,
        };
        self.samplers
            .get(&SamplerKey {
                filter,
                wrap,
                mipmaps,
            })
            .copied()
            .unwrap_or(clamped)
    }

    /// Makes the screen's stencil texture match the screen's size.
    ///
    /// # Safety
    /// The device must be alive.
    unsafe fn resize_screen_stencil(&mut self, width: u32, height: u32) {
        if self.screen_stencil_size == (width, height) && !self.screen_stencil.is_null() {
            return;
        }
        // SAFETY: the old texture came from this device, and SDL ignores
        // null.
        unsafe {
            SDL_ReleaseGPUTexture(self.device, self.screen_stencil);
            SDL_ReleaseGPUTexture(self.device, self.screen_msaa);
            self.screen_stencil = new_stencil_texture(
                self.device,
                self.stencil_format,
                width,
                height,
                self.screen_samples,
            );
            self.screen_msaa = if self.screen_samples > 1 {
                new_msaa_texture(
                    self.device,
                    self.screen_format,
                    width,
                    height,
                    self.screen_samples,
                )
            } else {
                ptr::null_mut()
            };
            // Without the multisampled picture, draw straight to the
            // window rather than not at all.
            if self.screen_samples > 1 && self.screen_msaa.is_null() {
                self.screen_samples = 1;
                SDL_ReleaseGPUTexture(self.device, self.screen_stencil);
                self.screen_stencil =
                    new_stencil_texture(self.device, self.stencil_format, width, height, 1);
            }
        }
        self.screen_stencil_size = (width, height);
        self.screen_stencil_fresh = true;
    }

    /// Makes room for at least `count` vertices.
    fn reserve_vertices(&mut self, count: u32) -> Result<(), String> {
        if count <= self.vertex_capacity {
            return Ok(());
        }
        let capacity = count.next_power_of_two();
        let size = capacity
            .checked_mul(VERTEX_SIZE)
            .ok_or("too many vertices in one frame")?;
        // SAFETY: the device is alive; old buffers are released exactly once
        // and replaced before anything can use them again.
        unsafe {
            let vertex_buffer = SDL_CreateGPUBuffer(
                self.device,
                &SDL_GPUBufferCreateInfo {
                    usage: SDL_GPU_BUFFERUSAGE_VERTEX,
                    size,
                    ..Default::default()
                },
            );
            let transfer_buffer = SDL_CreateGPUTransferBuffer(
                self.device,
                &SDL_GPUTransferBufferCreateInfo {
                    usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
                    size,
                    ..Default::default()
                },
            );
            if vertex_buffer.is_null() || transfer_buffer.is_null() {
                let message = format!("could not allocate vertex buffers: {}", sdl_error());
                SDL_ReleaseGPUBuffer(self.device, vertex_buffer);
                SDL_ReleaseGPUTransferBuffer(self.device, transfer_buffer);
                return Err(message);
            }
            SDL_ReleaseGPUBuffer(self.device, self.vertex_buffer);
            SDL_ReleaseGPUTransferBuffer(self.device, self.transfer_buffer);
            self.vertex_buffer = vertex_buffer;
            self.transfer_buffer = transfer_buffer;
            self.vertex_capacity = capacity;
        }
        Ok(())
    }

    /// Copies `vertices` into the vertex buffer. Returns whether it worked.
    ///
    /// # Safety
    /// `commands` must be a live command buffer with no pass in progress.
    unsafe fn upload_vertices(
        &mut self,
        commands: *mut SDL_GPUCommandBuffer,
        vertices: &[Vertex],
    ) -> bool {
        let Ok(count) = u32::try_from(vertices.len()) else {
            return false;
        };
        if self.reserve_vertices(count).is_err() {
            return false;
        }
        // SAFETY: the buffers hold at least `count` vertices, and cycling
        // gives fresh memory so a frame still in flight is not overwritten.
        unsafe {
            let mapped = SDL_MapGPUTransferBuffer(self.device, self.transfer_buffer, true);
            if mapped.is_null() {
                return false;
            }
            ptr::copy_nonoverlapping(vertices.as_ptr(), mapped.cast::<Vertex>(), vertices.len());
            SDL_UnmapGPUTransferBuffer(self.device, self.transfer_buffer);

            let pass = SDL_BeginGPUCopyPass(commands);
            SDL_UploadToGPUBuffer(
                pass,
                &SDL_GPUTransferBufferLocation {
                    transfer_buffer: self.transfer_buffer,
                    offset: 0,
                },
                &SDL_GPUBufferRegion {
                    buffer: self.vertex_buffer,
                    offset: 0,
                    size: count * VERTEX_SIZE,
                },
                true,
            );
            SDL_EndGPUCopyPass(pass);
        }
        true
    }

    /// Draws `frame` and shows it in `window`.
    pub fn present(&mut self, window: *mut SDL_Window, frame: &Frame) {
        self.render(Some(window), frame);
    }

    /// Draws the passes of `frame` that go to render targets. Passes that
    /// go to the screen are left out.
    pub fn render_offscreen(&mut self, frame: &Frame) {
        self.render(None, frame);
    }

    /// Copies a texture's pixels back from the GPU: four bytes each (red,
    /// green, blue, alpha), in rows from the top. Waits for the GPU to
    /// finish what it was given first.
    pub fn read_texture(&mut self, texture: TextureId) -> Result<(u32, u32, Vec<u8>), String> {
        let Some(&source) = self.textures.get(&texture.0) else {
            return Err("the texture no longer exists".to_string());
        };
        let length = source.width as usize * source.height as usize * 4;
        let size = u32::try_from(length).map_err(|_| "the image is too large".to_string())?;
        // SAFETY: the transfer buffer is `size` bytes, exactly what the
        // texture holds, it is only read once the GPU has filled it, and it
        // is released on every path.
        unsafe {
            let transfer = SDL_CreateGPUTransferBuffer(
                self.device,
                &SDL_GPUTransferBufferCreateInfo {
                    usage: SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD,
                    size,
                    ..Default::default()
                },
            );
            let commands = if transfer.is_null() {
                ptr::null_mut()
            } else {
                SDL_AcquireGPUCommandBuffer(self.device)
            };
            if commands.is_null() {
                let message = format!("could not read a texture: {}", sdl_error());
                SDL_ReleaseGPUTransferBuffer(self.device, transfer);
                return Err(message);
            }
            let pass = SDL_BeginGPUCopyPass(commands);
            SDL_DownloadFromGPUTexture(
                pass,
                &SDL_GPUTextureRegion {
                    texture: source.handle,
                    w: source.width,
                    h: source.height,
                    d: 1,
                    ..Default::default()
                },
                &SDL_GPUTextureTransferInfo {
                    transfer_buffer: transfer,
                    offset: 0,
                    pixels_per_row: source.width,
                    rows_per_layer: source.height,
                },
            );
            SDL_EndGPUCopyPass(pass);
            let fence = SDL_SubmitGPUCommandBufferAndAcquireFence(commands);
            let waited = !fence.is_null() && SDL_WaitForGPUFences(self.device, true, &fence, 1);
            SDL_ReleaseGPUFence(self.device, fence);
            let mapped = if waited {
                SDL_MapGPUTransferBuffer(self.device, transfer, false)
            } else {
                ptr::null_mut()
            };
            if mapped.is_null() {
                let message = format!("could not read a texture: {}", sdl_error());
                SDL_ReleaseGPUTransferBuffer(self.device, transfer);
                return Err(message);
            }
            let mut pixels = vec![0u8; length];
            ptr::copy_nonoverlapping(mapped.cast::<u8>(), pixels.as_mut_ptr(), length);
            SDL_UnmapGPUTransferBuffer(self.device, transfer);
            SDL_ReleaseGPUTransferBuffer(self.device, transfer);
            Ok((source.width, source.height, pixels))
        }
    }

    /// Draws `frame`, to `window` and to render targets, or with no window
    /// to render targets only.
    fn render(&mut self, window: Option<*mut SDL_Window>, frame: &Frame) {
        // SAFETY: passes are begun and ended in the order SDL requires, and
        // the command buffer is always submitted.
        unsafe {
            self.prepare(frame);

            let commands = SDL_AcquireGPUCommandBuffer(self.device);
            if commands.is_null() {
                return;
            }

            let have_geometry = !frame.calls.is_empty()
                && !frame.vertices.is_empty()
                && self.upload_vertices(commands, frame.vertices);

            let mut swapchain: *mut SDL_GPUTexture = ptr::null_mut();
            let (mut screen_width, mut screen_height) = (0u32, 0u32);
            if let Some(window) = window {
                let acquired = SDL_WaitAndAcquireGPUSwapchainTexture(
                    commands,
                    window,
                    &mut swapchain,
                    &mut screen_width,
                    &mut screen_height,
                );
                // The texture is null while the window is minimized.
                if !acquired {
                    swapchain = ptr::null_mut();
                }
            }
            if !swapchain.is_null() && screen_width > 0 && screen_height > 0 {
                self.resize_screen_stencil(screen_width, screen_height);
            }

            for (index, pass) in frame.passes.iter().enumerate() {
                // A pass whose target is gone, or is not there to draw on
                // this frame, is skipped.
                // New stencil memory holds garbage, so its first pass
                // clears it whatever the pass asked for.
                // A multisampled target is drawn in its `msaa` texture,
                // which is resolved into the real one.
                let (texture, msaa, stencil, fresh, width, height) = match pass.target {
                    None if window.is_none() => continue,
                    None => (
                        swapchain,
                        self.screen_msaa,
                        self.screen_stencil,
                        std::mem::take(&mut self.screen_stencil_fresh),
                        screen_width,
                        screen_height,
                    ),
                    Some(id) => match self.textures.get_mut(&id.0) {
                        Some(target) if target.is_target => (
                            target.handle,
                            target.msaa,
                            target.stencil,
                            std::mem::take(&mut target.stencil_fresh),
                            target.width,
                            target.height,
                        ),
                        _ => continue,
                    },
                };
                // Every pipeline expects a stencil texture to be attached.
                if stencil.is_null() {
                    continue;
                }
                if texture.is_null() || width == 0 || height == 0 {
                    continue;
                }

                let mut info = if msaa.is_null() {
                    SDL_GPUColorTargetInfo {
                        texture,
                        load_op: SDL_GPU_LOADOP_LOAD,
                        store_op: SDL_GPU_STOREOP_STORE,
                        ..Default::default()
                    }
                } else {
                    // The samples are kept for later passes to draw on.
                    SDL_GPUColorTargetInfo {
                        texture: msaa,
                        load_op: SDL_GPU_LOADOP_LOAD,
                        store_op: SDL_GPU_STOREOP_RESOLVE_AND_STORE,
                        resolve_texture: texture,
                        ..Default::default()
                    }
                };
                if let Some(color) = pass.clear {
                    info.load_op = SDL_GPU_LOADOP_CLEAR;
                    info.clear_color = SDL_FColor {
                        r: color.r,
                        g: color.g,
                        b: color.b,
                        a: color.a,
                    };
                }
                let stencil_info = SDL_GPUDepthStencilTargetInfo {
                    texture: stencil,
                    // The depth half of the texture is never used.
                    load_op: SDL_GPU_LOADOP_DONT_CARE,
                    store_op: SDL_GPU_STOREOP_DONT_CARE,
                    stencil_load_op: if pass.clear_stencil.is_some() || fresh {
                        SDL_GPU_LOADOP_CLEAR
                    } else {
                        SDL_GPU_LOADOP_LOAD
                    },
                    stencil_store_op: SDL_GPU_STOREOP_STORE,
                    clear_stencil: pass.clear_stencil.unwrap_or(0),
                    ..Default::default()
                };
                let render_pass = SDL_BeginGPURenderPass(commands, &info, 1, &stencil_info);
                if render_pass.is_null() {
                    continue;
                }
                let calls = (pass.first_call as usize)
                    .checked_add(pass.call_count as usize)
                    .and_then(|end| frame.calls.get(pass.first_call as usize..end));
                if let (true, Some(calls)) = (have_geometry, calls) {
                    self.draw_calls(
                        commands,
                        render_pass,
                        calls,
                        frame.uniforms,
                        frame.textures,
                        pass.target,
                        frame.vertices.len() as u32,
                        width,
                        height,
                    );
                }
                SDL_EndGPURenderPass(render_pass);
                // A target that keeps its mipmaps up to date makes them
                // once the passes drawing to it in a row are done.
                if let Some(id) = pass.target
                    && frame
                        .passes
                        .get(index + 1)
                        .is_none_or(|next| next.target != pass.target)
                    && let Some(target) = self.textures.get(&id.0)
                    && target.auto_mipmaps
                {
                    SDL_GenerateMipmapsForGPUTexture(commands, target.handle);
                }
            }

            SDL_SubmitGPUCommandBuffer(commands);
        }
    }

    /// # Safety
    /// `pass` must be an open render pass on `commands` whose target is
    /// `width` by `height` pixels, and the vertex
    /// buffer must already hold `vertex_count` vertices.
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_calls(
        &self,
        commands: *mut SDL_GPUCommandBuffer,
        pass: *mut SDL_GPURenderPass,
        calls: &[DrawCall],
        uniforms: &[u8],
        sent_textures: &[Option<BoundTexture>],
        target: Option<TextureId>,
        vertex_count: u32,
        width: u32,
        height: u32,
    ) {
        // Padded to 16 bytes, the size of the shader's uniform block.
        let pixel_to_clip = [2.0 / width as f32, 2.0 / height as f32, 0.0, 0.0];
        let mut bound_pipeline = ptr::null_mut();
        let mut bound_reference = None;
        let mut bound_scissor = None;

        // SAFETY: every handle bound here is alive, and each draw is checked
        // to stay inside the uploaded vertices.
        unsafe {
            SDL_BindGPUVertexBuffers(
                pass,
                0,
                &SDL_GPUBufferBinding {
                    buffer: self.vertex_buffer,
                    offset: 0,
                },
                1,
            );
            SDL_PushGPUVertexUniformData(
                commands,
                0,
                pixel_to_clip.as_ptr().cast::<c_void>(),
                size_of_val(&pixel_to_clip) as u32,
            );

            for call in calls {
                let in_range = call
                    .first_vertex
                    .checked_add(call.vertex_count)
                    .is_some_and(|end| end <= vertex_count);
                if call.vertex_count == 0 || !in_range {
                    continue;
                }
                // A target cannot be sampled while it is being drawn to.
                if call.texture.is_some() && call.texture == target {
                    continue;
                }
                // A texture that was freed while still queued draws nothing.
                let (texture, wrap, mipmaps, kind) = match call.texture {
                    None => (
                        self.white,
                        (Wrap::Clamp, Wrap::Clamp),
                        None,
                        TextureKind::Flat,
                    ),
                    Some(id) => match self.textures.get(&id.0) {
                        Some(texture) => {
                            (texture.handle, texture.wrap, texture.mipmaps, texture.kind)
                        }
                        None => continue,
                    },
                };
                // A cube cannot be drawn flat, and a custom shader's
                // `image` is always flat.
                let drawable = match kind {
                    TextureKind::Flat => true,
                    TextureKind::Array => call.shader.is_none(),
                    TextureKind::Cube => false,
                };
                if !drawable {
                    continue;
                }
                let Some(scissor) = clip_scissor(call.scissor, width, height) else {
                    continue;
                };
                let key = self.pipeline_key(call, target);
                // A pipeline that could not be built draws nothing.
                let Some(&pipeline) = self.pipelines.get(&key) else {
                    continue;
                };
                let mut extra_bindings = Vec::new();
                if let Some(id) = call.shader {
                    // A shader that was freed while still queued, or whose
                    // data is missing, draws nothing.
                    let Some(shader) = self.shaders.get(&id.0) else {
                        continue;
                    };
                    if !shader.textures.is_empty() {
                        let start = call.texture_offset as usize;
                        let sent = start
                            .checked_add(shader.textures.len())
                            .and_then(|end| sent_textures.get(start..end))
                            .unwrap_or(&[]);
                        extra_bindings = self.shader_bindings(shader, sent);
                    }
                    if shader.uniform_size > 0 {
                        let start = call.uniform_offset as usize;
                        let Some(data) = start
                            .checked_add(shader.uniform_size as usize)
                            .and_then(|end| uniforms.get(start..end))
                        else {
                            continue;
                        };
                        SDL_PushGPUFragmentUniformData(
                            commands,
                            0,
                            data.as_ptr().cast::<c_void>(),
                            shader.uniform_size,
                        );
                    }
                }
                if key.layered {
                    // Padded to 16 bytes, the size of the uniform block.
                    let layer = [call.layer as f32, 0.0, 0.0, 0.0];
                    SDL_PushGPUFragmentUniformData(
                        commands,
                        0,
                        layer.as_ptr().cast::<c_void>(),
                        size_of_val(&layer) as u32,
                    );
                }
                if call.stencil != StencilMode::Off && bound_reference != Some(call.stencil_value) {
                    SDL_SetGPUStencilReference(pass, call.stencil_value);
                    bound_reference = Some(call.stencil_value);
                }
                if bound_pipeline != pipeline {
                    SDL_BindGPUGraphicsPipeline(pass, pipeline);
                    bound_pipeline = pipeline;
                }
                if bound_scissor != Some(scissor) {
                    SDL_SetGPUScissor(
                        pass,
                        &SDL_Rect {
                            x: scissor.0,
                            y: scissor.1,
                            w: scissor.2,
                            h: scissor.3,
                        },
                    );
                    bound_scissor = Some(scissor);
                }
                let sampler = self.sampler(call.filter, wrap, mipmaps);
                let mut bindings = vec![SDL_GPUTextureSamplerBinding { texture, sampler }];
                bindings.extend(extra_bindings);
                SDL_BindGPUFragmentSamplers(pass, 0, bindings.as_ptr(), bindings.len() as u32);
                SDL_DrawGPUPrimitives(pass, call.vertex_count, 1, call.first_vertex, 0);
            }
        }
    }
}

/// Returns the part of `scissor` inside a `width` by `height` target as
/// `(x, y, w, h)`, or `None` if nothing is left to draw in.
fn clip_scissor(scissor: Option<Scissor>, width: u32, height: u32) -> Option<(i32, i32, i32, i32)> {
    let (width, height) = (i64::from(width), i64::from(height));
    let (left, top, right, bottom) = match scissor {
        None => (0, 0, width, height),
        Some(scissor) => {
            let left = i64::from(scissor.x);
            let top = i64::from(scissor.y);
            (
                left.max(0),
                top.max(0),
                (left + i64::from(scissor.width)).min(width),
                (top + i64::from(scissor.height)).min(height),
            )
        }
    };
    if right <= left || bottom <= top {
        return None;
    }
    let convert = |value: i64| i32::try_from(value).ok();
    Some((
        convert(left)?,
        convert(top)?,
        convert(right - left)?,
        convert(bottom - top)?,
    ))
}

/// The GPU blend settings for a blend mode, worked out as LÖVE's
/// `computeBlendState` does.
fn blend_state(mode: BlendMode, alpha: BlendAlphaMode) -> SDL_GPUColorTargetBlendState {
    const ONE: SDL_GPUBlendFactor = SDL_GPU_BLENDFACTOR_ONE;
    const ZERO: SDL_GPUBlendFactor = SDL_GPU_BLENDFACTOR_ZERO;
    const ONE_MINUS_SRC_ALPHA: SDL_GPUBlendFactor = SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA;
    // (source color, destination color, source alpha, destination alpha,
    // operation), for colors that are already premultiplied.
    let (mut src_color, dst_color, src_alpha, dst_alpha, op) = match mode {
        BlendMode::Alpha | BlendMode::Premultiplied => (
            ONE,
            ONE_MINUS_SRC_ALPHA,
            ONE,
            ONE_MINUS_SRC_ALPHA,
            SDL_GPU_BLENDOP_ADD,
        ),
        BlendMode::Add => (ONE, ONE, ZERO, ONE, SDL_GPU_BLENDOP_ADD),
        BlendMode::Subtract => (ONE, ONE, ZERO, ONE, SDL_GPU_BLENDOP_REVERSE_SUBTRACT),
        BlendMode::Multiply => (
            SDL_GPU_BLENDFACTOR_DST_COLOR,
            ZERO,
            SDL_GPU_BLENDFACTOR_DST_ALPHA,
            ZERO,
            SDL_GPU_BLENDOP_ADD,
        ),
        BlendMode::Replace => (ONE, ZERO, ONE, ZERO, SDL_GPU_BLENDOP_ADD),
        BlendMode::Screen => (
            ONE,
            SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_COLOR,
            ONE,
            ONE_MINUS_SRC_ALPHA,
            SDL_GPU_BLENDOP_ADD,
        ),
        // The factors are ignored by the minimum and maximum.
        BlendMode::Lighten => (ONE, ONE, ONE, ONE, SDL_GPU_BLENDOP_MAX),
        BlendMode::Darken => (ONE, ONE, ONE, ONE, SDL_GPU_BLENDOP_MIN),
    };
    // Colors that are not premultiplied yet are multiplied as they land.
    let premultiplied = alpha == BlendAlphaMode::Premultiplied || mode.needs_premultiplied();
    if !premultiplied && src_color == ONE {
        src_color = SDL_GPU_BLENDFACTOR_SRC_ALPHA;
    }
    SDL_GPUColorTargetBlendState {
        enable_blend: true,
        src_color_blendfactor: src_color,
        dst_color_blendfactor: dst_color,
        color_blend_op: op,
        src_alpha_blendfactor: src_alpha,
        dst_alpha_blendfactor: dst_alpha,
        alpha_blend_op: op,
        ..Default::default()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: every handle came from this device and is released once.
        // SDL ignores null handles.
        unsafe {
            for (_, texture) in self.textures.drain() {
                SDL_ReleaseGPUTexture(self.device, texture.handle);
                SDL_ReleaseGPUTexture(self.device, texture.stencil);
                SDL_ReleaseGPUTexture(self.device, texture.msaa);
            }
            SDL_ReleaseGPUTexture(self.device, self.screen_stencil);
            SDL_ReleaseGPUTexture(self.device, self.screen_msaa);
            SDL_ReleaseGPUTexture(self.device, self.white);
            SDL_ReleaseGPUTexture(self.device, self.white_array);
            SDL_ReleaseGPUTexture(self.device, self.white_cube);
            SDL_ReleaseGPUBuffer(self.device, self.vertex_buffer);
            SDL_ReleaseGPUTransferBuffer(self.device, self.transfer_buffer);
            SDL_ReleaseGPUSampler(self.device, self.linear);
            SDL_ReleaseGPUSampler(self.device, self.nearest);
            for &sampler in self.samplers.values() {
                SDL_ReleaseGPUSampler(self.device, sampler);
            }
            for &pipeline in self.pipelines.values() {
                SDL_ReleaseGPUGraphicsPipeline(self.device, pipeline);
            }
            for shader in self.shaders.values() {
                SDL_ReleaseGPUShader(self.device, shader.fragment);
            }
            SDL_ReleaseGPUShader(self.device, self.vertex_shader);
            SDL_ReleaseGPUShader(self.device, self.fragment_shader);
            SDL_ReleaseGPUShader(self.device, self.array_fragment_shader);
        }
    }
}

/// # Safety
/// `device` and both shaders must be alive.
unsafe fn create_pipeline(
    device: *mut SDL_GPUDevice,
    vertex_shader: *mut SDL_GPUShader,
    fragment_shader: *mut SDL_GPUShader,
    format: SDL_GPUTextureFormat,
    stencil_format: SDL_GPUTextureFormat,
    key: PipelineKey,
) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
    let PipelineKey {
        blend,
        samples,
        blend_alpha,
        stencil,
        color_mask,
        wireframe,
        cull,
        ..
    } = key;
    // SAFETY: all pointers in the create-info structs outlive the calls
    // that read them.
    unsafe {
        let buffers = [SDL_GPUVertexBufferDescription {
            slot: 0,
            pitch: VERTEX_SIZE,
            input_rate: SDL_GPU_VERTEXINPUTRATE_VERTEX,
            instance_step_rate: 0,
        }];
        let attribute = |location, format, offset: usize| SDL_GPUVertexAttribute {
            location,
            buffer_slot: 0,
            format,
            offset: offset as u32,
        };
        let attributes = [
            attribute(
                0,
                SDL_GPU_VERTEXELEMENTFORMAT_FLOAT2,
                std::mem::offset_of!(Vertex, position),
            ),
            attribute(
                1,
                SDL_GPU_VERTEXELEMENTFORMAT_FLOAT2,
                std::mem::offset_of!(Vertex, uv),
            ),
            attribute(
                2,
                SDL_GPU_VERTEXELEMENTFORMAT_FLOAT4,
                std::mem::offset_of!(Vertex, color),
            ),
        ];

        let mut blend_state = blend_state(blend, blend_alpha);
        // Writing to the stencil leaves the picture itself untouched.
        if matches!(stencil, StencilMode::Write(_)) {
            blend_state.enable_color_write_mask = true;
            blend_state.color_write_mask = SDL_GPUColorComponentFlags(0);
        } else if color_mask != ColorMask::ALL {
            let mut flags = 0;
            for (on, flag) in [
                (color_mask.r, SDL_GPU_COLORCOMPONENT_R),
                (color_mask.g, SDL_GPU_COLORCOMPONENT_G),
                (color_mask.b, SDL_GPU_COLORCOMPONENT_B),
                (color_mask.a, SDL_GPU_COLORCOMPONENT_A),
            ] {
                if on {
                    flags |= flag.0;
                }
            }
            blend_state.enable_color_write_mask = true;
            blend_state.color_write_mask = SDL_GPUColorComponentFlags(flags);
        }
        let targets = [SDL_GPUColorTargetDescription {
            format,
            blend_state,
        }];

        let info = SDL_GPUGraphicsPipelineCreateInfo {
            vertex_shader,
            fragment_shader,
            vertex_input_state: SDL_GPUVertexInputState {
                vertex_buffer_descriptions: buffers.as_ptr(),
                num_vertex_buffers: buffers.len() as u32,
                vertex_attributes: attributes.as_ptr(),
                num_vertex_attributes: attributes.len() as u32,
            },
            primitive_type: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
            multisample_state: SDL_GPUMultisampleState {
                sample_count: sample_count(samples),
                ..Default::default()
            },
            rasterizer_state: SDL_GPURasterizerState {
                fill_mode: if wireframe {
                    SDL_GPU_FILLMODE_LINE
                } else {
                    SDL_GPU_FILLMODE_FILL
                },
                // The GPU keeps the triangles wound the front-face way,
                // and windings read the same to it as on screen.
                cull_mode: if cull.is_some() {
                    SDL_GPU_CULLMODE_BACK
                } else {
                    SDL_GPU_CULLMODE_NONE
                },
                front_face: match cull {
                    Some(VertexWinding::Clockwise) => SDL_GPU_FRONTFACE_COUNTER_CLOCKWISE,
                    _ => SDL_GPU_FRONTFACE_CLOCKWISE,
                },
                ..Default::default()
            },
            target_info: SDL_GPUGraphicsPipelineTargetInfo {
                color_target_descriptions: targets.as_ptr(),
                num_color_targets: targets.len() as u32,
                depth_stencil_format: stencil_format,
                has_depth_stencil_target: true,
                ..Default::default()
            },
            depth_stencil_state: stencil_state(stencil),
            ..Default::default()
        };
        let pipeline = SDL_CreateGPUGraphicsPipeline(device, &info);

        if pipeline.is_null() {
            return Err(format!(
                "could not create the draw pipeline: {}",
                sdl_error()
            ));
        }
        Ok(pipeline)
    }
}

/// The GPU stencil settings for a stencil mode.
fn stencil_state(mode: StencilMode) -> SDL_GPUDepthStencilState {
    let (pass_op, compare_op, write_mask) = match mode {
        StencilMode::Off => return SDL_GPUDepthStencilState::default(),
        StencilMode::Write(action) => (
            match action {
                StencilAction::Replace => SDL_GPU_STENCILOP_REPLACE,
                StencilAction::Increment => SDL_GPU_STENCILOP_INCREMENT_AND_CLAMP,
                StencilAction::Decrement => SDL_GPU_STENCILOP_DECREMENT_AND_CLAMP,
                StencilAction::IncrementWrap => SDL_GPU_STENCILOP_INCREMENT_AND_WRAP,
                StencilAction::DecrementWrap => SDL_GPU_STENCILOP_DECREMENT_AND_WRAP,
                StencilAction::Invert => SDL_GPU_STENCILOP_INVERT,
            },
            SDL_GPU_COMPAREOP_ALWAYS,
            0xFF,
        ),
        // The GPU compares the test value with the stencil value, the
        // other way round from how `CompareMode` reads, so each is flipped.
        StencilMode::Test(compare) => (
            SDL_GPU_STENCILOP_KEEP,
            match compare {
                CompareMode::Equal => SDL_GPU_COMPAREOP_EQUAL,
                CompareMode::NotEqual => SDL_GPU_COMPAREOP_NOT_EQUAL,
                CompareMode::Less => SDL_GPU_COMPAREOP_GREATER,
                CompareMode::LessEqual => SDL_GPU_COMPAREOP_GREATER_OR_EQUAL,
                CompareMode::Greater => SDL_GPU_COMPAREOP_LESS,
                CompareMode::GreaterEqual => SDL_GPU_COMPAREOP_LESS_OR_EQUAL,
                CompareMode::Never => SDL_GPU_COMPAREOP_NEVER,
                CompareMode::Always => SDL_GPU_COMPAREOP_ALWAYS,
            },
            0,
        ),
    };
    let face = SDL_GPUStencilOpState {
        fail_op: SDL_GPU_STENCILOP_KEEP,
        pass_op,
        depth_fail_op: SDL_GPU_STENCILOP_KEEP,
        compare_op,
    };
    SDL_GPUDepthStencilState {
        front_stencil_state: face,
        back_stencil_state: face,
        compare_mask: 0xFF,
        write_mask,
        enable_stencil_test: true,
        ..Default::default()
    }
}

/// Creates a texture to hold stencil values, or null if the GPU refuses.
/// Its contents start out undefined; the first pass that uses it clears it.
///
/// # Safety
/// `device` must be alive.
unsafe fn new_stencil_texture(
    device: *mut SDL_GPUDevice,
    format: SDL_GPUTextureFormat,
    width: u32,
    height: u32,
    samples: u32,
) -> *mut SDL_GPUTexture {
    // SAFETY: the create-info is valid for the call.
    unsafe {
        SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type: SDL_GPU_TEXTURETYPE_2D,
                format,
                usage: SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET,
                width,
                height,
                layer_count_or_depth: 1,
                num_levels: 1,
                sample_count: sample_count(samples),
                ..Default::default()
            },
        )
    }
}

/// A multisampled color texture to draw in and resolve from. Null if the
/// GPU refuses.
///
/// # Safety
/// `device` must be alive.
unsafe fn new_msaa_texture(
    device: *mut SDL_GPUDevice,
    format: SDL_GPUTextureFormat,
    width: u32,
    height: u32,
    samples: u32,
) -> *mut SDL_GPUTexture {
    // SAFETY: the create-info is valid for the call.
    unsafe {
        SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type: SDL_GPU_TEXTURETYPE_2D,
                format,
                usage: SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                width,
                height,
                layer_count_or_depth: 1,
                num_levels: 1,
                sample_count: sample_count(samples),
                ..Default::default()
            },
        )
    }
}

/// SDL's name for a number of samples per pixel.
fn sample_count(samples: u32) -> SDL_GPUSampleCount {
    match samples {
        8.. => SDL_GPU_SAMPLECOUNT_8,
        4..=7 => SDL_GPU_SAMPLECOUNT_4,
        2..=3 => SDL_GPU_SAMPLECOUNT_2,
        _ => SDL_GPU_SAMPLECOUNT_1,
    }
}

/// # Safety
/// `device` must be alive.
unsafe fn create_shader(
    device: *mut SDL_GPUDevice,
    spirv: &[u8],
    entrypoint: &CStr,
    stage: SDL_GPUShaderStage,
    num_samplers: u32,
    num_uniform_buffers: u32,
) -> Result<*mut SDL_GPUShader, String> {
    let info = SDL_GPUShaderCreateInfo {
        code_size: spirv.len(),
        code: spirv.as_ptr(),
        entrypoint: entrypoint.as_ptr(),
        format: SDL_GPU_SHADERFORMAT_SPIRV,
        stage,
        num_samplers,
        num_uniform_buffers,
        ..Default::default()
    };
    // SAFETY: `info` and the bytes it points at are valid for the call.
    let shader = unsafe { SDL_CreateGPUShader(device, &info) };
    if shader.is_null() {
        return Err(format!("could not load a shader: {}", sdl_error()));
    }
    Ok(shader)
}

/// # Safety
/// `device` must be alive.
unsafe fn create_sampler(
    device: *mut SDL_GPUDevice,
    key: SamplerKey,
) -> Result<*mut SDL_GPUSampler, String> {
    let filter = match key.filter {
        Filter::Linear => SDL_GPU_FILTER_LINEAR,
        Filter::Nearest => SDL_GPU_FILTER_NEAREST,
    };
    let address_mode = |wrap: Wrap| match wrap {
        Wrap::Clamp => SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        Wrap::Repeat => SDL_GPU_SAMPLERADDRESSMODE_REPEAT,
        Wrap::MirroredRepeat => SDL_GPU_SAMPLERADDRESSMODE_MIRRORED_REPEAT,
    };
    let mut info = SDL_GPUSamplerCreateInfo {
        min_filter: filter,
        mag_filter: filter,
        address_mode_u: address_mode(key.wrap.0),
        address_mode_v: address_mode(key.wrap.1),
        address_mode_w: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        ..Default::default()
    };
    // Without mipmaps the level of detail stays at 0, the full picture.
    if let Some(mipmaps) = key.mipmaps {
        info.mipmap_mode = match mipmaps.filter {
            Filter::Linear => SDL_GPU_SAMPLERMIPMAPMODE_LINEAR,
            Filter::Nearest => SDL_GPU_SAMPLERMIPMAPMODE_NEAREST,
        };
        // Sharper means a larger mipmap, as in LÖVE.
        info.mip_lod_bias = -f32::from_bits(mipmaps.sharpness);
        info.max_lod = 1000.0;
    }
    // SAFETY: `info` is valid for the call.
    let sampler = unsafe { SDL_CreateGPUSampler(device, &info) };
    if sampler.is_null() {
        return Err(format!("could not create a sampler: {}", sdl_error()));
    }
    Ok(sampler)
}

impl SamplerKey {
    /// The sampler for a texture that clamps and has no mipmaps.
    fn clamped(filter: Filter) -> SamplerKey {
        SamplerKey {
            filter,
            wrap: (Wrap::Clamp, Wrap::Clamp),
            mipmaps: None,
        }
    }
}

/// The SDL format for a compressed format.
fn compressed_format(format: CompressedFormat) -> SDL_GPUTextureFormat {
    match format {
        CompressedFormat::Bc1 => SDL_GPU_TEXTUREFORMAT_BC1_RGBA_UNORM,
        CompressedFormat::Bc2 => SDL_GPU_TEXTUREFORMAT_BC2_RGBA_UNORM,
        CompressedFormat::Bc3 => SDL_GPU_TEXTUREFORMAT_BC3_RGBA_UNORM,
        CompressedFormat::Bc4 => SDL_GPU_TEXTUREFORMAT_BC4_R_UNORM,
        CompressedFormat::Bc5 => SDL_GPU_TEXTUREFORMAT_BC5_RG_UNORM,
        CompressedFormat::Bc7 => SDL_GPU_TEXTUREFORMAT_BC7_RGBA_UNORM,
    }
}

/// The size of mipmap `level` of a `width` by `height` texture.
fn level_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// How many mipmap levels a full chain for a `width` by `height` texture
/// has, down to 1 by 1.
fn mipmap_levels(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// The bytes of `levels` mipmap levels of a `width` by `height` texture.
fn mipmap_bytes(width: u32, height: u32, levels: u32) -> u64 {
    (0..levels)
        .map(|level| u64::from((width >> level).max(1)) * u64::from((height >> level).max(1)) * 4)
        .sum()
}

/// Fills every smaller mipmap level of `texture` from the full-size one.
///
/// # Safety
/// `device` and `texture` must be alive, and the texture must have been
/// made to be drawn to.
unsafe fn generate_mipmaps(
    device: *mut SDL_GPUDevice,
    texture: *mut SDL_GPUTexture,
) -> Result<(), String> {
    // SAFETY: the command buffer is submitted on the only path that gets
    // one, with no pass open.
    unsafe {
        let commands = SDL_AcquireGPUCommandBuffer(device);
        if commands.is_null() {
            return Err(format!("could not make mipmaps: {}", sdl_error()));
        }
        SDL_GenerateMipmapsForGPUTexture(commands, texture);
        SDL_SubmitGPUCommandBuffer(commands);
    }
    Ok(())
}

fn check_pixel_count(width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "{width}x{height} pixels need {expected} bytes of RGBA data, got {}",
            rgba.len()
        ))
    }
}

/// Creates a texture and fills it with `rgba`.
///
/// # Safety
/// `device` must be alive and `rgba` must hold `width * height * 4` bytes.
unsafe fn new_texture(
    device: *mut SDL_GPUDevice,
    width: u32,
    height: u32,
    levels: u32,
    rgba: &[u8],
) -> Result<*mut SDL_GPUTexture, String> {
    // Mipmaps are made by drawing each level from the one above.
    let usage = if levels > 1 {
        SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET
    } else {
        SDL_GPU_TEXTUREUSAGE_SAMPLER
    };
    // SAFETY: the create-info is valid for the call, and the new texture is
    // released if filling it fails.
    unsafe {
        let texture = SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type: SDL_GPU_TEXTURETYPE_2D,
                format: TEXTURE_FORMAT,
                usage,
                width,
                height,
                layer_count_or_depth: 1,
                num_levels: levels,
                sample_count: SDL_GPU_SAMPLECOUNT_1,
                ..Default::default()
            },
        );
        if texture.is_null() {
            return Err(format!("could not create a texture: {}", sdl_error()));
        }
        let filled = upload_region(device, texture, 0, 0, 0, width, height, rgba).and_then(|()| {
            if levels > 1 {
                generate_mipmaps(device, texture)
            } else {
                Ok(())
            }
        });
        if let Err(message) = filled {
            SDL_ReleaseGPUTexture(device, texture);
            return Err(message);
        }
        Ok(texture)
    }
}

/// Creates an array or cube texture and fills each layer.
///
/// # Safety
/// `device` must be alive, `kind` must not be flat, and each layer must
/// hold `width * height * 4` bytes.
unsafe fn new_layered_texture(
    device: *mut SDL_GPUDevice,
    kind: TextureKind,
    width: u32,
    height: u32,
    layers: &[&[u8]],
) -> Result<*mut SDL_GPUTexture, String> {
    let r#type = match kind {
        TextureKind::Cube => SDL_GPU_TEXTURETYPE_CUBE,
        _ => SDL_GPU_TEXTURETYPE_2D_ARRAY,
    };
    // SAFETY: the create-info is valid for the call, and the new texture
    // is released if filling it fails.
    unsafe {
        let texture = SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type,
                format: TEXTURE_FORMAT,
                usage: SDL_GPU_TEXTUREUSAGE_SAMPLER,
                width,
                height,
                layer_count_or_depth: layers.len() as u32,
                num_levels: 1,
                sample_count: SDL_GPU_SAMPLECOUNT_1,
                ..Default::default()
            },
        );
        if texture.is_null() {
            return Err(format!("could not create a texture: {}", sdl_error()));
        }
        for (index, layer) in layers.iter().enumerate() {
            if let Err(message) =
                upload_region(device, texture, index as u32, 0, 0, width, height, layer)
            {
                SDL_ReleaseGPUTexture(device, texture);
                return Err(message);
            }
        }
        Ok(texture)
    }
}

/// Where in a texture an upload goes.
#[derive(Clone, Copy, Default)]
struct Region {
    layer: u32,
    level: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Copies `rgba` into a region of `texture`.
///
/// # Safety
/// `device` and `texture` must be alive, the region must lie inside the
/// texture, and `rgba` must hold `width * height * 4` bytes.
#[allow(clippy::too_many_arguments)]
unsafe fn upload_region(
    device: *mut SDL_GPUDevice,
    texture: *mut SDL_GPUTexture,
    layer: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), String> {
    let region = Region {
        layer,
        level: 0,
        x,
        y,
        width,
        height,
    };
    // SAFETY: as the caller promised.
    unsafe { upload(device, texture, region, rgba) }
}

/// Copies `data` into `region` of `texture`, as tightly packed rows of
/// pixels or of compressed blocks.
///
/// # Safety
/// `device` and `texture` must be alive, the region must lie inside the
/// texture, and `data` must hold exactly the region's pixels in the
/// texture's format.
unsafe fn upload(
    device: *mut SDL_GPUDevice,
    texture: *mut SDL_GPUTexture,
    region: Region,
    rgba: &[u8],
) -> Result<(), String> {
    let size = u32::try_from(rgba.len()).map_err(|_| "the image is too large".to_string())?;
    // SAFETY: the transfer buffer is `size` bytes, exactly what is copied
    // in, and it is released on every path.
    unsafe {
        let transfer = SDL_CreateGPUTransferBuffer(
            device,
            &SDL_GPUTransferBufferCreateInfo {
                usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
                size,
                ..Default::default()
            },
        );
        let mapped = if transfer.is_null() {
            ptr::null_mut()
        } else {
            SDL_MapGPUTransferBuffer(device, transfer, false)
        };
        let commands = if mapped.is_null() {
            ptr::null_mut()
        } else {
            ptr::copy_nonoverlapping(rgba.as_ptr(), mapped.cast::<u8>(), rgba.len());
            SDL_UnmapGPUTransferBuffer(device, transfer);
            SDL_AcquireGPUCommandBuffer(device)
        };
        if commands.is_null() {
            let message = format!("could not upload a texture: {}", sdl_error());
            SDL_ReleaseGPUTransferBuffer(device, transfer);
            return Err(message);
        }

        let pass = SDL_BeginGPUCopyPass(commands);
        SDL_UploadToGPUTexture(
            pass,
            // Zero means the rows are tightly packed, which also suits
            // compressed blocks.
            &SDL_GPUTextureTransferInfo {
                transfer_buffer: transfer,
                offset: 0,
                pixels_per_row: 0,
                rows_per_layer: 0,
            },
            &SDL_GPUTextureRegion {
                texture,
                mip_level: region.level,
                layer: region.layer,
                x: region.x,
                y: region.y,
                w: region.width,
                h: region.height,
                d: 1,
                ..Default::default()
            },
            false,
        );
        SDL_EndGPUCopyPass(pass);
        SDL_SubmitGPUCommandBuffer(commands);
        SDL_ReleaseGPUTransferBuffer(device, transfer);
        Ok(())
    }
}

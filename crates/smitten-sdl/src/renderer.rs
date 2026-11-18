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
    BoundTexture, BufferId, BufferUsage, ColorMask, CompressedFormat, DrawCall, Frame,
    IndexDataType, MAX_VERTEX_ATTRIBUTES, MAX_VERTEX_BUFFERS, MeshDraw, Pass, RenderTargetDesc,
    RendererInfo, Scissor, ShaderId, ShaderInfo, TargetSlice, TextureId, TextureKind, Vertex,
    VertexDataType, VertexLayout, VertexLayoutId as MeshVertexLayoutId, VertexStep,
};
use smitten_core::{
    BlendAlphaMode, BlendMode, CompareMode, DepthMode, Filter, PixelFormat, StencilAction,
    StencilMode, VertexWinding, Wrap,
};

use crate::{sdl_error, shader};

// Compiled from the GLSL beside them by `shaders/build.sh`.
static VERTEX_SPIRV: &[u8] = include_bytes!("../shaders/sprite.vert.spv");
static FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite.frag.spv");
static ARRAY_FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite_array.frag.spv");

const VERTEX_SIZE: u32 = size_of::<Vertex>() as u32;
const INITIAL_VERTEX_CAPACITY: u32 = 4096;
/// The format images and render targets are made in unless asked for
/// another. The screen may differ.
const DEFAULT_TEXTURE_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
/// The most color targets one pass can draw to, as SDL allows.
const MAX_COLOR_TARGETS: usize = 4;

/// What a pipeline draws to: the format of each color target, how many
/// samples each pixel holds, and the depth and stencil format.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TargetDesc {
    /// The format of each color target in order. Only the first
    /// `color_count` are used; the rest stay invalid.
    colors: [SDL_GPUTextureFormat; MAX_COLOR_TARGETS],
    color_count: u8,
    /// How many samples each pixel of every target holds.
    samples: u32,
    depth_stencil: SDL_GPUTextureFormat,
}

impl TargetDesc {
    /// A single color target.
    fn single(
        color: SDL_GPUTextureFormat,
        samples: u32,
        depth_stencil: SDL_GPUTextureFormat,
    ) -> Self {
        let mut colors = [SDL_GPU_TEXTUREFORMAT_INVALID; MAX_COLOR_TARGETS];
        colors[0] = color;
        Self {
            colors,
            color_count: 1,
            samples,
            depth_stencil,
        }
    }

    /// The formats of the color targets in use.
    fn colors(&self) -> &[SDL_GPUTextureFormat] {
        &self.colors[..usize::from(self.color_count)]
    }
}

/// Which arrangement of vertex buffers and attributes a pipeline reads.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct VertexLayoutId(u32);

impl VertexLayoutId {
    /// One buffer of [`Vertex`], already in pixels of the target, which
    /// is what every draw but a mesh's uses. Mesh layouts count from 1.
    const STANDARD: Self = Self(0);
}

/// A vertex layout for meshes, and the built-in vertex shader for it.
struct MeshLayout {
    desc: VertexLayout,
    vertex_shader: *mut SDL_GPUShader,
    /// The index into `desc.attributes` of what each input location of
    /// the shader reads.
    locations: Vec<usize>,
}

/// A buffer the game keeps on the GPU.
#[derive(Clone, Copy)]
struct GpuBuffer {
    handle: *mut SDL_GPUBuffer,
    usage: BufferUsage,
    /// How many bytes it holds.
    size: u32,
}

/// The vertex buffers and attributes a pipeline reads.
struct VertexInput {
    buffers: Vec<SDL_GPUVertexBufferDescription>,
    attributes: Vec<SDL_GPUVertexAttribute>,
}

/// Everything that needs its own pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    /// A custom shader, or `None` for the built-in one.
    shader: Option<u32>,
    /// Whether the built-in shader draws a layer of an array texture.
    layered: bool,
    /// What the pipeline draws to.
    target: TargetDesc,
    /// How the pipeline reads its vertices.
    layout: VertexLayoutId,
    blend: BlendMode,
    blend_alpha: BlendAlphaMode,
    stencil: StencilMode,
    depth: DepthMode,
    color_mask: ColorMask,
    wireframe: bool,
    /// The on-screen winding of the triangles to leave out.
    cull: Option<VertexWinding>,
}

#[derive(Clone, Copy)]
struct Texture {
    handle: *mut SDL_GPUTexture,
    /// The format of `handle`, and of `msaa` if there is one.
    format: SDL_GPUTextureFormat,
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
    /// How many layers an array has, or how deep a volume is: 6 for a
    /// cube and 1 for a flat texture.
    layers: u32,
    /// Whether the pixels are compressed, which means they cannot be
    /// replaced.
    compressed: bool,
    /// Whether a render target's mipmaps are made again after each run of
    /// passes that draws to it.
    auto_mipmaps: bool,
    /// How a readable depth target is compared when a shader samples it
    /// as a depth texture, or `None` to read plain depth values.
    depth_compare: Option<CompareMode>,
    /// Whether shaders can sample it. Only a depth target can be made
    /// without.
    readable: bool,
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
    /// For a depth texture sampled by comparing, how it compares.
    compare: Option<CompareMode>,
}

/// A fragment shader supplied by the game.
struct CustomShader {
    fragment: *mut SDL_GPUShader,
    /// The game's WGSL, compiled again for drawing to several canvases.
    source: String,
    /// How many bytes of uniform data each draw must supply.
    uniform_size: u32,
    /// The kinds of the textures it declares besides `image`, and whether
    /// each is a depth texture sampled by comparing.
    textures: Vec<(TextureKind, bool)>,
    /// Whether the game wrote a vertex shader, `position`, which is
    /// compiled for each vertex layout it is drawn with.
    has_vertex: bool,
    /// Its vertex shader for each layout drawn with so far, and the index
    /// into the layout's attributes each input location reads.
    vertex_variants: HashMap<VertexLayoutId, (*mut SDL_GPUShader, Vec<usize>)>,
}

impl CustomShader {
    /// Frees its GPU shaders.
    ///
    /// # Safety
    ///
    /// They must have come from `device`, which must be alive.
    unsafe fn release(&self, device: *mut SDL_GPUDevice) {
        // SAFETY: as the caller promises. SDL defers the free until no
        // in-flight frame uses them.
        unsafe {
            SDL_ReleaseGPUShader(device, self.fragment);
            for &(variant, _) in self.vertex_variants.values() {
                SDL_ReleaseGPUShader(device, variant);
            }
        }
    }
}

pub(crate) struct Renderer {
    device: *mut SDL_GPUDevice,
    screen_format: SDL_GPUTextureFormat,
    /// The vertex shader for vertices already in pixels.
    vertex_shader: *mut SDL_GPUShader,
    /// The layouts meshes have been drawn with, by id, and the id of each.
    mesh_layouts: HashMap<u32, MeshLayout>,
    mesh_layout_ids: HashMap<VertexLayout, u32>,
    buffers: HashMap<u32, GpuBuffer>,
    next_buffer_id: u32,
    shaders: HashMap<u32, CustomShader>,
    next_shader_id: u32,
    /// The built-in fragment shader.
    fragment_shader: *mut SDL_GPUShader,
    /// The built-in fragment shader for a layer of an array texture.
    array_fragment_shader: *mut SDL_GPUShader,
    /// Built the first time each combination is drawn with.
    pipelines: HashMap<PipelineKey, *mut SDL_GPUGraphicsPipeline>,
    /// Fragment shaders that write to several canvases at once, made the
    /// first time each is needed, by custom shader (or `None` for the
    /// built-in one), whether it draws an array texture, and how many
    /// canvases.
    fragment_variants: HashMap<(Option<u32>, bool, u8), *mut SDL_GPUShader>,
    /// The depth and stencil format of a canvas's own values.
    stencil_format: SDL_GPUTextureFormat,
    /// The depth and stencil format of the screen's, as the game asked.
    screen_depth_format: SDL_GPUTextureFormat,
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
    white_volume: *mut SDL_GPUTexture,
    /// A stand-in for a depth texture a shader declares but was not sent,
    /// made the first time one is needed, at the far depth of 1, and the
    /// comparing sampler that goes with it.
    white_depth: *mut SDL_GPUTexture,
    comparing: *mut SDL_GPUSampler,
    textures: HashMap<u32, Texture>,
    next_texture_id: u32,
    vertex_buffer: *mut SDL_GPUBuffer,
    transfer_buffer: *mut SDL_GPUTransferBuffer,
    /// How many vertices the two buffers above can hold.
    vertex_capacity: u32,
    /// Whether the screen is sRGB-encoded, so vertex and clear colors,
    /// which games give in sRGB, are converted to linear light.
    gamma_correct: bool,
}

impl Renderer {
    /// Builds the pipeline for `window`, which `device` must already own.
    /// With `gamma_correct`, the window's swapchain is sRGB-encoded.
    /// `screen_depth` is the bits of depth the screen asks for, and
    /// whether it has a stencil buffer.
    pub fn new(
        device: *mut SDL_GPUDevice,
        window: *mut SDL_Window,
        gamma_correct: bool,
        screen_depth: (Option<u8>, bool),
    ) -> Result<Self, String> {
        let mut renderer = Self {
            device,
            screen_format: SDL_GPU_TEXTUREFORMAT_INVALID,
            vertex_shader: ptr::null_mut(),
            mesh_layouts: HashMap::new(),
            mesh_layout_ids: HashMap::new(),
            buffers: HashMap::new(),
            next_buffer_id: 1,
            shaders: HashMap::new(),
            next_shader_id: 1,
            fragment_shader: ptr::null_mut(),
            array_fragment_shader: ptr::null_mut(),
            pipelines: HashMap::new(),
            fragment_variants: HashMap::new(),
            stencil_format: SDL_GPU_TEXTUREFORMAT_INVALID,
            screen_depth_format: SDL_GPU_TEXTUREFORMAT_INVALID,
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
            white_volume: ptr::null_mut(),
            white_depth: ptr::null_mut(),
            comparing: ptr::null_mut(),
            textures: HashMap::new(),
            next_texture_id: 1,
            vertex_buffer: ptr::null_mut(),
            transfer_buffer: ptr::null_mut(),
            vertex_capacity: 0,
            gamma_correct,
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
            renderer.screen_depth_format = screen_depth_formats(screen_depth)
                .iter()
                .copied()
                .find(|&format| {
                    SDL_GPUTextureSupportsFormat(
                        device,
                        format,
                        SDL_GPU_TEXTURETYPE_2D,
                        SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET,
                    )
                })
                .unwrap_or(renderer.stencil_format);
            // Build the pipeline nearly every frame uses now, so a broken
            // setup fails at startup rather than drawing nothing.
            let common = PipelineKey {
                shader: None,
                layered: false,
                target: TargetDesc::single(screen_format, 1, renderer.screen_depth_format),
                layout: VertexLayoutId::STANDARD,
                blend: BlendMode::Alpha,
                blend_alpha: BlendAlphaMode::AlphaMultiply,
                stencil: StencilMode::Off,
                depth: DepthMode::default(),
                color_mask: ColorMask::ALL,
                wireframe: false,
                cull: None,
            };
            let pipeline = renderer.create_pipeline(common)?;
            renderer.pipelines.insert(common, pipeline);
            renderer.linear = create_sampler(device, SamplerKey::clamped(Filter::Linear))?;
            renderer.nearest = create_sampler(device, SamplerKey::clamped(Filter::Nearest))?;
            renderer.white = new_texture(
                device,
                DEFAULT_TEXTURE_FORMAT,
                1,
                1,
                1,
                &[255, 255, 255, 255],
            )?;
            let white: &[u8] = &[255; 4];
            renderer.white_array = new_layered_texture(
                device,
                DEFAULT_TEXTURE_FORMAT,
                TextureKind::Array,
                1,
                1,
                &[white],
            )?;
            renderer.white_cube = new_layered_texture(
                device,
                DEFAULT_TEXTURE_FORMAT,
                TextureKind::Cube,
                1,
                1,
                &[white; 6],
            )?;
            renderer.white_volume = new_layered_texture(
                device,
                DEFAULT_TEXTURE_FORMAT,
                TextureKind::Volume,
                1,
                1,
                &[white],
            )?;
            // Every GPU can sample 16-bit depth.
            renderer.white_depth =
                new_depth_texture(device, SDL_GPU_TEXTUREFORMAT_D16_UNORM, 1, 1, 1, true);
            if renderer.white_depth.is_null() {
                return Err(format!("could not create a depth texture: {}", sdl_error()));
            }
            renderer.comparing = create_sampler(
                device,
                SamplerKey {
                    compare: Some(CompareMode::LessEqual),
                    ..SamplerKey::clamped(Filter::Linear)
                },
            )?;
        }
        renderer.reserve_vertices(INITIAL_VERTEX_CAPACITY)?;
        Ok(renderer)
    }

    /// Creates a texture from `pixels` in `format`.
    pub fn create_texture(
        &mut self,
        format: PixelFormat,
        width: u32,
        height: u32,
        pixels: &[u8],
        mipmapped: bool,
    ) -> Result<TextureId, String> {
        if width == 0 || height == 0 {
            return Err("an image must be at least 1x1 pixels".to_string());
        }
        if !self.supports_pixel_format(format, false) {
            return Err(format!("this GPU cannot use {format} images"));
        }
        let format = sdl_pixel_format(format);
        check_pixel_count(width, height, pixels, format)?;
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
        // SAFETY: the device is alive and `pixels` was just checked to
        // cover the whole texture.
        let handle = unsafe { new_texture(self.device, format, width, height, levels, pixels)? };
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                format,
                width,
                height,
                is_target: false,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa: ptr::null_mut(),
                samples: 1,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: mipmap_bytes(format, width, height, levels),
                levels,
                mipmaps,
                kind: TextureKind::Flat,
                layers: 1,
                compressed: false,
                auto_mipmaps: false,
                depth_compare: None,
                readable: true,
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

    /// Whether the GPU can sample textures in `format`, and with
    /// `render_target`, draw to them too.
    pub fn supports_pixel_format(&self, format: PixelFormat, render_target: bool) -> bool {
        let usage = match (format.is_depth(), render_target) {
            (true, true) => SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET,
            (true, false) => {
                SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER
            }
            (false, true) => SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            (false, false) => SDL_GPU_TEXTUREUSAGE_SAMPLER,
        };
        // SAFETY: the device is alive.
        unsafe {
            SDL_GPUTextureSupportsFormat(
                self.device,
                sdl_pixel_format(format),
                SDL_GPU_TEXTURETYPE_2D,
                usage,
            )
        }
    }

    pub fn create_compressed_texture(
        &mut self,
        format: CompressedFormat,
        srgb: bool,
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
                    "level {level} of a {width}x{height} {} image needs {expected} bytes, got {}",
                    format.name(),
                    data.len()
                ));
            }
        }
        if !self.supports_compressed_format(format) {
            return Err(format!("this GPU cannot read {} images", format.name()));
        }
        let level_count = levels.len() as u32;
        // Fall back to the linear version where there is no sRGB one.
        let format = srgb
            .then(|| compressed_srgb_format(format))
            .flatten()
            .filter(|&format| {
                // SAFETY: the device is alive.
                unsafe {
                    SDL_GPUTextureSupportsFormat(
                        self.device,
                        format,
                        SDL_GPU_TEXTURETYPE_2D,
                        SDL_GPU_TEXTUREUSAGE_SAMPLER,
                    )
                }
            })
            .unwrap_or_else(|| compressed_format(format));
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
                    format,
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
                format,
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
                layers: 1,
                compressed: true,
                auto_mipmaps: false,
                depth_compare: None,
                readable: true,
            },
        );
        Ok(TextureId(id))
    }

    pub fn create_layered_texture(
        &mut self,
        kind: TextureKind,
        pixel_format: PixelFormat,
        width: u32,
        height: u32,
        layers: &[&[u8]],
    ) -> Result<TextureId, String> {
        if kind == TextureKind::Flat {
            let [layer] = layers else {
                return Err("a flat texture has exactly one layer".to_string());
            };
            return self.create_texture(pixel_format, width, height, layer, false);
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
            check_pixel_count(width, height, layer, sdl_pixel_format(pixel_format))?;
        }
        let format = sdl_pixel_format(pixel_format);
        // SAFETY: the device is alive and every layer was just checked to
        // cover the whole texture.
        let handle =
            unsafe { new_layered_texture(self.device, format, kind, width, height, layers)? };
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                format,
                width,
                height,
                is_target: false,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa: ptr::null_mut(),
                samples: 1,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: mipmap_bytes(format, width, height, 1) * layers.len() as u64,
                levels: 1,
                mipmaps: None,
                kind,
                layers: layers.len() as u32,
                compressed: false,
                auto_mipmaps: false,
                depth_compare: None,
                readable: true,
            },
        );
        Ok(TextureId(id))
    }

    /// Creates a render target as `desc` describes: with a full chain of
    /// mipmaps unless `desc.mipmaps` is `None` (`Some(true)` makes them
    /// again whenever the target has been drawn to), and multisampled
    /// with as many samples as the GPU allows up to `desc.samples`.
    pub fn create_render_target(&mut self, desc: &RenderTargetDesc) -> Result<TextureId, String> {
        let &RenderTargetDesc {
            width,
            height,
            format,
            mipmaps,
            samples,
            kind,
            layers,
            readable,
        } = desc;
        if width == 0 || height == 0 {
            return Err("a canvas must be at least 1x1 pixels".to_string());
        }
        if format.is_depth() {
            return self.create_depth_target(desc);
        }
        let layers = match kind {
            TextureKind::Flat => 1,
            TextureKind::Cube if width != height => {
                return Err("a cube canvas must be square".to_string());
            }
            TextureKind::Cube => 6,
            TextureKind::Array | TextureKind::Volume if layers == 0 => {
                return Err("a canvas must have at least one layer".to_string());
            }
            TextureKind::Array | TextureKind::Volume => layers,
        };
        if kind != TextureKind::Flat && samples > 1 {
            return Err("only a flat canvas can be multisampled".to_string());
        }
        if !readable {
            return Err("a color canvas is always readable".to_string());
        }
        if !self.supports_pixel_format(format, true) {
            return Err(format!("this GPU cannot draw to {format} canvases"));
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
        let format = sdl_pixel_format(format);
        // SAFETY: the device is alive, and the passes are begun, ended and
        // submitted in the order SDL requires.
        let handle = unsafe {
            let handle = SDL_CreateGPUTexture(
                self.device,
                &SDL_GPUTextureCreateInfo {
                    r#type: match kind {
                        TextureKind::Flat => SDL_GPU_TEXTURETYPE_2D,
                        TextureKind::Array => SDL_GPU_TEXTURETYPE_2D_ARRAY,
                        TextureKind::Cube => SDL_GPU_TEXTURETYPE_CUBE,
                        TextureKind::Volume => SDL_GPU_TEXTURETYPE_3D,
                    },
                    format,
                    usage: SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                    width,
                    height,
                    layer_count_or_depth: layers,
                    num_levels: levels,
                    sample_count: SDL_GPU_SAMPLECOUNT_1,
                    ..Default::default()
                },
            );
            if handle.is_null() {
                return Err(format!("could not create a canvas: {}", sdl_error()));
            }
            // New GPU memory holds garbage, so start every layer out
            // transparent.
            let commands = SDL_AcquireGPUCommandBuffer(self.device);
            if !commands.is_null() {
                for layer in 0..layers {
                    let target = SDL_GPUColorTargetInfo {
                        texture: handle,
                        layer_or_depth_plane: layer,
                        load_op: SDL_GPU_LOADOP_CLEAR,
                        store_op: SDL_GPU_STOREOP_STORE,
                        ..Default::default()
                    };
                    let pass = SDL_BeginGPURenderPass(commands, &target, 1, ptr::null());
                    if !pass.is_null() {
                        SDL_EndGPURenderPass(pass);
                    }
                }
                // The smaller levels start out transparent too.
                if levels > 1 {
                    SDL_GenerateMipmapsForGPUTexture(commands, handle);
                }
                SDL_SubmitGPUCommandBuffer(commands);
            }
            handle
        };
        let samples = self.supported_samples(format, samples);
        let msaa = if samples > 1 {
            // SAFETY: the device is alive.
            let msaa = unsafe { new_msaa_texture(self.device, format, width, height, samples) };
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
                format,
                width,
                height,
                is_target: true,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa,
                samples,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: mipmap_bytes(format, width, height, levels) * u64::from(layers)
                    + mipmap_bytes(format, width, height, 1) * u64::from(samples.max(1) - 1),
                levels,
                mipmaps: blend,
                kind,
                layers,
                compressed: false,
                auto_mipmaps: mipmaps == Some(true),
                depth_compare: None,
                readable: true,
            },
        );
        Ok(TextureId(id))
    }

    /// Creates a depth target as `desc`, which has a depth format,
    /// describes.
    fn create_depth_target(&mut self, desc: &RenderTargetDesc) -> Result<TextureId, String> {
        let &RenderTargetDesc {
            width,
            height,
            format,
            mipmaps,
            samples,
            kind,
            readable,
            ..
        } = desc;
        if kind != TextureKind::Flat {
            return Err(format!("a {format} canvas must be flat"));
        }
        if mipmaps.is_some() {
            return Err(format!("a {format} canvas cannot have mipmaps"));
        }
        if readable && samples > 1 {
            return Err(format!(
                "a multisampled {format} canvas cannot be readable; make it with readable off"
            ));
        }
        if !self.supports_pixel_format(format, !readable) {
            return Err(if readable {
                format!("this GPU cannot sample {format} canvases; make it with readable off")
            } else {
                format!("this GPU cannot draw to {format} canvases")
            });
        }
        let format = sdl_pixel_format(format);
        let samples = self.supported_samples(format, samples);
        // SAFETY: the device is alive.
        let handle =
            unsafe { new_depth_texture(self.device, format, width, height, samples, readable) };
        if handle.is_null() {
            return Err(format!("could not create a depth canvas: {}", sdl_error()));
        }
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            Texture {
                handle,
                format,
                width,
                height,
                is_target: true,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                msaa: ptr::null_mut(),
                samples,
                wrap: (Wrap::Clamp, Wrap::Clamp),
                bytes: mipmap_bytes(format, width, height, 1) * u64::from(samples.max(1)),
                levels: 1,
                mipmaps: None,
                kind,
                layers: 1,
                compressed: false,
                auto_mipmaps: false,
                depth_compare: None,
                readable,
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
        check_pixel_count(width, height, rgba, target.format)?;
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
        self.ensure_samplers_comparing(wrap, mipmaps, None)
    }

    /// As `ensure_samplers`, for samplers that compare with `compare`.
    fn ensure_samplers_comparing(
        &mut self,
        wrap: (Wrap, Wrap),
        mipmaps: Option<Mipmaps>,
        compare: Option<CompareMode>,
    ) -> Result<(), String> {
        for filter in [Filter::Linear, Filter::Nearest] {
            let key = SamplerKey {
                filter,
                wrap,
                mipmaps,
                compare,
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
        let (mipmaps, compare) = (entry.mipmaps, entry.depth_compare);
        let made = self
            .ensure_samplers(wrap, mipmaps)
            .and_then(|()| match compare {
                Some(_) => self.ensure_samplers_comparing(wrap, mipmaps, compare),
                None => Ok(()),
            });
        if let Err(message) = made {
            eprintln!("smitten: {message}");
            return;
        }
        if let Some(entry) = self.textures.get_mut(&texture.0) {
            entry.wrap = wrap;
        }
    }

    /// Sets how a readable depth target compares when a shader samples it
    /// as a depth texture.
    pub fn set_texture_depth_compare(
        &mut self,
        texture: TextureId,
        compare: Option<CompareMode>,
    ) -> Result<(), String> {
        let Some(&entry) = self.textures.get(&texture.0) else {
            return Err("the texture no longer exists".to_string());
        };
        if !is_depth_format(entry.format) {
            return Err("only a depth canvas has a depth sample mode".to_string());
        }
        if !entry.readable {
            return Err("only a readable depth canvas can be sampled".to_string());
        }
        if compare.is_some() {
            self.ensure_samplers_comparing(entry.wrap, entry.mipmaps, compare)?;
        }
        if let Some(entry) = self.textures.get_mut(&texture.0) {
            entry.depth_compare = compare;
        }
        Ok(())
    }

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

    /// The most samples a render target of the default format can have.
    pub fn max_msaa(&self) -> u32 {
        self.supported_samples(DEFAULT_TEXTURE_FORMAT, u32::MAX)
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

    pub fn create_buffer(&mut self, usage: BufferUsage, data: &[u8]) -> Result<BufferId, String> {
        let size = u32::try_from(data.len()).map_err(|_| "the buffer is too large".to_string())?;
        if size == 0 {
            return Err("a buffer cannot be empty".to_string());
        }
        // SAFETY: the device is alive, and the buffer is released if
        // filling it fails.
        let handle = unsafe {
            let handle = SDL_CreateGPUBuffer(
                self.device,
                &SDL_GPUBufferCreateInfo {
                    usage: match usage {
                        BufferUsage::Vertex => SDL_GPU_BUFFERUSAGE_VERTEX,
                        BufferUsage::Index => SDL_GPU_BUFFERUSAGE_INDEX,
                    },
                    size,
                    ..Default::default()
                },
            );
            if handle.is_null() {
                return Err(format!("could not create a buffer: {}", sdl_error()));
            }
            if let Err(message) = upload_buffer(self.device, handle, 0, data, false) {
                SDL_ReleaseGPUBuffer(self.device, handle);
                return Err(message);
            }
            handle
        };
        let id = self.next_buffer_id;
        self.next_buffer_id += 1;
        self.buffers.insert(
            id,
            GpuBuffer {
                handle,
                usage,
                size,
            },
        );
        Ok(BufferId(id))
    }

    pub fn update_buffer(
        &mut self,
        buffer: BufferId,
        offset: u32,
        data: &[u8],
    ) -> Result<(), String> {
        let Some(&entry) = self.buffers.get(&buffer.0) else {
            return Err("the buffer no longer exists".to_string());
        };
        let fits = u32::try_from(data.len())
            .ok()
            .and_then(|length| offset.checked_add(length))
            .is_some_and(|end| end <= entry.size);
        if !fits {
            return Err(format!(
                "{} bytes at {offset} do not fit in a {}-byte buffer",
                data.len(),
                entry.size
            ));
        }
        if data.is_empty() {
            return Ok(());
        }
        // Replacing all of it can cycle, which gives fresh memory rather
        // than waiting for frames still using the old. Part of it cannot,
        // since a cycled buffer loses the rest of its bytes.
        let whole = offset == 0 && data.len() == entry.size as usize;
        // SAFETY: the buffer is alive and the bytes were checked to fit.
        unsafe { upload_buffer(self.device, entry.handle, offset, data, whole) }
    }

    pub fn create_vertex_layout(
        &mut self,
        layout: &VertexLayout,
    ) -> Result<MeshVertexLayoutId, String> {
        if let Some(&id) = self.mesh_layout_ids.get(layout) {
            return Ok(MeshVertexLayoutId(id));
        }
        check_vertex_layout(layout)?;
        let compiled = shader::mesh_vertex(layout, self.gamma_correct)?;
        // SAFETY: the device is alive and the SPIR-V outlives the call.
        let vertex_shader = unsafe {
            create_shader(
                self.device,
                &compiled.spirv,
                shader::MESH_VERTEX_ENTRY_POINT,
                SDL_GPU_SHADERSTAGE_VERTEX,
                0,
                1,
            )?
        };
        let id = self.mesh_layouts.len() as u32 + 1;
        self.mesh_layouts.insert(
            id,
            MeshLayout {
                desc: layout.clone(),
                vertex_shader,
                locations: compiled.locations,
            },
        );
        self.mesh_layout_ids.insert(layout.clone(), id);
        Ok(MeshVertexLayoutId(id))
    }

    pub fn destroy_buffer(&mut self, buffer: BufferId) {
        if let Some(entry) = self.buffers.remove(&buffer.0) {
            // SAFETY: the buffer came from this device. SDL defers the
            // free until no in-flight frame uses it.
            unsafe { SDL_ReleaseGPUBuffer(self.device, entry.handle) };
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
        let mut custom = CustomShader {
            fragment,
            source: source.to_string(),
            uniform_size: compiled.uniform_size,
            textures: compiled
                .textures
                .iter()
                .map(|texture| (texture.kind, texture.comparison))
                .collect(),
            has_vertex: compiled.has_vertex,
            vertex_variants: HashMap::new(),
        };
        // The vertex shader for ordinary draws is made now, so a mistake
        // in it is reported here rather than drawing nothing.
        if custom.has_vertex {
            match self.compile_vertex_variant(&custom, VertexLayoutId::STANDARD) {
                Ok(variant) => {
                    custom
                        .vertex_variants
                        .insert(VertexLayoutId::STANDARD, variant);
                }
                Err(error) => {
                    // SAFETY: the fragment shader came from this device.
                    unsafe { custom.release(self.device) };
                    return Err(error);
                }
            }
        }
        let id = self.next_shader_id;
        self.next_shader_id += 1;
        self.shaders.insert(id, custom);
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
            unsafe { shader.release(device) };
        }
        let id = shader.0;
        self.fragment_variants
            .retain(|&(owner, _, _), &mut variant| {
                let owned = owner == Some(id);
                if owned {
                    // SAFETY: as above.
                    unsafe { SDL_ReleaseGPUShader(device, variant) };
                }
                !owned
            });
    }

    fn create_pipeline(
        &mut self,
        key: PipelineKey,
    ) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
        let fragment = match key.shader {
            _ if key.target.color_count > 1 => {
                self.fragment_variant(key.shader, key.layered, key.target.color_count)?
            }
            None if key.layered => self.array_fragment_shader,
            None => self.fragment_shader,
            Some(id) => {
                self.shaders
                    .get(&id)
                    .ok_or("the shader no longer exists")?
                    .fragment
            }
        };
        let custom_vertex = key
            .shader
            .and_then(|id| self.shaders.get(&id))
            .is_some_and(|shader| shader.has_vertex);
        let (vertex, input) = match key.shader {
            Some(id) if custom_vertex => self.custom_vertex_stage(id, key.layout)?,
            _ => self.vertex_stage(key.layout)?,
        };
        // SAFETY: the device and both shaders are alive.
        unsafe { create_pipeline(self.device, vertex, fragment, &input, key) }
    }

    /// The fragment shader of a custom shader, or of the built-in one for
    /// `None`, that draws to `targets` canvases at once.
    fn fragment_variant(
        &mut self,
        shader: Option<u32>,
        layered: bool,
        targets: u8,
    ) -> Result<*mut SDL_GPUShader, String> {
        let key = (shader, layered, targets);
        if let Some(&variant) = self.fragment_variants.get(&key) {
            return Ok(variant);
        }
        let source = match shader {
            Some(id) => {
                &self
                    .shaders
                    .get(&id)
                    .ok_or("the shader no longer exists")?
                    .source
            }
            None if layered => shader::BUILTIN_ARRAY,
            None => shader::BUILTIN,
        };
        // A custom shader declares its own textures, so it is never given
        // an array texture as `image`.
        let compiled =
            shader::compile_for(source, u32::from(targets), layered && shader.is_none())?;
        // SAFETY: the device is alive and the SPIR-V outlives the call.
        let variant = unsafe {
            create_shader(
                self.device,
                &compiled.spirv,
                shader::ENTRY_POINT,
                SDL_GPU_SHADERSTAGE_FRAGMENT,
                1 + compiled.textures.len() as u32,
                u32::from(compiled.uniform_size > 0),
            )?
        };
        self.fragment_variants.insert(key, variant);
        Ok(variant)
    }

    /// What a pass draws to: its first target as [`Renderer::target_desc`]
    /// says, and its extra targets' formats after it.
    fn pass_target_desc(&self, pass: &Pass) -> TargetDesc {
        let mut desc = self.target_desc(pass.target);
        if let Some(depth) = pass.depth_target.and_then(|id| self.textures.get(&id.0)) {
            desc.depth_stencil = depth.format;
        }
        for id in pass.extra_targets.iter().flatten() {
            let Some(texture) = self.textures.get(&id.0) else {
                break;
            };
            if usize::from(desc.color_count) == MAX_COLOR_TARGETS {
                break;
            }
            desc.colors[usize::from(desc.color_count)] = texture.format;
            desc.color_count += 1;
        }
        desc
    }

    /// The vertex shader and vertex input for a layout.
    fn vertex_stage(
        &self,
        layout: VertexLayoutId,
    ) -> Result<(*mut SDL_GPUShader, VertexInput), String> {
        match layout {
            VertexLayoutId::STANDARD => Ok((self.vertex_shader, standard_vertex_input())),
            VertexLayoutId(id) => {
                let layout = self
                    .mesh_layouts
                    .get(&id)
                    .ok_or_else(|| format!("there is no vertex layout {id}"))?;
                Ok((
                    layout.vertex_shader,
                    mesh_vertex_input(&layout.desc, &layout.locations),
                ))
            }
        }
    }

    /// The layout `layout` names: the one ordinary draws use, or a mesh's.
    fn layout_desc(&self, layout: VertexLayoutId) -> Result<VertexLayout, String> {
        match layout {
            VertexLayoutId::STANDARD => Ok(shader::standard_layout()),
            VertexLayoutId(id) => self
                .mesh_layouts
                .get(&id)
                .map(|layout| layout.desc.clone())
                .ok_or_else(|| format!("there is no vertex layout {id}")),
        }
    }

    /// Compiles the game's vertex shader of `shader` for `layout`.
    fn compile_vertex_variant(
        &self,
        shader: &CustomShader,
        layout: VertexLayoutId,
    ) -> Result<(*mut SDL_GPUShader, Vec<usize>), String> {
        let desc = self.layout_desc(layout)?;
        // Ordinary draws' colors were made linear on the CPU already.
        let linearize = self.gamma_correct && layout != VertexLayoutId::STANDARD;
        let compiled = shader::compile_vertex(&shader.source, &desc, linearize)?;
        // SAFETY: the device is alive and the SPIR-V outlives the call.
        let variant = unsafe {
            create_shader(
                self.device,
                &compiled.spirv,
                shader::MESH_VERTEX_ENTRY_POINT,
                SDL_GPU_SHADERSTAGE_VERTEX,
                1 + shader.textures.len() as u32,
                1 + u32::from(shader.uniform_size > 0),
            )?
        };
        Ok((variant, compiled.locations))
    }

    /// The game's vertex shader of custom shader `id` for `layout`, made
    /// the first time it is needed, and the vertex input to go with it.
    fn custom_vertex_stage(
        &mut self,
        id: u32,
        layout: VertexLayoutId,
    ) -> Result<(*mut SDL_GPUShader, VertexInput), String> {
        let shader = self.shaders.get(&id).ok_or("the shader no longer exists")?;
        let (variant, locations) = match shader.vertex_variants.get(&layout) {
            Some((variant, locations)) => (*variant, locations.clone()),
            None => {
                let (variant, locations) = self.compile_vertex_variant(shader, layout)?;
                if let Some(shader) = self.shaders.get_mut(&id) {
                    shader
                        .vertex_variants
                        .insert(layout, (variant, locations.clone()));
                }
                (variant, locations)
            }
        };
        let desc = self.layout_desc(layout)?;
        Ok((variant, mesh_vertex_input(&desc, &locations)))
    }

    /// What a pass to `target`, or to the screen for `None`, draws to.
    fn target_desc(&self, target: Option<TextureId>) -> TargetDesc {
        match target.and_then(|id| self.textures.get(&id.0)) {
            None if target.is_none() => TargetDesc::single(
                self.screen_format,
                self.screen_samples,
                self.screen_depth_format,
            ),
            None => TargetDesc::single(DEFAULT_TEXTURE_FORMAT, 1, self.stencil_format),
            Some(texture) => {
                TargetDesc::single(texture.format, texture.samples, self.stencil_format)
            }
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
                && !is_depth_format(target.format)
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
            let target = self.pass_target_desc(pass);
            for call in calls {
                let key = self.pipeline_key(call, target, frame.meshes);
                if !self.pipelines.contains_key(&key)
                    && let Ok(pipeline) = self.create_pipeline(key)
                {
                    self.pipelines.insert(key, pipeline);
                }
            }
        }
    }

    /// The pipeline `call` draws with.
    fn pipeline_key(
        &self,
        call: &DrawCall,
        target: TargetDesc,
        meshes: &[MeshDraw],
    ) -> PipelineKey {
        let layered = call.shader.is_none()
            && call
                .texture
                .and_then(|id| self.textures.get(&id.0))
                .is_some_and(|texture| texture.kind == TextureKind::Array);
        PipelineKey {
            shader: call.shader.map(|id| id.0),
            layered,
            target,
            layout: call
                .mesh
                .and_then(|index| meshes.get(index as usize))
                .map_or(VertexLayoutId::STANDARD, |draw| {
                    VertexLayoutId(draw.layout.0)
                }),
            blend: call.blend,
            blend_alpha: call.blend_alpha,
            stencil: call.stencil,
            depth: call.depth,
            color_mask: call.color_mask,
            wireframe: call.wireframe,
            cull: call.cull,
        }
    }

    /// The texture and sampler for each texture `shader` declares, from
    /// what the game sent, or white where it sent nothing usable.
    /// A depth texture declared for comparing gets a depth target with a
    /// compare mode, and the stand-in, at the far depth, otherwise.
    /// Nothing reads the depth target being drawn to, `depth_target`.
    fn shader_bindings(
        &self,
        shader: &CustomShader,
        sent: &[Option<BoundTexture>],
        depth_target: Option<TextureId>,
    ) -> Vec<SDL_GPUTextureSamplerBinding> {
        shader
            .textures
            .iter()
            .enumerate()
            .map(|(index, &(kind, comparison))| {
                let found = sent
                    .get(index)
                    .copied()
                    .flatten()
                    .filter(|bound| Some(bound.texture) != depth_target)
                    .and_then(|bound| Some((bound, self.textures.get(&bound.texture.0)?)))
                    .filter(|(_, texture)| {
                        texture.kind == kind
                            && texture.readable
                            && texture.depth_compare.is_some() == comparison
                    });
                if comparison {
                    return match found {
                        Some((bound, texture)) => SDL_GPUTextureSamplerBinding {
                            texture: texture.handle,
                            sampler: self.comparing_sampler(
                                bound.filter,
                                texture.wrap,
                                texture.mipmaps,
                                texture.depth_compare.unwrap_or(CompareMode::LessEqual),
                            ),
                        },
                        None => SDL_GPUTextureSamplerBinding {
                            texture: self.white_depth,
                            sampler: self.comparing,
                        },
                    };
                }
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
                            TextureKind::Volume => self.white_volume,
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
                compare: None,
            })
            .copied()
            .unwrap_or(clamped)
    }

    /// The comparing sampler for a depth texture with these settings,
    /// made beforehand by `set_texture_depth_compare`, or the stand-in's.
    fn comparing_sampler(
        &self,
        filter: Filter,
        wrap: (Wrap, Wrap),
        mipmaps: Option<Mipmaps>,
        compare: CompareMode,
    ) -> *mut SDL_GPUSampler {
        self.samplers
            .get(&SamplerKey {
                filter,
                wrap,
                mipmaps,
                compare: Some(compare),
            })
            .copied()
            .unwrap_or(self.comparing)
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
                self.screen_depth_format,
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
                    new_stencil_texture(self.device, self.screen_depth_format, width, height, 1);
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
            if self.gamma_correct {
                linearize_colors(vertices, mapped.cast::<Vertex>());
            } else {
                ptr::copy_nonoverlapping(
                    vertices.as_ptr(),
                    mapped.cast::<Vertex>(),
                    vertices.len(),
                );
            }
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
    pub fn read_texture(
        &mut self,
        texture: TextureId,
        slice: TargetSlice,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        let Some(&source) = self.textures.get(&texture.0) else {
            return Err("the texture no longer exists".to_string());
        };
        if source.compressed || is_depth_format(source.format) {
            return Err("only a color texture can be read back".to_string());
        }
        if slice.layer >= source.layers || slice.mipmap >= source.levels {
            return Err(format!(
                "layer {} at mipmap {} does not exist in a texture with {} layers and {} mipmaps",
                slice.layer, slice.mipmap, source.layers, source.levels
            ));
        }
        let (width, height) = level_size(source.width, source.height, slice.mipmap);
        // A volume's slices are depths, not layers.
        let (layer, z) = if source.kind == TextureKind::Volume {
            (0, slice.layer)
        } else {
            (slice.layer, 0)
        };
        let length = width as usize * height as usize * bytes_per_pixel(source.format) as usize;
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
                    mip_level: slice.mipmap,
                    layer,
                    z,
                    w: width,
                    h: height,
                    d: 1,
                    ..Default::default()
                },
                &SDL_GPUTextureTransferInfo {
                    transfer_buffer: transfer,
                    offset: 0,
                    pixels_per_row: width,
                    rows_per_layer: height,
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
            Ok((width, height, pixels))
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

            // Calls that draw from GPU buffers need none of these.
            let uploaded = !frame.calls.is_empty()
                && !frame.vertices.is_empty()
                && self.upload_vertices(commands, frame.vertices);
            let vertex_count = if uploaded {
                frame.vertices.len() as u32
            } else {
                0
            };

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
                        Some(target)
                            if target.is_target
                                && !is_depth_format(target.format)
                                && slice_fits(target, pass.slices[0]) =>
                        {
                            let (width, height) =
                                level_size(target.width, target.height, pass.slices[0].mipmap);
                            (
                                target.handle,
                                target.msaa,
                                target.stencil,
                                // A depth canvas leaves the target's own
                                // values for a later pass to clear.
                                pass.depth_target.is_none()
                                    && std::mem::take(&mut target.stencil_fresh),
                                width,
                                height,
                            )
                        }
                        _ => continue,
                    },
                };
                // A depth canvas takes the place of the target's own depth
                // and stencil values, and must match it.
                let (stencil, fresh) = match pass.depth_target {
                    None => (stencil, fresh),
                    Some(id) => match (pass.target, self.textures.get(&id.0)) {
                        (Some(target), Some(depth))
                            if is_depth_format(depth.format)
                                && (depth.width, depth.height) == (width, height)
                                && self
                                    .textures
                                    .get(&target.0)
                                    .is_some_and(|target| target.samples == depth.samples) =>
                        {
                            (depth.handle, false)
                        }
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

                let mut infos = [SDL_GPUColorTargetInfo::default(); MAX_COLOR_TARGETS];
                infos[0] = self.color_target_info(texture, msaa, pass.clear, pass.slices[0]);
                let mut info_count = 1;
                // The extra targets must match the first, or the pass is
                // skipped.
                let mut extras_ok = true;
                for ((id, clear), slice) in pass
                    .extra_targets
                    .iter()
                    .zip(pass.extra_clears)
                    .zip(&pass.slices[1..])
                {
                    let Some(id) = id else { break };
                    match self.textures.get(&id.0) {
                        Some(extra)
                            if extra.is_target
                                && slice_fits(extra, *slice)
                                && level_size(extra.width, extra.height, slice.mipmap)
                                    == (width, height)
                                && extra.msaa.is_null() == msaa.is_null() =>
                        {
                            infos[info_count] =
                                self.color_target_info(extra.handle, extra.msaa, clear, *slice);
                            info_count += 1;
                        }
                        _ => extras_ok = false,
                    }
                }
                if !extras_ok {
                    continue;
                }
                let stencil_info = SDL_GPUDepthStencilTargetInfo {
                    texture: stencil,
                    load_op: if pass.clear_depth.is_some() || fresh {
                        SDL_GPU_LOADOP_CLEAR
                    } else {
                        SDL_GPU_LOADOP_LOAD
                    },
                    store_op: SDL_GPU_STOREOP_STORE,
                    clear_depth: pass.clear_depth.unwrap_or(1.0),
                    stencil_load_op: if pass.clear_stencil.is_some() || fresh {
                        SDL_GPU_LOADOP_CLEAR
                    } else {
                        SDL_GPU_LOADOP_LOAD
                    },
                    stencil_store_op: SDL_GPU_STOREOP_STORE,
                    clear_stencil: pass.clear_stencil.unwrap_or(0),
                    ..Default::default()
                };
                let render_pass = SDL_BeginGPURenderPass(
                    commands,
                    infos.as_ptr(),
                    info_count as u32,
                    &stencil_info,
                );
                if render_pass.is_null() {
                    continue;
                }
                let calls = (pass.first_call as usize)
                    .checked_add(pass.call_count as usize)
                    .and_then(|end| frame.calls.get(pass.first_call as usize..end));
                if let Some(calls) = calls {
                    self.draw_calls(
                        commands,
                        render_pass,
                        calls,
                        frame,
                        pass.target,
                        pass.depth_target,
                        self.pass_target_desc(pass),
                        vertex_count,
                        width,
                        height,
                    );
                }
                SDL_EndGPURenderPass(render_pass);
                // A target that keeps its mipmaps up to date makes them
                // once the passes drawing to it in a row are done.
                let next = frame.passes.get(index + 1);
                let same_next = next.is_some_and(|next| {
                    next.target == pass.target && next.extra_targets == pass.extra_targets
                });
                if !same_next {
                    for id in pass
                        .target
                        .iter()
                        .chain(pass.extra_targets.iter().flatten())
                    {
                        if let Some(target) = self.textures.get(&id.0)
                            && target.auto_mipmaps
                        {
                            SDL_GenerateMipmapsForGPUTexture(commands, target.handle);
                        }
                    }
                }
            }

            SDL_SubmitGPUCommandBuffer(commands);
        }
    }

    /// How a pass draws to `texture`, which is resolved from `msaa` unless
    /// that is null, filled with `clear` first if given.
    fn color_target_info(
        &self,
        texture: *mut SDL_GPUTexture,
        msaa: *mut SDL_GPUTexture,
        clear: Option<smitten_core::Color>,
        slice: TargetSlice,
    ) -> SDL_GPUColorTargetInfo {
        let mut info = if msaa.is_null() {
            SDL_GPUColorTargetInfo {
                texture,
                // A layer of an array, a face of a cube, or a depth of a
                // volume.
                layer_or_depth_plane: slice.layer,
                mip_level: slice.mipmap,
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
        if let Some(color) = clear {
            info.load_op = SDL_GPU_LOADOP_CLEAR;
            let [r, g, b, a] = if self.gamma_correct {
                linear_color([color.r, color.g, color.b, color.a])
            } else {
                [color.r, color.g, color.b, color.a]
            };
            info.clear_color = SDL_FColor { r, g, b, a };
        }
        info
    }

    /// # Safety
    /// `pass` must be an open render pass on `commands` whose target is
    /// `width` by `height` pixels, and the vertex
    /// buffer must already hold `vertex_count` of the frame's vertices.
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_calls(
        &self,
        commands: *mut SDL_GPUCommandBuffer,
        pass: *mut SDL_GPURenderPass,
        calls: &[DrawCall],
        frame: &Frame,
        target: Option<TextureId>,
        depth_target: Option<TextureId>,
        target_desc: TargetDesc,
        vertex_count: u32,
        width: u32,
        height: u32,
    ) {
        let uniforms = frame.uniforms;
        let sent_textures = frame.textures;
        // Padded to 16 bytes, the size of the shader's uniform block.
        let pixel_to_clip = [2.0 / width as f32, 2.0 / height as f32, 0.0, 0.0];
        let projection = pixel_projection(width, height);
        let mut bound_pipeline = ptr::null_mut();
        let mut bound_reference = None;
        let mut bound_scissor = None;
        // What the vertex buffer slot and the vertex uniforms hold: the
        // frame's vertices in pixels, or a mesh's own.
        let mut bound_mesh: Option<MeshDraw> = None;
        let mut screen_bound = false;
        let mut bound_uniforms = VertexUniforms::Unknown;
        let standard_uniforms = MeshUniforms {
            transform_projection: projection,
            tint: [1.0; 4],
        };

        // SAFETY: every handle bound here is alive, and each draw is checked
        // to stay inside the uploaded vertices or its buffers.
        unsafe {
            for call in calls {
                // Each draw finds what it reads from first, and draws
                // nothing if that is missing or too short.
                let mesh = match call.mesh {
                    None => {
                        let in_range = call
                            .first_vertex
                            .checked_add(call.vertex_count)
                            .is_some_and(|end| end <= vertex_count);
                        if call.vertex_count == 0 || !in_range {
                            continue;
                        }
                        None
                    }
                    Some(index) => {
                        let Some(&draw) = frame.meshes.get(index as usize) else {
                            continue;
                        };
                        let Some((vertices, indices)) = self.mesh_buffers(&draw) else {
                            continue;
                        };
                        Some((draw, vertices, indices))
                    }
                };
                // A target cannot be sampled while it is being drawn to.
                if call.texture.is_some()
                    && (call.texture == target || call.texture == depth_target)
                {
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
                        Some(texture) if texture.readable => {
                            (texture.handle, texture.wrap, texture.mipmaps, texture.kind)
                        }
                        _ => continue,
                    },
                };
                // A cube cannot be drawn flat, and a custom shader's
                // `image` is always flat.
                let drawable = match kind {
                    TextureKind::Flat => true,
                    TextureKind::Array => call.shader.is_none(),
                    TextureKind::Cube | TextureKind::Volume => false,
                };
                if !drawable {
                    continue;
                }
                let Some(scissor) = clip_scissor(call.scissor, width, height) else {
                    continue;
                };
                let key = self.pipeline_key(call, target_desc, frame.meshes);
                // A pipeline that could not be built draws nothing.
                let Some(&pipeline) = self.pipelines.get(&key) else {
                    continue;
                };
                let mut extra_bindings = Vec::new();
                let mut custom_vertex = false;
                if let Some(id) = call.shader {
                    // A shader that was freed while still queued, or whose
                    // data is missing, draws nothing.
                    let Some(shader) = self.shaders.get(&id.0) else {
                        continue;
                    };
                    custom_vertex = shader.has_vertex;
                    if !shader.textures.is_empty() {
                        let start = call.texture_offset as usize;
                        let sent = start
                            .checked_add(shader.textures.len())
                            .and_then(|end| sent_textures.get(start..end))
                            .unwrap_or(&[]);
                        extra_bindings = self.shader_bindings(shader, sent, depth_target);
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
                        // The game's vertex shader reads the same
                        // `params`, after the framework's own uniforms.
                        if custom_vertex {
                            SDL_PushGPUVertexUniformData(
                                commands,
                                1,
                                data.as_ptr().cast::<c_void>(),
                                shader.uniform_size,
                            );
                        }
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
                if custom_vertex {
                    SDL_BindGPUVertexSamplers(pass, 0, bindings.as_ptr(), bindings.len() as u32);
                }
                match mesh {
                    None => {
                        if !screen_bound {
                            SDL_BindGPUVertexBuffers(
                                pass,
                                0,
                                &SDL_GPUBufferBinding {
                                    buffer: self.vertex_buffer,
                                    offset: 0,
                                },
                                1,
                            );
                            screen_bound = true;
                            bound_mesh = None;
                        }
                        // The built-in vertex shader takes pixels to clip
                        // space itself; a game's is given the matrix.
                        if custom_vertex && bound_uniforms != VertexUniforms::Standard {
                            SDL_PushGPUVertexUniformData(
                                commands,
                                0,
                                ptr::from_ref(&standard_uniforms).cast::<c_void>(),
                                size_of::<MeshUniforms>() as u32,
                            );
                            bound_uniforms = VertexUniforms::Standard;
                        } else if !custom_vertex && bound_uniforms != VertexUniforms::PixelToClip {
                            SDL_PushGPUVertexUniformData(
                                commands,
                                0,
                                pixel_to_clip.as_ptr().cast::<c_void>(),
                                size_of_val(&pixel_to_clip) as u32,
                            );
                            bound_uniforms = VertexUniforms::PixelToClip;
                        }
                        SDL_DrawGPUPrimitives(pass, call.vertex_count, 1, call.first_vertex, 0);
                    }
                    Some((draw, vertices, indices)) => {
                        if bound_mesh != Some(draw) {
                            let bindings: Vec<SDL_GPUBufferBinding> = vertices
                                .iter()
                                .map(|&buffer| SDL_GPUBufferBinding { buffer, offset: 0 })
                                .collect();
                            SDL_BindGPUVertexBuffers(
                                pass,
                                0,
                                bindings.as_ptr(),
                                bindings.len() as u32,
                            );
                            SDL_BindGPUIndexBuffer(
                                pass,
                                &SDL_GPUBufferBinding {
                                    buffer: indices,
                                    offset: 0,
                                },
                                match draw.index_type {
                                    IndexDataType::U16 => SDL_GPU_INDEXELEMENTSIZE_16BIT,
                                    IndexDataType::U32 => SDL_GPU_INDEXELEMENTSIZE_32BIT,
                                },
                            );
                            bound_mesh = Some(draw);
                            screen_bound = false;
                        }
                        if bound_uniforms != VertexUniforms::Mesh(draw) {
                            let uniforms = MeshUniforms {
                                transform_projection: multiply(&projection, &draw.transform),
                                tint: if self.gamma_correct {
                                    linear_color(draw.tint)
                                } else {
                                    draw.tint
                                },
                            };
                            SDL_PushGPUVertexUniformData(
                                commands,
                                0,
                                ptr::from_ref(&uniforms).cast::<c_void>(),
                                size_of::<MeshUniforms>() as u32,
                            );
                            bound_uniforms = VertexUniforms::Mesh(draw);
                        }
                        SDL_DrawGPUIndexedPrimitives(
                            pass,
                            draw.index_count,
                            draw.instance_count,
                            draw.first_index,
                            0,
                            0,
                        );
                    }
                }
            }
        }
    }
}

impl Renderer {
    /// The vertex buffers, one for each of the layout's slots, and the
    /// index buffer a mesh draw reads, if they all exist and hold enough
    /// for the draw.
    fn mesh_buffers(
        &self,
        draw: &MeshDraw,
    ) -> Option<(Vec<*mut SDL_GPUBuffer>, *mut SDL_GPUBuffer)> {
        let layout = self.mesh_layouts.get(&draw.layout.0)?;
        let indices = self
            .buffers
            .get(&draw.indices.0)
            .filter(|indices| indices.usage == BufferUsage::Index)?;
        let index_end = draw
            .first_index
            .checked_add(draw.index_count)?
            .checked_mul(draw.index_type.size())?;
        if draw.index_count == 0 || draw.instance_count == 0 || index_end > indices.size {
            return None;
        }
        let mut vertices = Vec::with_capacity(layout.desc.buffers.len());
        for (slot, buffer_layout) in layout.desc.buffers.iter().enumerate() {
            let buffer = self
                .buffers
                .get(&draw.buffers.get(slot).copied().flatten()?.0)
                .filter(|buffer| buffer.usage == BufferUsage::Vertex)?;
            // Every vertex the indices may reach, and every instance
            // drawn, must be there.
            let needed = match buffer_layout.step {
                VertexStep::PerVertex => draw.vertex_count,
                VertexStep::PerInstance => draw.instance_count,
            };
            if u64::from(needed) * u64::from(buffer_layout.stride) > u64::from(buffer.size) {
                return None;
            }
            vertices.push(buffer.handle);
        }
        Some((vertices, indices.handle))
    }
}

/// Checks that a mesh layout is one the GPU can read.
fn check_vertex_layout(layout: &VertexLayout) -> Result<(), String> {
    if layout.buffers.is_empty() || layout.buffers.len() > MAX_VERTEX_BUFFERS {
        return Err(format!(
            "a vertex layout needs 1 to {MAX_VERTEX_BUFFERS} buffers, not {}",
            layout.buffers.len()
        ));
    }
    if layout.attributes.len() > MAX_VERTEX_ATTRIBUTES {
        return Err(format!(
            "a vertex layout can have at most {MAX_VERTEX_ATTRIBUTES} attributes"
        ));
    }
    for buffer in &layout.buffers {
        if buffer.stride == 0 || !buffer.stride.is_multiple_of(4) {
            return Err(format!(
                "a vertex buffer's stride must be a positive multiple of 4, not {}",
                buffer.stride
            ));
        }
    }
    for (index, attribute) in layout.attributes.iter().enumerate() {
        let name = &attribute.name;
        if layout.attributes[..index]
            .iter()
            .any(|other| other.name == *name)
        {
            return Err(format!("vertex attribute `{name}` is named twice"));
        }
        if !(1..=4).contains(&attribute.components) {
            return Err(format!(
                "vertex attribute `{name}` must have 1 to 4 components"
            ));
        }
        let Some(buffer) = layout.buffers.get(attribute.buffer as usize) else {
            return Err(format!(
                "vertex attribute `{name}` is in a buffer that is not there"
            ));
        };
        let end = attribute.offset + attribute.data_type.size(attribute.components);
        if !attribute.offset.is_multiple_of(4) || end > buffer.stride {
            return Err(format!(
                "vertex attribute `{name}` does not fit in its vertex"
            ));
        }
    }
    Ok(())
}

/// The vertex input for a mesh layout, with the attributes its shader
/// reads at their locations.
fn mesh_vertex_input(layout: &VertexLayout, locations: &[usize]) -> VertexInput {
    VertexInput {
        buffers: layout
            .buffers
            .iter()
            .enumerate()
            .map(|(slot, buffer)| SDL_GPUVertexBufferDescription {
                slot: slot as u32,
                pitch: buffer.stride,
                input_rate: match buffer.step {
                    VertexStep::PerVertex => SDL_GPU_VERTEXINPUTRATE_VERTEX,
                    VertexStep::PerInstance => SDL_GPU_VERTEXINPUTRATE_INSTANCE,
                },
                instance_step_rate: 0,
            })
            .collect(),
        attributes: locations
            .iter()
            .enumerate()
            .map(|(location, &index)| {
                let attribute = &layout.attributes[index];
                SDL_GPUVertexAttribute {
                    location: location as u32,
                    buffer_slot: attribute.buffer,
                    format: vertex_element_format(attribute.data_type, attribute.components),
                    offset: attribute.offset,
                }
            })
            .collect(),
    }
}

/// The SDL format a vertex attribute is read as, matching
/// [`shader::attribute_shader_type`].
fn vertex_element_format(data_type: VertexDataType, components: u8) -> SDL_GPUVertexElementFormat {
    match (data_type, components) {
        (VertexDataType::Float, 1) => SDL_GPU_VERTEXELEMENTFORMAT_FLOAT,
        (VertexDataType::Float, 2) => SDL_GPU_VERTEXELEMENTFORMAT_FLOAT2,
        (VertexDataType::Float, 3) => SDL_GPU_VERTEXELEMENTFORMAT_FLOAT3,
        (VertexDataType::Float, _) => SDL_GPU_VERTEXELEMENTFORMAT_FLOAT4,
        (VertexDataType::UNorm8, _) => SDL_GPU_VERTEXELEMENTFORMAT_UBYTE4_NORM,
        (VertexDataType::UNorm16, 1 | 2) => SDL_GPU_VERTEXELEMENTFORMAT_USHORT2_NORM,
        (VertexDataType::UNorm16, _) => SDL_GPU_VERTEXELEMENTFORMAT_USHORT4_NORM,
    }
}

/// What a pass's vertex uniforms in slot 0 hold, so they are pushed again
/// only when a draw needs something else.
#[derive(Clone, Copy, PartialEq)]
enum VertexUniforms {
    Unknown,
    /// The scale from pixels to clip space, for the built-in vertex shader
    /// of ordinary draws.
    PixelToClip,
    /// [`MeshUniforms`] for ordinary draws with a game's vertex shader:
    /// the pixel projection and no tint.
    Standard,
    /// [`MeshUniforms`] for a mesh draw.
    Mesh(MeshDraw),
}

/// What every mesh vertex shader, and a game's, reads for each draw, laid
/// out as `SmittenDraw`.
#[repr(C)]
struct MeshUniforms {
    transform_projection: [f32; 16],
    tint: [f32; 4],
}

/// The column-major matrix that takes pixels of a `width` by `height`
/// target, from its top-left with y down, to SDL's clip space, with y up.
/// z from -10 to 10 becomes depth 1 to 0, as LÖVE's projection makes it,
/// so z 0 is depth 0.5 like everything `sprite.vert` draws.
fn pixel_projection(width: u32, height: u32) -> [f32; 16] {
    let (width, height) = (width as f32, height as f32);
    [
        2.0 / width,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / height,
        0.0,
        0.0,
        0.0,
        0.0,
        -0.05,
        0.0,
        -1.0,
        1.0,
        0.5,
        1.0,
    ]
}

/// `a * b` for column-major 4 by 4 matrices, so `b` applies first.
fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            out[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
        }
    }
    out
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
            SDL_ReleaseGPUTexture(self.device, self.white_volume);
            SDL_ReleaseGPUTexture(self.device, self.white_depth);
            SDL_ReleaseGPUBuffer(self.device, self.vertex_buffer);
            SDL_ReleaseGPUTransferBuffer(self.device, self.transfer_buffer);
            for (_, buffer) in self.buffers.drain() {
                SDL_ReleaseGPUBuffer(self.device, buffer.handle);
            }
            SDL_ReleaseGPUSampler(self.device, self.linear);
            SDL_ReleaseGPUSampler(self.device, self.nearest);
            if !self.comparing.is_null() {
                SDL_ReleaseGPUSampler(self.device, self.comparing);
            }
            for &sampler in self.samplers.values() {
                SDL_ReleaseGPUSampler(self.device, sampler);
            }
            for &pipeline in self.pipelines.values() {
                SDL_ReleaseGPUGraphicsPipeline(self.device, pipeline);
            }
            for shader in self.shaders.values() {
                shader.release(self.device);
            }
            SDL_ReleaseGPUShader(self.device, self.vertex_shader);
            for layout in self.mesh_layouts.values() {
                SDL_ReleaseGPUShader(self.device, layout.vertex_shader);
            }
            SDL_ReleaseGPUShader(self.device, self.fragment_shader);
            SDL_ReleaseGPUShader(self.device, self.array_fragment_shader);
            for &variant in self.fragment_variants.values() {
                SDL_ReleaseGPUShader(self.device, variant);
            }
        }
    }
}

/// # Safety
/// `device` and both shaders must be alive.
unsafe fn create_pipeline(
    device: *mut SDL_GPUDevice,
    vertex_shader: *mut SDL_GPUShader,
    fragment_shader: *mut SDL_GPUShader,
    input: &VertexInput,
    key: PipelineKey,
) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
    let PipelineKey {
        target,
        blend,
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
        let targets: Vec<_> = target
            .colors()
            .iter()
            .map(|&format| SDL_GPUColorTargetDescription {
                format,
                blend_state,
            })
            .collect();

        let info = SDL_GPUGraphicsPipelineCreateInfo {
            vertex_shader,
            fragment_shader,
            vertex_input_state: SDL_GPUVertexInputState {
                vertex_buffer_descriptions: input.buffers.as_ptr(),
                num_vertex_buffers: input.buffers.len() as u32,
                vertex_attributes: input.attributes.as_ptr(),
                num_vertex_attributes: input.attributes.len() as u32,
            },
            primitive_type: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
            multisample_state: SDL_GPUMultisampleState {
                sample_count: sample_count(target.samples),
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
                depth_stencil_format: target.depth_stencil,
                has_depth_stencil_target: target.depth_stencil != SDL_GPU_TEXTUREFORMAT_INVALID,
                ..Default::default()
            },
            // A depth canvas without a stencil part ignores stencil modes.
            depth_stencil_state: depth_stencil_state(
                if has_stencil(target.depth_stencil) {
                    stencil
                } else {
                    StencilMode::Off
                },
                key.depth,
            ),
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

/// The single buffer of [`Vertex`] that ordinary draws use.
fn standard_vertex_input() -> VertexInput {
    let attribute = |location, format, offset: usize| SDL_GPUVertexAttribute {
        location,
        buffer_slot: 0,
        format,
        offset: offset as u32,
    };
    VertexInput {
        buffers: vec![SDL_GPUVertexBufferDescription {
            slot: 0,
            pitch: VERTEX_SIZE,
            input_rate: SDL_GPU_VERTEXINPUTRATE_VERTEX,
            instance_step_rate: 0,
        }],
        attributes: vec![
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
        ],
    }
}

/// The GPU's name for a comparison of a new value with the one there.
fn compare_op(compare: CompareMode) -> SDL_GPUCompareOp {
    match compare {
        CompareMode::Equal => SDL_GPU_COMPAREOP_EQUAL,
        CompareMode::NotEqual => SDL_GPU_COMPAREOP_NOT_EQUAL,
        CompareMode::Less => SDL_GPU_COMPAREOP_LESS,
        CompareMode::LessEqual => SDL_GPU_COMPAREOP_LESS_OR_EQUAL,
        CompareMode::Greater => SDL_GPU_COMPAREOP_GREATER,
        CompareMode::GreaterEqual => SDL_GPU_COMPAREOP_GREATER_OR_EQUAL,
        CompareMode::Never => SDL_GPU_COMPAREOP_NEVER,
        CompareMode::Always => SDL_GPU_COMPAREOP_ALWAYS,
    }
}

/// The GPU depth and stencil settings for a depth and a stencil mode.
fn depth_stencil_state(stencil: StencilMode, depth: DepthMode) -> SDL_GPUDepthStencilState {
    SDL_GPUDepthStencilState {
        // The GPU compares the new depth with the one there, as
        // `DepthMode` reads.
        compare_op: compare_op(depth.compare),
        // Writing needs the test on, even one that always passes.
        enable_depth_test: depth.is_used(),
        enable_depth_write: depth.write,
        ..stencil_state(stencil)
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

/// Creates a depth target, which `readable` lets shaders sample, with
/// every depth value at 1, the farthest, and every stencil value at 0.
/// Null if the GPU refuses.
///
/// # Safety
/// `device` must be alive.
unsafe fn new_depth_texture(
    device: *mut SDL_GPUDevice,
    format: SDL_GPUTextureFormat,
    width: u32,
    height: u32,
    samples: u32,
    readable: bool,
) -> *mut SDL_GPUTexture {
    let mut usage = SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET;
    if readable {
        usage |= SDL_GPU_TEXTUREUSAGE_SAMPLER;
    }
    // SAFETY: the create-info is valid for the call, and the pass is
    // begun, ended and submitted in order.
    unsafe {
        let texture = SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type: SDL_GPU_TEXTURETYPE_2D,
                format,
                usage,
                width,
                height,
                layer_count_or_depth: 1,
                num_levels: 1,
                sample_count: sample_count(samples),
                ..Default::default()
            },
        );
        if texture.is_null() {
            return texture;
        }
        let commands = SDL_AcquireGPUCommandBuffer(device);
        if !commands.is_null() {
            let target = SDL_GPUDepthStencilTargetInfo {
                texture,
                clear_depth: 1.0,
                load_op: SDL_GPU_LOADOP_CLEAR,
                store_op: SDL_GPU_STOREOP_STORE,
                stencil_load_op: SDL_GPU_LOADOP_CLEAR,
                stencil_store_op: SDL_GPU_STOREOP_STORE,
                ..Default::default()
            };
            let pass = SDL_BeginGPURenderPass(commands, ptr::null(), 0, &target);
            if !pass.is_null() {
                SDL_EndGPURenderPass(pass);
            }
            SDL_SubmitGPUCommandBuffer(commands);
        }
        texture
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
        // SDL has no border colors, so clampzero falls back to clamp.
        Wrap::Clamp | Wrap::ClampZero => SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
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
    if let Some(compare) = key.compare {
        info.enable_compare = true;
        info.compare_op = compare_op(compare);
    }
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
            compare: None,
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
        CompressedFormat::Bc6h => SDL_GPU_TEXTUREFORMAT_BC6H_RGB_UFLOAT,
        CompressedFormat::Bc6hSigned => SDL_GPU_TEXTUREFORMAT_BC6H_RGB_FLOAT,
        CompressedFormat::Bc7 => SDL_GPU_TEXTUREFORMAT_BC7_RGBA_UNORM,
        CompressedFormat::Astc4x4 => SDL_GPU_TEXTUREFORMAT_ASTC_4x4_UNORM,
        CompressedFormat::Astc5x4 => SDL_GPU_TEXTUREFORMAT_ASTC_5x4_UNORM,
        CompressedFormat::Astc5x5 => SDL_GPU_TEXTUREFORMAT_ASTC_5x5_UNORM,
        CompressedFormat::Astc6x5 => SDL_GPU_TEXTUREFORMAT_ASTC_6x5_UNORM,
        CompressedFormat::Astc6x6 => SDL_GPU_TEXTUREFORMAT_ASTC_6x6_UNORM,
        CompressedFormat::Astc8x5 => SDL_GPU_TEXTUREFORMAT_ASTC_8x5_UNORM,
        CompressedFormat::Astc8x6 => SDL_GPU_TEXTUREFORMAT_ASTC_8x6_UNORM,
        CompressedFormat::Astc8x8 => SDL_GPU_TEXTUREFORMAT_ASTC_8x8_UNORM,
        CompressedFormat::Astc10x5 => SDL_GPU_TEXTUREFORMAT_ASTC_10x5_UNORM,
        CompressedFormat::Astc10x6 => SDL_GPU_TEXTUREFORMAT_ASTC_10x6_UNORM,
        CompressedFormat::Astc10x8 => SDL_GPU_TEXTUREFORMAT_ASTC_10x8_UNORM,
        CompressedFormat::Astc10x10 => SDL_GPU_TEXTUREFORMAT_ASTC_10x10_UNORM,
        CompressedFormat::Astc12x10 => SDL_GPU_TEXTUREFORMAT_ASTC_12x10_UNORM,
        CompressedFormat::Astc12x12 => SDL_GPU_TEXTUREFORMAT_ASTC_12x12_UNORM,
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

/// The bytes of `levels` mipmap levels of a `width` by `height` texture in
/// an uncompressed `format`.
fn mipmap_bytes(format: SDL_GPUTextureFormat, width: u32, height: u32, levels: u32) -> u64 {
    let pixel = u64::from(bytes_per_pixel(format));
    (0..levels)
        .map(|level| {
            u64::from((width >> level).max(1)) * u64::from((height >> level).max(1)) * pixel
        })
        .sum()
}

/// How many bytes one pixel of an uncompressed `format` takes.
fn bytes_per_pixel(format: SDL_GPUTextureFormat) -> u32 {
    // SAFETY: SDL only reads the value.
    unsafe { SDL_GPUTextureFormatTexelBlockSize(format) }
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

fn check_pixel_count(
    width: u32,
    height: u32,
    pixels: &[u8],
    format: SDL_GPUTextureFormat,
) -> Result<(), String> {
    let expected = width as usize * height as usize * bytes_per_pixel(format) as usize;
    if pixels.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "{width}x{height} pixels need {expected} bytes, got {}",
            pixels.len()
        ))
    }
}

/// `color`, given sRGB-encoded as games give colors, in linear light.
/// Alpha is left alone.
fn linear_color([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    let linear = |component: f32| {
        if component <= 0.04045 {
            component / 12.92
        } else {
            ((component + 0.055) / 1.055).powf(2.4)
        }
    };
    [linear(r), linear(g), linear(b), a]
}

/// Copies `vertices` to `out` with their colors in linear light.
/// Neighboring vertices usually share a color, so the last conversion is
/// reused.
///
/// # Safety
/// `out` must have room for `vertices.len()` vertices.
unsafe fn linearize_colors(vertices: &[Vertex], out: *mut Vertex) {
    let mut last: Option<([f32; 4], [f32; 4])> = None;
    for (index, vertex) in vertices.iter().enumerate() {
        let color = match last {
            Some((from, to)) if from == vertex.color => to,
            _ => {
                let to = linear_color(vertex.color);
                last = Some((vertex.color, to));
                to
            }
        };
        // SAFETY: as the caller promised.
        unsafe { out.add(index).write(Vertex { color, ..*vertex }) };
    }
}

/// The sRGB version of a compressed format, if it has one.
fn compressed_srgb_format(format: CompressedFormat) -> Option<SDL_GPUTextureFormat> {
    match format {
        CompressedFormat::Bc1 => Some(SDL_GPU_TEXTUREFORMAT_BC1_RGBA_UNORM_SRGB),
        CompressedFormat::Bc2 => Some(SDL_GPU_TEXTUREFORMAT_BC2_RGBA_UNORM_SRGB),
        CompressedFormat::Bc3 => Some(SDL_GPU_TEXTUREFORMAT_BC3_RGBA_UNORM_SRGB),
        CompressedFormat::Bc7 => Some(SDL_GPU_TEXTUREFORMAT_BC7_RGBA_UNORM_SRGB),
        CompressedFormat::Astc4x4 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_4x4_UNORM_SRGB),
        CompressedFormat::Astc5x4 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_5x4_UNORM_SRGB),
        CompressedFormat::Astc5x5 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_5x5_UNORM_SRGB),
        CompressedFormat::Astc6x5 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_6x5_UNORM_SRGB),
        CompressedFormat::Astc6x6 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_6x6_UNORM_SRGB),
        CompressedFormat::Astc8x5 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_8x5_UNORM_SRGB),
        CompressedFormat::Astc8x6 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_8x6_UNORM_SRGB),
        CompressedFormat::Astc8x8 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_8x8_UNORM_SRGB),
        CompressedFormat::Astc10x5 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_10x5_UNORM_SRGB),
        CompressedFormat::Astc10x6 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_10x6_UNORM_SRGB),
        CompressedFormat::Astc10x8 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_10x8_UNORM_SRGB),
        CompressedFormat::Astc10x10 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_10x10_UNORM_SRGB),
        CompressedFormat::Astc12x10 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_12x10_UNORM_SRGB),
        CompressedFormat::Astc12x12 => Some(SDL_GPU_TEXTUREFORMAT_ASTC_12x12_UNORM_SRGB),
        // Floating-point and one- or two-channel formats hold no colors
        // to encode.
        CompressedFormat::Bc4
        | CompressedFormat::Bc5
        | CompressedFormat::Bc6h
        | CompressedFormat::Bc6hSigned => None,
    }
}

/// Whether `slice` names a layer and mipmap level that `texture` has.
fn slice_fits(texture: &Texture, slice: TargetSlice) -> bool {
    slice.layer < texture.layers && slice.mipmap < texture.levels
}

/// The SDL format for a pixel format.
fn sdl_pixel_format(format: PixelFormat) -> SDL_GPUTextureFormat {
    use PixelFormat::*;
    match format {
        R8 => SDL_GPU_TEXTUREFORMAT_R8_UNORM,
        Rg8 => SDL_GPU_TEXTUREFORMAT_R8G8_UNORM,
        Rgba8 => SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        Srgba8 => SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
        R16 => SDL_GPU_TEXTUREFORMAT_R16_UNORM,
        Rg16 => SDL_GPU_TEXTUREFORMAT_R16G16_UNORM,
        Rgba16 => SDL_GPU_TEXTUREFORMAT_R16G16B16A16_UNORM,
        R16f => SDL_GPU_TEXTUREFORMAT_R16_FLOAT,
        Rg16f => SDL_GPU_TEXTUREFORMAT_R16G16_FLOAT,
        Rgba16f => SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
        R32f => SDL_GPU_TEXTUREFORMAT_R32_FLOAT,
        Rg32f => SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT,
        Rgba32f => SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT,
        Rgb10a2 => SDL_GPU_TEXTUREFORMAT_R10G10B10A2_UNORM,
        Rg11b10f => SDL_GPU_TEXTUREFORMAT_R11G11B10_UFLOAT,
        Depth16 => SDL_GPU_TEXTUREFORMAT_D16_UNORM,
        Depth24 => SDL_GPU_TEXTUREFORMAT_D24_UNORM,
        Depth32f => SDL_GPU_TEXTUREFORMAT_D32_FLOAT,
        Depth24Stencil8 => SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT,
        Depth32fStencil8 => SDL_GPU_TEXTUREFORMAT_D32_FLOAT_S8_UINT,
    }
}

/// Whether `format` holds depth, and maybe stencil, rather than colors.
fn is_depth_format(format: SDL_GPUTextureFormat) -> bool {
    [
        SDL_GPU_TEXTUREFORMAT_D16_UNORM,
        SDL_GPU_TEXTUREFORMAT_D24_UNORM,
        SDL_GPU_TEXTUREFORMAT_D32_FLOAT,
        SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT,
        SDL_GPU_TEXTUREFORMAT_D32_FLOAT_S8_UINT,
    ]
    .contains(&format)
}

/// The depth and stencil formats that give the screen `bits` of depth,
/// and a stencil part if asked for, best first. Without a number of bits,
/// the smallest.
fn screen_depth_formats((bits, stencil): (Option<u8>, bool)) -> &'static [SDL_GPUTextureFormat] {
    match (bits, stencil) {
        (Some(32), true) => &[
            SDL_GPU_TEXTUREFORMAT_D32_FLOAT_S8_UINT,
            SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT,
        ],
        (_, true) => &[
            SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT,
            SDL_GPU_TEXTUREFORMAT_D32_FLOAT_S8_UINT,
        ],
        (Some(32), false) => &[
            SDL_GPU_TEXTUREFORMAT_D32_FLOAT,
            SDL_GPU_TEXTUREFORMAT_D24_UNORM,
        ],
        (Some(bits), false) if bits > 16 => &[
            SDL_GPU_TEXTUREFORMAT_D24_UNORM,
            SDL_GPU_TEXTUREFORMAT_D32_FLOAT,
        ],
        (_, false) => &[SDL_GPU_TEXTUREFORMAT_D16_UNORM],
    }
}

/// Whether a depth `format` has a stencil part.
fn has_stencil(format: SDL_GPUTextureFormat) -> bool {
    [
        SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT,
        SDL_GPU_TEXTUREFORMAT_D32_FLOAT_S8_UINT,
    ]
    .contains(&format)
}

/// Creates a texture and fills it with `rgba`.
///
/// # Safety
/// `device` must be alive and `rgba` must hold `width * height * 4` bytes.
unsafe fn new_texture(
    device: *mut SDL_GPUDevice,
    format: SDL_GPUTextureFormat,
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
                format,
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

/// Creates an array, cube or volume texture and fills each layer, or
/// each slice of a volume.
///
/// # Safety
/// `device` must be alive, `kind` must not be flat, and each layer must
/// hold `width * height * 4` bytes.
unsafe fn new_layered_texture(
    device: *mut SDL_GPUDevice,
    format: SDL_GPUTextureFormat,
    kind: TextureKind,
    width: u32,
    height: u32,
    layers: &[&[u8]],
) -> Result<*mut SDL_GPUTexture, String> {
    let r#type = match kind {
        TextureKind::Cube => SDL_GPU_TEXTURETYPE_CUBE,
        TextureKind::Volume => SDL_GPU_TEXTURETYPE_3D,
        _ => SDL_GPU_TEXTURETYPE_2D_ARRAY,
    };
    // SAFETY: the create-info is valid for the call, and the new texture
    // is released if filling it fails.
    unsafe {
        let texture = SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type,
                format,
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
            // A volume's slices are depths of one layer, not layers.
            let region = if kind == TextureKind::Volume {
                Region {
                    z: index as u32,
                    width,
                    height,
                    ..Region::default()
                }
            } else {
                Region {
                    layer: index as u32,
                    width,
                    height,
                    ..Region::default()
                }
            };
            if let Err(message) = upload(device, texture, region, layer) {
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
    /// The slice of a volume texture.
    z: u32,
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
        z: 0,
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
                z: region.z,
                w: region.width,
                h: region.height,
                d: 1,
            },
            false,
        );
        SDL_EndGPUCopyPass(pass);
        SDL_SubmitGPUCommandBuffer(commands);
        SDL_ReleaseGPUTransferBuffer(device, transfer);
        Ok(())
    }
}

/// Copies `data` into `buffer` from `offset`, in a command buffer of its
/// own that runs before any frame submitted after it.
///
/// # Safety
/// `device` and `buffer` must be alive, and the bytes must fit in the
/// buffer from `offset`.
unsafe fn upload_buffer(
    device: *mut SDL_GPUDevice,
    buffer: *mut SDL_GPUBuffer,
    offset: u32,
    data: &[u8],
    cycle: bool,
) -> Result<(), String> {
    let size = u32::try_from(data.len()).map_err(|_| "the buffer is too large".to_string())?;
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
            ptr::copy_nonoverlapping(data.as_ptr(), mapped.cast::<u8>(), data.len());
            SDL_UnmapGPUTransferBuffer(device, transfer);
            SDL_AcquireGPUCommandBuffer(device)
        };
        if commands.is_null() {
            let message = format!("could not upload a buffer: {}", sdl_error());
            SDL_ReleaseGPUTransferBuffer(device, transfer);
            return Err(message);
        }
        let pass = SDL_BeginGPUCopyPass(commands);
        SDL_UploadToGPUBuffer(
            pass,
            &SDL_GPUTransferBufferLocation {
                transfer_buffer: transfer,
                offset: 0,
            },
            &SDL_GPUBufferRegion {
                buffer,
                offset,
                size,
            },
            cycle,
        );
        SDL_EndGPUCopyPass(pass);
        SDL_SubmitGPUCommandBuffer(commands);
        SDL_ReleaseGPUTransferBuffer(device, transfer);
        Ok(())
    }
}

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
use smitten_core::backend::{DrawCall, Frame, Scissor, ShaderId, ShaderInfo, TextureId, Vertex};
use smitten_core::{BlendMode, CompareMode, Filter, StencilAction, StencilMode, Wrap};

use crate::{sdl_error, shader};

// Compiled from the GLSL beside them by `shaders/build.sh`.
static VERTEX_SPIRV: &[u8] = include_bytes!("../shaders/sprite.vert.spv");
static FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite.frag.spv");

const VERTEX_SIZE: u32 = size_of::<Vertex>() as u32;
const INITIAL_VERTEX_CAPACITY: u32 = 4096;
/// The format of images and render targets. The screen may differ.
const TEXTURE_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;

/// Everything that needs its own pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    /// A custom shader, or `None` for the built-in one.
    shader: Option<u32>,
    /// Whether it draws to a render target rather than the screen.
    to_target: bool,
    blend: BlendMode,
    stencil: StencilMode,
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
    /// What shows beyond the edges, horizontally and vertically.
    wrap: (Wrap, Wrap),
}

/// A fragment shader supplied by the game.
struct CustomShader {
    fragment: *mut SDL_GPUShader,
    /// How many bytes of uniform data each draw must supply.
    uniform_size: u32,
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
    /// Built the first time each combination is drawn with.
    pipelines: HashMap<PipelineKey, *mut SDL_GPUGraphicsPipeline>,
    stencil_format: SDL_GPUTextureFormat,
    /// The screen's stencil values, and the size they were made for.
    screen_stencil: *mut SDL_GPUTexture,
    screen_stencil_size: (u32, u32),
    screen_stencil_fresh: bool,
    linear: *mut SDL_GPUSampler,
    nearest: *mut SDL_GPUSampler,
    /// Samplers for textures that do not clamp, made when first asked for.
    wrapping: HashMap<(Filter, Wrap, Wrap), *mut SDL_GPUSampler>,
    white: *mut SDL_GPUTexture,
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
            pipelines: HashMap::new(),
            stencil_format: SDL_GPU_TEXTUREFORMAT_INVALID,
            screen_stencil: ptr::null_mut(),
            screen_stencil_size: (0, 0),
            screen_stencil_fresh: false,
            linear: ptr::null_mut(),
            wrapping: HashMap::new(),
            nearest: ptr::null_mut(),
            white: ptr::null_mut(),
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
                to_target: false,
                blend: BlendMode::Alpha,
                stencil: StencilMode::Off,
            };
            let pipeline = renderer.create_pipeline(common)?;
            renderer.pipelines.insert(common, pipeline);
            renderer.linear = create_sampler(device, Filter::Linear, (Wrap::Clamp, Wrap::Clamp))?;
            renderer.nearest = create_sampler(device, Filter::Nearest, (Wrap::Clamp, Wrap::Clamp))?;
            renderer.white = new_texture(device, 1, 1, &[255, 255, 255, 255])?;
        }
        renderer.reserve_vertices(INITIAL_VERTEX_CAPACITY)?;
        Ok(renderer)
    }

    pub fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<TextureId, String> {
        if width == 0 || height == 0 {
            return Err("an image must be at least 1x1 pixels".to_string());
        }
        check_pixel_count(width, height, rgba)?;
        // SAFETY: the device is alive and `rgba` was just checked to cover
        // the whole texture.
        let handle = unsafe { new_texture(self.device, width, height, rgba)? };
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
                wrap: (Wrap::Clamp, Wrap::Clamp),
            },
        );
        Ok(TextureId(id))
    }

    pub fn create_render_target(&mut self, width: u32, height: u32) -> Result<TextureId, String> {
        if width == 0 || height == 0 {
            return Err("a canvas must be at least 1x1 pixels".to_string());
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
                    num_levels: 1,
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
                SDL_SubmitGPUCommandBuffer(commands);
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
                is_target: true,
                stencil: ptr::null_mut(),
                stencil_fresh: false,
                wrap: (Wrap::Clamp, Wrap::Clamp),
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
        unsafe { upload_region(self.device, target.handle, x, y, width, height, rgba) }
    }

    pub fn set_texture_wrap(&mut self, texture: TextureId, horizontal: Wrap, vertical: Wrap) {
        let Some(entry) = self.textures.get_mut(&texture.0) else {
            return;
        };
        let wrap = (horizontal, vertical);
        if wrap != (Wrap::Clamp, Wrap::Clamp) {
            // Made now, for both filters, so that drawing never has to
            // stop to make one.
            for filter in [Filter::Linear, Filter::Nearest] {
                if self.wrapping.contains_key(&(filter, horizontal, vertical)) {
                    continue;
                }
                // SAFETY: the device is alive.
                match unsafe { create_sampler(self.device, filter, wrap) } {
                    Ok(sampler) => {
                        self.wrapping
                            .insert((filter, horizontal, vertical), sampler);
                    }
                    Err(message) => {
                        eprintln!("smitten: {message}");
                        return;
                    }
                }
            }
        }
        entry.wrap = wrap;
    }

    pub fn destroy_texture(&mut self, texture: TextureId) {
        if let Some(texture) = self.textures.remove(&texture.0) {
            // SAFETY: the texture came from this device. SDL defers the
            // actual free until no in-flight frame uses it.
            // SDL ignores a null stencil texture.
            unsafe {
                SDL_ReleaseGPUTexture(self.device, texture.handle);
                SDL_ReleaseGPUTexture(self.device, texture.stencil);
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
                1,
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
            },
        );
        Ok(ShaderInfo {
            id: ShaderId(id),
            uniform_size: compiled.uniform_size,
            fields: compiled.fields,
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
                key.blend,
                key.stencil,
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
                    )
                };
                target.stencil_fresh = true;
            }
            let calls = (pass.first_call as usize)
                .checked_add(pass.call_count as usize)
                .and_then(|end| frame.calls.get(pass.first_call as usize..end))
                .unwrap_or(&[]);
            for call in calls {
                let key = PipelineKey {
                    shader: call.shader.map(|id| id.0),
                    to_target: pass.target.is_some(),
                    blend: call.blend,
                    stencil: call.stencil,
                };
                if !self.pipelines.contains_key(&key)
                    && let Ok(pipeline) = self.create_pipeline(key)
                {
                    self.pipelines.insert(key, pipeline);
                }
            }
        }
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
            self.screen_stencil =
                new_stencil_texture(self.device, self.stencil_format, width, height);
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

            for pass in frame.passes {
                // A pass whose target is gone, or is not there to draw on
                // this frame, is skipped.
                // New stencil memory holds garbage, so its first pass
                // clears it whatever the pass asked for.
                let (texture, stencil, fresh, width, height) = match pass.target {
                    None if window.is_none() => continue,
                    None => (
                        swapchain,
                        self.screen_stencil,
                        std::mem::take(&mut self.screen_stencil_fresh),
                        screen_width,
                        screen_height,
                    ),
                    Some(id) => match self.textures.get_mut(&id.0) {
                        Some(target) if target.is_target => (
                            target.handle,
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

                let mut info = SDL_GPUColorTargetInfo {
                    texture,
                    load_op: SDL_GPU_LOADOP_LOAD,
                    store_op: SDL_GPU_STOREOP_STORE,
                    ..Default::default()
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
                    stencil_load_op: if pass.clear_stencil || fresh {
                        SDL_GPU_LOADOP_CLEAR
                    } else {
                        SDL_GPU_LOADOP_LOAD
                    },
                    stencil_store_op: SDL_GPU_STOREOP_STORE,
                    clear_stencil: 0,
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
                        pass.target,
                        frame.vertices.len() as u32,
                        width,
                        height,
                    );
                }
                SDL_EndGPURenderPass(render_pass);
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
                let (texture, wrap) = match call.texture {
                    None => (self.white, (Wrap::Clamp, Wrap::Clamp)),
                    Some(id) => match self.textures.get(&id.0) {
                        Some(texture) => (texture.handle, texture.wrap),
                        None => continue,
                    },
                };
                let Some(scissor) = clip_scissor(call.scissor, width, height) else {
                    continue;
                };
                let key = PipelineKey {
                    shader: call.shader.map(|id| id.0),
                    to_target: target.is_some(),
                    blend: call.blend,
                    stencil: call.stencil,
                };
                // A pipeline that could not be built draws nothing.
                let Some(&pipeline) = self.pipelines.get(&key) else {
                    continue;
                };
                if let Some(id) = call.shader {
                    // A shader that was freed while still queued, or whose
                    // data is missing, draws nothing.
                    let Some(shader) = self.shaders.get(&id.0) else {
                        continue;
                    };
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
                let clamped = match call.filter {
                    Filter::Linear => self.linear,
                    Filter::Nearest => self.nearest,
                };
                let sampler = self
                    .wrapping
                    .get(&(call.filter, wrap.0, wrap.1))
                    .copied()
                    .unwrap_or(clamped);
                SDL_BindGPUFragmentSamplers(
                    pass,
                    0,
                    &SDL_GPUTextureSamplerBinding { texture, sampler },
                    1,
                );
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

/// The GPU blend settings for a blend mode, matching LÖVE's.
fn blend_state(mode: BlendMode) -> SDL_GPUColorTargetBlendState {
    let (src_color, dst_color, color_op, src_alpha, dst_alpha) = match mode {
        BlendMode::Alpha => (
            SDL_GPU_BLENDFACTOR_SRC_ALPHA,
            SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
            SDL_GPU_BLENDOP_ADD,
            SDL_GPU_BLENDFACTOR_ONE,
            SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
        ),
        BlendMode::Premultiplied => (
            SDL_GPU_BLENDFACTOR_ONE,
            SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
            SDL_GPU_BLENDOP_ADD,
            SDL_GPU_BLENDFACTOR_ONE,
            SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
        ),
        BlendMode::Add => (
            SDL_GPU_BLENDFACTOR_SRC_ALPHA,
            SDL_GPU_BLENDFACTOR_ONE,
            SDL_GPU_BLENDOP_ADD,
            SDL_GPU_BLENDFACTOR_ZERO,
            SDL_GPU_BLENDFACTOR_ONE,
        ),
        BlendMode::Subtract => (
            SDL_GPU_BLENDFACTOR_SRC_ALPHA,
            SDL_GPU_BLENDFACTOR_ONE,
            SDL_GPU_BLENDOP_REVERSE_SUBTRACT,
            SDL_GPU_BLENDFACTOR_ZERO,
            SDL_GPU_BLENDFACTOR_ONE,
        ),
        BlendMode::Multiply => (
            SDL_GPU_BLENDFACTOR_DST_COLOR,
            SDL_GPU_BLENDFACTOR_ZERO,
            SDL_GPU_BLENDOP_ADD,
            SDL_GPU_BLENDFACTOR_DST_ALPHA,
            SDL_GPU_BLENDFACTOR_ZERO,
        ),
        BlendMode::Replace => (
            SDL_GPU_BLENDFACTOR_SRC_ALPHA,
            SDL_GPU_BLENDFACTOR_ZERO,
            SDL_GPU_BLENDOP_ADD,
            SDL_GPU_BLENDFACTOR_ONE,
            SDL_GPU_BLENDFACTOR_ZERO,
        ),
    };
    SDL_GPUColorTargetBlendState {
        enable_blend: true,
        src_color_blendfactor: src_color,
        dst_color_blendfactor: dst_color,
        color_blend_op: color_op,
        src_alpha_blendfactor: src_alpha,
        dst_alpha_blendfactor: dst_alpha,
        alpha_blend_op: SDL_GPU_BLENDOP_ADD,
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
            }
            SDL_ReleaseGPUTexture(self.device, self.screen_stencil);
            SDL_ReleaseGPUTexture(self.device, self.white);
            SDL_ReleaseGPUBuffer(self.device, self.vertex_buffer);
            SDL_ReleaseGPUTransferBuffer(self.device, self.transfer_buffer);
            SDL_ReleaseGPUSampler(self.device, self.linear);
            SDL_ReleaseGPUSampler(self.device, self.nearest);
            for &sampler in self.wrapping.values() {
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
    blend: BlendMode,
    stencil: StencilMode,
) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
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

        let mut blend_state = blend_state(blend);
        // Writing to the stencil leaves the picture itself untouched.
        if matches!(stencil, StencilMode::Write(_)) {
            blend_state.enable_color_write_mask = true;
            blend_state.color_write_mask = SDL_GPUColorComponentFlags(0);
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
            rasterizer_state: SDL_GPURasterizerState {
                fill_mode: SDL_GPU_FILLMODE_FILL,
                cull_mode: SDL_GPU_CULLMODE_NONE,
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
                StencilAction::Increment => SDL_GPU_STENCILOP_INCREMENT_AND_WRAP,
                StencilAction::Decrement => SDL_GPU_STENCILOP_DECREMENT_AND_WRAP,
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
                sample_count: SDL_GPU_SAMPLECOUNT_1,
                ..Default::default()
            },
        )
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
    filter: Filter,
    wrap: (Wrap, Wrap),
) -> Result<*mut SDL_GPUSampler, String> {
    let filter = match filter {
        Filter::Linear => SDL_GPU_FILTER_LINEAR,
        Filter::Nearest => SDL_GPU_FILTER_NEAREST,
    };
    let address_mode = |wrap: Wrap| match wrap {
        Wrap::Clamp => SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        Wrap::Repeat => SDL_GPU_SAMPLERADDRESSMODE_REPEAT,
        Wrap::MirroredRepeat => SDL_GPU_SAMPLERADDRESSMODE_MIRRORED_REPEAT,
    };
    let info = SDL_GPUSamplerCreateInfo {
        min_filter: filter,
        mag_filter: filter,
        address_mode_u: address_mode(wrap.0),
        address_mode_v: address_mode(wrap.1),
        address_mode_w: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        ..Default::default()
    };
    // SAFETY: `info` is valid for the call.
    let sampler = unsafe { SDL_CreateGPUSampler(device, &info) };
    if sampler.is_null() {
        return Err(format!("could not create a sampler: {}", sdl_error()));
    }
    Ok(sampler)
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
    rgba: &[u8],
) -> Result<*mut SDL_GPUTexture, String> {
    // SAFETY: the create-info is valid for the call, and the new texture is
    // released if filling it fails.
    unsafe {
        let texture = SDL_CreateGPUTexture(
            device,
            &SDL_GPUTextureCreateInfo {
                r#type: SDL_GPU_TEXTURETYPE_2D,
                format: TEXTURE_FORMAT,
                usage: SDL_GPU_TEXTUREUSAGE_SAMPLER,
                width,
                height,
                layer_count_or_depth: 1,
                num_levels: 1,
                sample_count: SDL_GPU_SAMPLECOUNT_1,
                ..Default::default()
            },
        );
        if texture.is_null() {
            return Err(format!("could not create a texture: {}", sdl_error()));
        }
        if let Err(message) = upload_region(device, texture, 0, 0, width, height, rgba) {
            SDL_ReleaseGPUTexture(device, texture);
            return Err(message);
        }
        Ok(texture)
    }
}

/// Copies `rgba` into a region of `texture`.
///
/// # Safety
/// `device` and `texture` must be alive, the region must lie inside the
/// texture, and `rgba` must hold `width * height * 4` bytes.
unsafe fn upload_region(
    device: *mut SDL_GPUDevice,
    texture: *mut SDL_GPUTexture,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
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
            &SDL_GPUTextureTransferInfo {
                transfer_buffer: transfer,
                offset: 0,
                pixels_per_row: width,
                rows_per_layer: height,
            },
            &SDL_GPUTextureRegion {
                texture,
                x,
                y,
                w: width,
                h: height,
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

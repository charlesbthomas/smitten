//! Drawing through SDL's GPU API.
//!
//! The renderer has one pipeline: textured, vertex-colored triangles with
//! alpha blending. Shapes sample a 1x1 white texture so they can share it.
//! Each frame's vertices are uploaded once and drawn as a few runs.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use sdl3_sys::everything::*;
use smitten_core::Filter;
use smitten_core::backend::{Frame, TextureId, Vertex};

use crate::sdl_error;

// Compiled from the GLSL beside them by `shaders/build.sh`.
static VERTEX_SPIRV: &[u8] = include_bytes!("../shaders/sprite.vert.spv");
static FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite.frag.spv");

const VERTEX_SIZE: u32 = size_of::<Vertex>() as u32;
const INITIAL_VERTEX_CAPACITY: u32 = 4096;

#[derive(Clone, Copy)]
struct Texture {
    handle: *mut SDL_GPUTexture,
    width: u32,
    height: u32,
}

pub(crate) struct Renderer {
    device: *mut SDL_GPUDevice,
    pipeline: *mut SDL_GPUGraphicsPipeline,
    linear: *mut SDL_GPUSampler,
    nearest: *mut SDL_GPUSampler,
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
            pipeline: ptr::null_mut(),
            linear: ptr::null_mut(),
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
            renderer.pipeline = create_pipeline(device, window)?;
            renderer.linear = create_sampler(device, SDL_GPU_FILTER_LINEAR)?;
            renderer.nearest = create_sampler(device, SDL_GPU_FILTER_NEAREST)?;
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

    pub fn destroy_texture(&mut self, texture: TextureId) {
        if let Some(texture) = self.textures.remove(&texture.0) {
            // SAFETY: the texture came from this device. SDL defers the
            // actual free until no in-flight frame uses it.
            unsafe { SDL_ReleaseGPUTexture(self.device, texture.handle) };
        }
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

    pub fn present(&mut self, window: *mut SDL_Window, frame: &Frame) {
        // SAFETY: passes are begun and ended in the order SDL requires, and
        // the command buffer is always submitted.
        unsafe {
            let commands = SDL_AcquireGPUCommandBuffer(self.device);
            if commands.is_null() {
                return;
            }

            let have_geometry = !frame.calls.is_empty()
                && !frame.vertices.is_empty()
                && self.upload_vertices(commands, frame.vertices);

            let mut swapchain: *mut SDL_GPUTexture = ptr::null_mut();
            let (mut width, mut height) = (0u32, 0u32);
            let acquired = SDL_WaitAndAcquireGPUSwapchainTexture(
                commands,
                window,
                &mut swapchain,
                &mut width,
                &mut height,
            );

            // The texture is null while the window is minimized.
            if acquired && !swapchain.is_null() {
                let target = SDL_GPUColorTargetInfo {
                    texture: swapchain,
                    clear_color: SDL_FColor {
                        r: frame.clear.r,
                        g: frame.clear.g,
                        b: frame.clear.b,
                        a: frame.clear.a,
                    },
                    load_op: SDL_GPU_LOADOP_CLEAR,
                    store_op: SDL_GPU_STOREOP_STORE,
                    ..Default::default()
                };
                let pass = SDL_BeginGPURenderPass(commands, &target, 1, ptr::null());
                if !pass.is_null() {
                    if have_geometry && width > 0 && height > 0 {
                        self.draw_calls(commands, pass, frame, width, height);
                    }
                    SDL_EndGPURenderPass(pass);
                }
            }

            SDL_SubmitGPUCommandBuffer(commands);
        }
    }

    /// # Safety
    /// `pass` must be an open render pass on `commands`, and the vertex
    /// buffer must already hold `frame.vertices`.
    unsafe fn draw_calls(
        &self,
        commands: *mut SDL_GPUCommandBuffer,
        pass: *mut SDL_GPURenderPass,
        frame: &Frame,
        width: u32,
        height: u32,
    ) {
        // Padded to 16 bytes, the size of the shader's uniform block.
        let pixel_to_clip = [2.0 / width as f32, 2.0 / height as f32, 0.0, 0.0];
        let vertex_count = frame.vertices.len() as u32;

        // SAFETY: every handle bound here is alive, and each draw is checked
        // to stay inside the uploaded vertices.
        unsafe {
            SDL_BindGPUGraphicsPipeline(pass, self.pipeline);
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

            for call in frame.calls {
                let in_range = call
                    .first_vertex
                    .checked_add(call.vertex_count)
                    .is_some_and(|end| end <= vertex_count);
                if call.vertex_count == 0 || !in_range {
                    continue;
                }
                // A texture that was freed while still queued draws nothing.
                let texture = match call.texture {
                    None => self.white,
                    Some(id) => match self.textures.get(&id.0) {
                        Some(texture) => texture.handle,
                        None => continue,
                    },
                };
                let sampler = match call.filter {
                    Filter::Linear => self.linear,
                    Filter::Nearest => self.nearest,
                };
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

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: every handle came from this device and is released once.
        // SDL ignores null handles.
        unsafe {
            for (_, texture) in self.textures.drain() {
                SDL_ReleaseGPUTexture(self.device, texture.handle);
            }
            SDL_ReleaseGPUTexture(self.device, self.white);
            SDL_ReleaseGPUBuffer(self.device, self.vertex_buffer);
            SDL_ReleaseGPUTransferBuffer(self.device, self.transfer_buffer);
            SDL_ReleaseGPUSampler(self.device, self.linear);
            SDL_ReleaseGPUSampler(self.device, self.nearest);
            SDL_ReleaseGPUGraphicsPipeline(self.device, self.pipeline);
        }
    }
}

/// # Safety
/// `device` must be alive and own `window`.
unsafe fn create_pipeline(
    device: *mut SDL_GPUDevice,
    window: *mut SDL_Window,
) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
    // SAFETY: all pointers in the create-info structs outlive the calls
    // that read them, and the shaders are released on every path.
    unsafe {
        let vertex_shader = create_shader(device, VERTEX_SPIRV, SDL_GPU_SHADERSTAGE_VERTEX, 0, 1)?;
        let fragment_shader =
            match create_shader(device, FRAGMENT_SPIRV, SDL_GPU_SHADERSTAGE_FRAGMENT, 1, 0) {
                Ok(shader) => shader,
                Err(message) => {
                    SDL_ReleaseGPUShader(device, vertex_shader);
                    return Err(message);
                }
            };

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

        // LÖVE's default "alpha" blend mode with straight alpha.
        let targets = [SDL_GPUColorTargetDescription {
            format: SDL_GetGPUSwapchainTextureFormat(device, window),
            blend_state: SDL_GPUColorTargetBlendState {
                enable_blend: true,
                src_color_blendfactor: SDL_GPU_BLENDFACTOR_SRC_ALPHA,
                dst_color_blendfactor: SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
                color_blend_op: SDL_GPU_BLENDOP_ADD,
                src_alpha_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
                dst_alpha_blendfactor: SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
                alpha_blend_op: SDL_GPU_BLENDOP_ADD,
                ..Default::default()
            },
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
                ..Default::default()
            },
            ..Default::default()
        };
        let pipeline = SDL_CreateGPUGraphicsPipeline(device, &info);

        // The pipeline keeps what it needs from the shaders.
        SDL_ReleaseGPUShader(device, vertex_shader);
        SDL_ReleaseGPUShader(device, fragment_shader);

        if pipeline.is_null() {
            return Err(format!(
                "could not create the draw pipeline: {}",
                sdl_error()
            ));
        }
        Ok(pipeline)
    }
}

/// # Safety
/// `device` must be alive.
unsafe fn create_shader(
    device: *mut SDL_GPUDevice,
    spirv: &'static [u8],
    stage: SDL_GPUShaderStage,
    num_samplers: u32,
    num_uniform_buffers: u32,
) -> Result<*mut SDL_GPUShader, String> {
    let info = SDL_GPUShaderCreateInfo {
        code_size: spirv.len(),
        code: spirv.as_ptr(),
        entrypoint: c"main".as_ptr(),
        format: SDL_GPU_SHADERFORMAT_SPIRV,
        stage,
        num_samplers,
        num_uniform_buffers,
        ..Default::default()
    };
    // SAFETY: `info` and the bytes it points at are valid for the call.
    let shader = unsafe { SDL_CreateGPUShader(device, &info) };
    if shader.is_null() {
        return Err(format!("could not load a built-in shader: {}", sdl_error()));
    }
    Ok(shader)
}

/// # Safety
/// `device` must be alive.
unsafe fn create_sampler(
    device: *mut SDL_GPUDevice,
    filter: SDL_GPUFilter,
) -> Result<*mut SDL_GPUSampler, String> {
    let info = SDL_GPUSamplerCreateInfo {
        min_filter: filter,
        mag_filter: filter,
        address_mode_u: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_v: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
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
                format: SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
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

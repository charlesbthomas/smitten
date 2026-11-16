//! The SDL3 backend for Smitten: window and events through SDL, drawing
//! through SDL's GPU API (Vulkan, Direct3D 12 or Metal depending on the
//! platform).
//!
//! This is the only Smitten crate that contains `unsafe` code or links a
//! native library.

use std::ffi::{CStr, CString};
use std::ptr;

use sdl3_sys::everything::*;

use smitten_core::backend::{Backend, BackendEvent};
use smitten_core::{Color, Config, Key};

/// A window and GPU device owned through SDL3.
pub struct SdlBackend {
    window: *mut SDL_Window,
    device: *mut SDL_GPUDevice,
}

fn sdl_error() -> String {
    // SAFETY: SDL_GetError always returns a valid NUL-terminated string.
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}

fn c_string(text: &str) -> CString {
    // Interior NULs cannot be represented in a C string, so drop them.
    CString::new(text.replace('\0', "")).expect("NULs were removed")
}

impl SdlBackend {
    /// Initializes SDL and opens the game window.
    ///
    /// Must be called on the main thread. The error is a message suitable
    /// for showing to the developer.
    pub fn new(config: &Config) -> Result<Self, String> {
        // SAFETY: plain SDL calls on the main thread. Every pointer passed is
        // valid for the duration of its call, and each failure path releases
        // what was created before it.
        unsafe {
            if !SDL_Init(SDL_INIT_VIDEO) {
                return Err(format!("SDL_Init failed: {}", sdl_error()));
            }

            let mut flags = SDL_WINDOW_HIGH_PIXEL_DENSITY;
            if config.resizable {
                flags |= SDL_WINDOW_RESIZABLE;
            }
            let title = c_string(&config.title);
            let window = SDL_CreateWindow(
                title.as_ptr(),
                config.width as i32,
                config.height as i32,
                flags,
            );
            if window.is_null() {
                let message = format!("could not create a window: {}", sdl_error());
                SDL_Quit();
                return Err(message);
            }

            // Declare every shader format Smitten can supply so SDL is free
            // to pick the best driver for the platform.
            let formats =
                SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_DXIL | SDL_GPU_SHADERFORMAT_MSL;
            let device = SDL_CreateGPUDevice(formats, cfg!(debug_assertions), ptr::null());
            if device.is_null() {
                let message = format!("could not create a GPU device: {}", sdl_error());
                SDL_DestroyWindow(window);
                SDL_Quit();
                return Err(message);
            }

            if !SDL_ClaimWindowForGPUDevice(device, window) {
                let message = format!("could not attach the GPU to the window: {}", sdl_error());
                SDL_DestroyGPUDevice(device);
                SDL_DestroyWindow(window);
                SDL_Quit();
                return Err(message);
            }

            if !config.vsync
                && SDL_WindowSupportsGPUPresentMode(device, window, SDL_GPU_PRESENTMODE_IMMEDIATE)
            {
                SDL_SetGPUSwapchainParameters(
                    device,
                    window,
                    SDL_GPU_SWAPCHAINCOMPOSITION_SDR,
                    SDL_GPU_PRESENTMODE_IMMEDIATE,
                );
            }

            Ok(Self { window, device })
        }
    }
}

impl Drop for SdlBackend {
    fn drop(&mut self) {
        // SAFETY: both pointers were created in `new` and are released once.
        unsafe {
            SDL_ReleaseWindowFromGPUDevice(self.device, self.window);
            SDL_DestroyGPUDevice(self.device);
            SDL_DestroyWindow(self.window);
            SDL_Quit();
        }
    }
}

impl Backend for SdlBackend {
    fn poll_event(&mut self) -> Option<BackendEvent> {
        // SAFETY: SDL_PollEvent fills the event it is given, and the union
        // field read in each arm is the one SDL documents for that type.
        unsafe {
            let mut event: SDL_Event = std::mem::zeroed();
            while SDL_PollEvent(&mut event) {
                let kind = SDL_EventType(event.r#type);
                if kind == SDL_EVENT_QUIT {
                    return Some(BackendEvent::Quit);
                } else if kind == SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED {
                    return Some(BackendEvent::Resized {
                        width: event.window.data1.max(0) as u32,
                        height: event.window.data2.max(0) as u32,
                    });
                } else if kind == SDL_EVENT_KEY_DOWN {
                    if let Some(key) = key_from_scancode(event.key.scancode) {
                        return Some(BackendEvent::KeyDown {
                            key,
                            is_repeat: event.key.repeat,
                        });
                    }
                } else if kind == SDL_EVENT_KEY_UP
                    && let Some(key) = key_from_scancode(event.key.scancode)
                {
                    return Some(BackendEvent::KeyUp { key });
                }
                // Anything else is not surfaced yet; keep draining.
            }
            None
        }
    }

    fn present(&mut self, clear: Color) {
        // SAFETY: the command buffer and render pass are used in the order
        // SDL requires, and the command buffer is always submitted.
        unsafe {
            let commands = SDL_AcquireGPUCommandBuffer(self.device);
            if commands.is_null() {
                return;
            }

            let mut swapchain: *mut SDL_GPUTexture = ptr::null_mut();
            let acquired = SDL_WaitAndAcquireGPUSwapchainTexture(
                commands,
                self.window,
                &mut swapchain,
                ptr::null_mut(),
                ptr::null_mut(),
            );

            // The texture is null while the window is minimized.
            if acquired && !swapchain.is_null() {
                let target = SDL_GPUColorTargetInfo {
                    texture: swapchain,
                    clear_color: SDL_FColor {
                        r: clear.r,
                        g: clear.g,
                        b: clear.b,
                        a: clear.a,
                    },
                    load_op: SDL_GPU_LOADOP_CLEAR,
                    store_op: SDL_GPU_STOREOP_STORE,
                    ..Default::default()
                };
                let pass = SDL_BeginGPURenderPass(commands, &target, 1, ptr::null());
                if !pass.is_null() {
                    SDL_EndGPURenderPass(pass);
                }
            }

            SDL_SubmitGPUCommandBuffer(commands);
        }
    }

    fn ticks_ns(&self) -> u64 {
        // SAFETY: no preconditions once SDL is initialized.
        unsafe { SDL_GetTicksNS() }
    }

    fn size_in_pixels(&self) -> (u32, u32) {
        let (mut width, mut height) = (0, 0);
        // SAFETY: the window is alive and both out-pointers are valid.
        unsafe { SDL_GetWindowSizeInPixels(self.window, &mut width, &mut height) };
        (width.max(0) as u32, height.max(0) as u32)
    }

    fn set_title(&mut self, title: &str) {
        let title = c_string(title);
        // SAFETY: the window is alive and SDL copies the string.
        unsafe { SDL_SetWindowTitle(self.window, title.as_ptr()) };
    }
}

fn key_from_scancode(scancode: SDL_Scancode) -> Option<Key> {
    use Key::*;

    const LETTERS: [Key; 26] = [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    ];
    // SDL orders the digit row 1 through 9 and then 0.
    const DIGITS: [Key; 10] = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0];
    const FUNCTION: [Key; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];

    let code = scancode.0;
    let in_run = |first: SDL_Scancode, keys: &[Key]| {
        let offset = code - first.0;
        (offset >= 0 && (offset as usize) < keys.len()).then(|| keys[offset as usize])
    };

    if let Some(key) = in_run(SDL_SCANCODE_A, &LETTERS)
        .or_else(|| in_run(SDL_SCANCODE_1, &DIGITS))
        .or_else(|| in_run(SDL_SCANCODE_F1, &FUNCTION))
    {
        return Some(key);
    }

    let named = [
        (SDL_SCANCODE_UP, Up),
        (SDL_SCANCODE_DOWN, Down),
        (SDL_SCANCODE_LEFT, Left),
        (SDL_SCANCODE_RIGHT, Right),
        (SDL_SCANCODE_SPACE, Space),
        (SDL_SCANCODE_RETURN, Return),
        (SDL_SCANCODE_ESCAPE, Escape),
        (SDL_SCANCODE_TAB, Tab),
        (SDL_SCANCODE_BACKSPACE, Backspace),
        (SDL_SCANCODE_LSHIFT, LeftShift),
        (SDL_SCANCODE_RSHIFT, RightShift),
        (SDL_SCANCODE_LCTRL, LeftCtrl),
        (SDL_SCANCODE_RCTRL, RightCtrl),
        (SDL_SCANCODE_LALT, LeftAlt),
        (SDL_SCANCODE_RALT, RightAlt),
    ];
    named
        .iter()
        .find(|(sdl, _)| sdl.0 == code)
        .map(|&(_, key)| key)
}

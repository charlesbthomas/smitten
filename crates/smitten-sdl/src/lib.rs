//! The SDL3 backend for Smitten: window and events through SDL, drawing
//! through SDL's GPU API (Vulkan, Direct3D 12 or Metal depending on the
//! platform).
//!
//! This is the only Smitten crate that contains `unsafe` code or links a
//! native library.

mod renderer;

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::mem::ManuallyDrop;
use std::ptr;

use sdl3_sys::everything::*;

use smitten_core::backend::{Backend, BackendEvent, Frame, TextureId};
use smitten_core::{Config, GamepadAxis, GamepadButton, JoystickId, Key, MouseButton};

use renderer::Renderer;

/// A window and GPU device owned through SDL3.
pub struct SdlBackend {
    window: *mut SDL_Window,
    device: *mut SDL_GPUDevice,
    /// Dropped by hand in `Drop`, before the device it draws with.
    renderer: ManuallyDrop<Renderer>,
    /// Opened gamepads by SDL joystick id. SDL only reports input from
    /// gamepads that are open.
    gamepads: HashMap<u32, *mut SDL_Gamepad>,
}

pub(crate) fn sdl_error() -> String {
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
            if !SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMEPAD) {
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

            // The built-in shaders are SPIR-V only for now, which selects the
            // Vulkan driver. Direct3D 12 and Metal need their own shader
            // formats before they can be offered here.
            let formats = SDL_GPU_SHADERFORMAT_SPIRV;
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

            let renderer = match Renderer::new(device, window) {
                Ok(renderer) => renderer,
                Err(message) => {
                    SDL_ReleaseWindowFromGPUDevice(device, window);
                    SDL_DestroyGPUDevice(device);
                    SDL_DestroyWindow(window);
                    SDL_Quit();
                    return Err(message);
                }
            };

            // Typed text is reported by default, as LÖVE does on desktop.
            SDL_StartTextInput(window);

            Ok(Self {
                window,
                device,
                renderer: ManuallyDrop::new(renderer),
                gamepads: HashMap::new(),
            })
        }
    }
}

impl Drop for SdlBackend {
    fn drop(&mut self) {
        // SAFETY: everything was created in `new` and is released once, the
        // renderer first because its resources belong to the device.
        unsafe {
            for (_, gamepad) in self.gamepads.drain() {
                SDL_CloseGamepad(gamepad);
            }
            ManuallyDrop::drop(&mut self.renderer);
            SDL_ReleaseWindowFromGPUDevice(self.device, self.window);
            SDL_DestroyGPUDevice(self.device);
            SDL_DestroyWindow(self.window);
            SDL_Quit();
        }
    }
}

impl Backend for SdlBackend {
    fn poll_event(&mut self) -> Option<BackendEvent> {
        // SAFETY: SDL_PollEvent fills the event it is given.
        unsafe {
            let mut event: SDL_Event = std::mem::zeroed();
            while SDL_PollEvent(&mut event) {
                if let Some(translated) = self.translate(&event) {
                    return Some(translated);
                }
                // Anything else is not surfaced yet; keep draining.
            }
            None
        }
    }

    fn set_mouse_visible(&mut self, visible: bool) {
        // SAFETY: no preconditions once the video subsystem is running.
        unsafe {
            if visible {
                SDL_ShowCursor();
            } else {
                SDL_HideCursor();
            }
        }
    }

    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<TextureId, String> {
        self.renderer.create_texture(width, height, rgba)
    }

    fn destroy_texture(&mut self, texture: TextureId) {
        self.renderer.destroy_texture(texture);
    }

    fn present(&mut self, frame: &Frame) {
        self.renderer.present(self.window, frame);
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

impl SdlBackend {
    /// Turns an SDL event into a Smitten one, if Smitten has an equivalent.
    ///
    /// # Safety
    /// `event` must have been filled in by SDL, so that the union field
    /// matching its type is the valid one.
    unsafe fn translate(&mut self, event: &SDL_Event) -> Option<BackendEvent> {
        // SAFETY: each arm reads only the union field SDL documents for
        // that event type.
        unsafe {
            let kind = SDL_EventType(event.r#type);
            if kind == SDL_EVENT_QUIT {
                Some(BackendEvent::Quit)
            } else if kind == SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED {
                Some(BackendEvent::Resized {
                    width: event.window.data1.max(0) as u32,
                    height: event.window.data2.max(0) as u32,
                })
            } else if kind == SDL_EVENT_KEY_DOWN {
                let key = key_from_scancode(event.key.scancode)?;
                Some(BackendEvent::KeyDown {
                    key,
                    is_repeat: event.key.repeat,
                })
            } else if kind == SDL_EVENT_KEY_UP {
                let key = key_from_scancode(event.key.scancode)?;
                Some(BackendEvent::KeyUp { key })
            } else if kind == SDL_EVENT_TEXT_INPUT {
                if event.text.text.is_null() {
                    return None;
                }
                let text = CStr::from_ptr(event.text.text).to_string_lossy();
                Some(BackendEvent::TextInput(text.into_owned()))
            } else if kind == SDL_EVENT_MOUSE_MOTION {
                // SDL reports window coordinates; Smitten works in pixels.
                let scale = self.pixel_density();
                Some(BackendEvent::MouseMoved {
                    x: event.motion.x * scale,
                    y: event.motion.y * scale,
                    dx: event.motion.xrel * scale,
                    dy: event.motion.yrel * scale,
                })
            } else if kind == SDL_EVENT_MOUSE_BUTTON_DOWN || kind == SDL_EVENT_MOUSE_BUTTON_UP {
                let scale = self.pixel_density();
                let button = mouse_button(event.button.button)?;
                let (x, y) = (event.button.x * scale, event.button.y * scale);
                let presses = u32::from(event.button.clicks);
                Some(if event.button.down {
                    BackendEvent::MousePressed {
                        x,
                        y,
                        button,
                        presses,
                    }
                } else {
                    BackendEvent::MouseReleased {
                        x,
                        y,
                        button,
                        presses,
                    }
                })
            } else if kind == SDL_EVENT_MOUSE_WHEEL {
                // "Flipped" means the system reports natural scrolling with
                // the signs reversed; undo that so up is always positive.
                let sign = if event.wheel.direction == SDL_MOUSEWHEEL_FLIPPED {
                    -1.0
                } else {
                    1.0
                };
                Some(BackendEvent::WheelMoved {
                    dx: event.wheel.x * sign,
                    dy: event.wheel.y * sign,
                })
            } else if kind == SDL_EVENT_GAMEPAD_ADDED {
                let which = event.gdevice.which;
                if self.gamepads.contains_key(&which.0) {
                    return None;
                }
                let gamepad = SDL_OpenGamepad(which);
                if gamepad.is_null() {
                    return None;
                }
                self.gamepads.insert(which.0, gamepad);
                let name = SDL_GetGamepadName(gamepad);
                let name = if name.is_null() {
                    "Gamepad".to_string()
                } else {
                    CStr::from_ptr(name).to_string_lossy().into_owned()
                };
                Some(BackendEvent::JoystickAdded {
                    id: JoystickId(which.0),
                    name,
                })
            } else if kind == SDL_EVENT_GAMEPAD_REMOVED {
                let which = event.gdevice.which;
                let gamepad = self.gamepads.remove(&which.0)?;
                SDL_CloseGamepad(gamepad);
                Some(BackendEvent::JoystickRemoved {
                    id: JoystickId(which.0),
                })
            } else if kind == SDL_EVENT_GAMEPAD_BUTTON_DOWN || kind == SDL_EVENT_GAMEPAD_BUTTON_UP {
                let id = JoystickId(event.gbutton.which.0);
                let button = gamepad_button(event.gbutton.button)?;
                Some(if event.gbutton.down {
                    BackendEvent::GamepadPressed { id, button }
                } else {
                    BackendEvent::GamepadReleased { id, button }
                })
            } else if kind == SDL_EVENT_GAMEPAD_AXIS_MOTION {
                let axis = gamepad_axis(event.gaxis.axis)?;
                Some(BackendEvent::GamepadAxis {
                    id: JoystickId(event.gaxis.which.0),
                    axis,
                    // SDL's range is -32768 to 32767.
                    value: (f32::from(event.gaxis.value) / 32767.0).clamp(-1.0, 1.0),
                })
            } else {
                None
            }
        }
    }

    /// Pixels per window coordinate, above 1.0 on high-DPI displays.
    fn pixel_density(&self) -> f32 {
        // SAFETY: the window is alive.
        let density = unsafe { SDL_GetWindowPixelDensity(self.window) };
        if density > 0.0 { density } else { 1.0 }
    }
}

fn mouse_button(button: u8) -> Option<MouseButton> {
    let button = i32::from(button);
    [
        (SDL_BUTTON_LEFT, MouseButton::Left),
        (SDL_BUTTON_RIGHT, MouseButton::Right),
        (SDL_BUTTON_MIDDLE, MouseButton::Middle),
        (SDL_BUTTON_X1, MouseButton::X1),
        (SDL_BUTTON_X2, MouseButton::X2),
    ]
    .iter()
    .find(|(sdl, _)| *sdl == button)
    .map(|&(_, button)| button)
}

fn gamepad_button(button: u8) -> Option<GamepadButton> {
    use GamepadButton::*;
    let button = i32::from(button);
    [
        (SDL_GAMEPAD_BUTTON_SOUTH, A),
        (SDL_GAMEPAD_BUTTON_EAST, B),
        (SDL_GAMEPAD_BUTTON_WEST, X),
        (SDL_GAMEPAD_BUTTON_NORTH, Y),
        (SDL_GAMEPAD_BUTTON_BACK, Back),
        (SDL_GAMEPAD_BUTTON_GUIDE, Guide),
        (SDL_GAMEPAD_BUTTON_START, Start),
        (SDL_GAMEPAD_BUTTON_LEFT_STICK, LeftStick),
        (SDL_GAMEPAD_BUTTON_RIGHT_STICK, RightStick),
        (SDL_GAMEPAD_BUTTON_LEFT_SHOULDER, LeftShoulder),
        (SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER, RightShoulder),
        (SDL_GAMEPAD_BUTTON_DPAD_UP, DpadUp),
        (SDL_GAMEPAD_BUTTON_DPAD_DOWN, DpadDown),
        (SDL_GAMEPAD_BUTTON_DPAD_LEFT, DpadLeft),
        (SDL_GAMEPAD_BUTTON_DPAD_RIGHT, DpadRight),
    ]
    .iter()
    .find(|(sdl, _)| sdl.0 == button)
    .map(|&(_, button)| button)
}

fn gamepad_axis(axis: u8) -> Option<GamepadAxis> {
    let axis = i32::from(axis);
    [
        (SDL_GAMEPAD_AXIS_LEFTX, GamepadAxis::LeftX),
        (SDL_GAMEPAD_AXIS_LEFTY, GamepadAxis::LeftY),
        (SDL_GAMEPAD_AXIS_RIGHTX, GamepadAxis::RightX),
        (SDL_GAMEPAD_AXIS_RIGHTY, GamepadAxis::RightY),
        (SDL_GAMEPAD_AXIS_LEFT_TRIGGER, GamepadAxis::TriggerLeft),
        (SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, GamepadAxis::TriggerRight),
    ]
    .iter()
    .find(|(sdl, _)| sdl.0 == axis)
    .map(|&(_, axis)| axis)
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
        (SDL_SCANCODE_MINUS, Minus),
        (SDL_SCANCODE_EQUALS, Equals),
        (SDL_SCANCODE_LEFTBRACKET, LeftBracket),
        (SDL_SCANCODE_RIGHTBRACKET, RightBracket),
        (SDL_SCANCODE_BACKSLASH, Backslash),
        (SDL_SCANCODE_SEMICOLON, Semicolon),
        (SDL_SCANCODE_APOSTROPHE, Apostrophe),
        (SDL_SCANCODE_GRAVE, Grave),
        (SDL_SCANCODE_COMMA, Comma),
        (SDL_SCANCODE_PERIOD, Period),
        (SDL_SCANCODE_SLASH, Slash),
        (SDL_SCANCODE_CAPSLOCK, CapsLock),
        (SDL_SCANCODE_INSERT, Insert),
        (SDL_SCANCODE_DELETE, Delete),
        (SDL_SCANCODE_HOME, Home),
        (SDL_SCANCODE_END, End),
        (SDL_SCANCODE_PAGEUP, PageUp),
        (SDL_SCANCODE_PAGEDOWN, PageDown),
        (SDL_SCANCODE_LGUI, LeftGui),
        (SDL_SCANCODE_RGUI, RightGui),
    ];
    named
        .iter()
        .find(|(sdl, _)| sdl.0 == code)
        .map(|&(_, key)| key)
}

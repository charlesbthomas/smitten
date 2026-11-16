//! The SDL3 backend for Smitten: window and events through SDL, drawing
//! through SDL's GPU API (Vulkan, Direct3D 12 or Metal depending on the
//! platform).
//!
//! This is the only Smitten crate that contains `unsafe` code or links a
//! native library.

mod audio;
mod renderer;
mod shader;

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::mem::ManuallyDrop;
use std::path::PathBuf;
use std::ptr;

use sdl3_sys::everything::*;

use smitten_core::backend::{
    AudioFill, Backend, BackendEvent, Frame, ShaderId, ShaderInfo, TextureId,
};
use smitten_core::{
    Config, Cursor, GamepadAxis, GamepadButton, JoystickId, Key, MouseButton, PowerInfo, PowerState,
};

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
    /// Present once `start_audio` has succeeded. Dropping it stops playback.
    audio: Option<audio::AudioOutput>,
    /// System cursors made so far. SDL frees them when it shuts down.
    cursors: HashMap<Cursor, *mut SDL_Cursor>,
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
            if config.fullscreen {
                flags |= SDL_WINDOW_FULLSCREEN;
            }
            // Where window sizes are in pixels (Windows, X11), a display
            // set to 150% needs a window 1.5 times as large to show the
            // same number of units. Where they are in points (macOS,
            // Wayland) this scale is 1 and the pixels come for free.
            let content_scale = if config.dpi_scaling {
                let scale = SDL_GetDisplayContentScale(SDL_GetPrimaryDisplay());
                if scale > 0.0 { scale } else { 1.0 }
            } else {
                1.0
            };
            let title = c_string(&config.title);
            let window = SDL_CreateWindow(
                title.as_ptr(),
                (config.width as f32 * content_scale).round() as i32,
                (config.height as f32 * content_scale).round() as i32,
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
                audio: None,
                cursors: HashMap::new(),
            })
        }
    }
}

impl Drop for SdlBackend {
    fn drop(&mut self) {
        // SAFETY: everything was created in `new` and is released once, the
        // renderer first because its resources belong to the device.
        unsafe {
            // Stop the audio thread before tearing anything else down.
            self.audio = None;
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

    fn start_audio(&mut self, fill: AudioFill) -> Result<(), String> {
        if self.audio.is_some() {
            return Err("audio is already running".to_string());
        }
        self.audio = Some(audio::AudioOutput::open(fill)?);
        Ok(())
    }

    fn save_directory(&mut self, identity: &str) -> Result<PathBuf, String> {
        let identity = c_string(identity);
        // SAFETY: both strings are valid for the call. SDL returns a string
        // it allocated, which is copied and then freed exactly once.
        unsafe {
            // An empty organization puts the folder directly under the
            // user's application data directory.
            let path = SDL_GetPrefPath(c"".as_ptr(), identity.as_ptr());
            if path.is_null() {
                return Err(format!("could not find a save directory: {}", sdl_error()));
            }
            let owned = CStr::from_ptr(path).to_string_lossy().into_owned();
            SDL_free(path.cast());
            Ok(PathBuf::from(owned))
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

    fn set_cursor(&mut self, cursor: Cursor) -> Result<(), String> {
        let id = match cursor {
            Cursor::Arrow => SDL_SYSTEM_CURSOR_DEFAULT,
            Cursor::IBeam => SDL_SYSTEM_CURSOR_TEXT,
            Cursor::Wait => SDL_SYSTEM_CURSOR_WAIT,
            Cursor::WaitArrow => SDL_SYSTEM_CURSOR_PROGRESS,
            Cursor::Crosshair => SDL_SYSTEM_CURSOR_CROSSHAIR,
            Cursor::Hand => SDL_SYSTEM_CURSOR_POINTER,
            Cursor::No => SDL_SYSTEM_CURSOR_NOT_ALLOWED,
            Cursor::SizeAll => SDL_SYSTEM_CURSOR_MOVE,
            Cursor::SizeNs => SDL_SYSTEM_CURSOR_NS_RESIZE,
            Cursor::SizeWe => SDL_SYSTEM_CURSOR_EW_RESIZE,
            Cursor::SizeNwse => SDL_SYSTEM_CURSOR_NWSE_RESIZE,
            Cursor::SizeNesw => SDL_SYSTEM_CURSOR_NESW_RESIZE,
        };
        // SAFETY: plain SDL calls on the main thread. A cursor stays valid
        // until SDL shuts down, which is after its last use here.
        unsafe {
            let handle = match self.cursors.get(&cursor) {
                Some(&handle) => handle,
                None => {
                    let handle = SDL_CreateSystemCursor(id);
                    if handle.is_null() {
                        return Err(sdl_error());
                    }
                    self.cursors.insert(cursor, handle);
                    handle
                }
            };
            if SDL_SetCursor(handle) {
                Ok(())
            } else {
                Err(sdl_error())
            }
        }
    }

    fn set_relative_mouse(&mut self, relative: bool) -> Result<(), String> {
        // SAFETY: the window is alive.
        if unsafe { SDL_SetWindowRelativeMouseMode(self.window, relative) } {
            Ok(())
        } else {
            Err(sdl_error())
        }
    }

    fn set_mouse_grabbed(&mut self, grabbed: bool) -> Result<(), String> {
        // SAFETY: the window is alive.
        if unsafe { SDL_SetWindowMouseGrab(self.window, grabbed) } {
            Ok(())
        } else {
            Err(sdl_error())
        }
    }

    fn warp_mouse(&mut self, x: f32, y: f32) {
        // SDL takes window coordinates; Smitten works in pixels.
        let scale = self.pixel_density();
        // SAFETY: the window is alive.
        unsafe { SDL_WarpMouseInWindow(self.window, x / scale, y / scale) };
    }

    fn set_clipboard_text(&mut self, text: &str) -> Result<(), String> {
        let text = c_string(text);
        // SAFETY: SDL copies the string.
        if unsafe { SDL_SetClipboardText(text.as_ptr()) } {
            Ok(())
        } else {
            Err(sdl_error())
        }
    }

    fn clipboard_text(&mut self) -> String {
        // SAFETY: SDL returns a NUL-terminated string that the caller
        // frees, or null if it could not allocate one.
        unsafe {
            let text = SDL_GetClipboardText();
            if text.is_null() {
                return String::new();
            }
            let owned = CStr::from_ptr(text).to_string_lossy().into_owned();
            SDL_free(text.cast());
            owned
        }
    }

    fn open_url(&mut self, url: &str) -> Result<(), String> {
        let url = c_string(url);
        // SAFETY: the string is valid for the call.
        if unsafe { SDL_OpenURL(url.as_ptr()) } {
            Ok(())
        } else {
            Err(sdl_error())
        }
    }

    fn power_info(&mut self) -> PowerInfo {
        let (mut seconds, mut percent) = (-1, -1);
        // SAFETY: SDL fills both numbers, with -1 for what it cannot tell.
        let state = unsafe { SDL_GetPowerInfo(&mut seconds, &mut percent) };
        PowerInfo {
            state: if state == SDL_POWERSTATE_ON_BATTERY {
                PowerState::Battery
            } else if state == SDL_POWERSTATE_NO_BATTERY {
                PowerState::NoBattery
            } else if state == SDL_POWERSTATE_CHARGING {
                PowerState::Charging
            } else if state == SDL_POWERSTATE_CHARGED {
                PowerState::Charged
            } else {
                PowerState::Unknown
            },
            percent: u8::try_from(percent).ok(),
            seconds: u32::try_from(seconds).ok(),
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

    fn update_texture(
        &mut self,
        texture: TextureId,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), String> {
        self.renderer
            .update_texture(texture, x, y, width, height, rgba)
    }

    fn create_render_target(&mut self, width: u32, height: u32) -> Result<TextureId, String> {
        self.renderer.create_render_target(width, height)
    }

    fn destroy_texture(&mut self, texture: TextureId) {
        self.renderer.destroy_texture(texture);
    }

    fn create_shader(&mut self, source: &str) -> Result<ShaderInfo, String> {
        self.renderer.create_shader(source)
    }

    fn destroy_shader(&mut self, shader: ShaderId) {
        self.renderer.destroy_shader(shader);
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

    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), String> {
        // SAFETY: the window is alive. With no fullscreen mode chosen, SDL
        // uses the desktop's own resolution, without changing video mode.
        unsafe {
            if !SDL_SetWindowFullscreen(self.window, fullscreen) {
                return Err(sdl_error());
            }
            // The change may be applied later by the window system. Wait
            // for it so the new size can be read straight away.
            SDL_SyncWindow(self.window);
        }
        Ok(())
    }

    fn is_fullscreen(&self) -> bool {
        // SAFETY: the window is alive.
        let flags = unsafe { SDL_GetWindowFlags(self.window) };
        flags & SDL_WINDOW_FULLSCREEN != 0
    }

    fn set_icon(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return Err(format!(
                "an icon of {width} by {height} pixels needs {} bytes, got {}",
                width as usize * height as usize * 4,
                rgba.len()
            ));
        }
        // SAFETY: the surface borrows `rgba`, which holds `height` rows of
        // `width * 4` bytes as checked above and outlives the surface. SDL
        // copies the pixels when the icon is set and never writes to them.
        unsafe {
            let surface = SDL_CreateSurfaceFrom(
                width as i32,
                height as i32,
                SDL_PIXELFORMAT_RGBA32,
                rgba.as_ptr().cast_mut().cast(),
                width as i32 * 4,
            );
            if surface.is_null() {
                return Err(sdl_error());
            }
            let done = SDL_SetWindowIcon(self.window, surface);
            let message = if done { String::new() } else { sdl_error() };
            SDL_DestroySurface(surface);
            if done { Ok(()) } else { Err(message) }
        }
    }

    fn dpi_scale(&self) -> f32 {
        // SAFETY: the window is alive.
        let scale = unsafe { SDL_GetWindowDisplayScale(self.window) };
        if scale > 0.0 { scale } else { 1.0 }
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

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
    AudioFill, Backend, BackendEvent, CompressedFormat, CursorId, Frame, RecordingDeviceId,
    RecordingSink, RendererInfo, ShaderId, ShaderInfo, TextureId, TextureKind,
};
use smitten_core::{
    Config, Cursor, DisplayOrientation, Filter, FullscreenType, GamepadAxis, GamepadButton,
    JoystickHat, JoystickId, JoystickInfo, Key, MessageBoxKind, MouseButton, PowerInfo, PowerState,
    TouchId, WindowState, Wrap,
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
    /// Every opened joystick by SDL joystick id, gamepads included.
    joysticks: HashMap<u32, *mut SDL_Joystick>,
    /// Present once `start_audio` has succeeded. Dropping it stops playback.
    audio: Option<audio::AudioOutput>,
    /// The recording devices that are capturing.
    recordings: HashMap<RecordingDeviceId, audio::AudioInput>,
    /// Held from the first time recording devices are listed, so that
    /// their ids stay valid.
    audio_subsystem: Option<audio::AudioSubsystem>,
    /// System cursors made so far. SDL frees them when it shuts down.
    cursors: HashMap<Cursor, *mut SDL_Cursor>,
    /// Cursors made from images, freed by `destroy_cursor`.
    image_cursors: HashMap<u32, *mut SDL_Cursor>,
    next_image_cursor: u32,
    /// Whether presenting waits for the display's refresh.
    vsync: bool,
    /// Whether window sizes are multiplied by the display's scale.
    content_scaling: bool,
    /// Whether the window was last reported shown rather than hidden or
    /// minimized.
    visible: bool,
    /// The device's own vibration motor, opened on the first `vibrate`.
    /// Null if it has not been opened or there is none.
    vibrator: *mut SDL_Haptic,
    /// Whether opening the vibration motor has been tried.
    vibrator_tried: bool,
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

            // Hidden until it has been placed and given its mode, so it
            // does not appear in one place and then jump to another.
            let mut flags = SDL_WINDOW_HIGH_PIXEL_DENSITY | SDL_WINDOW_HIDDEN;
            if config.resizable {
                flags |= SDL_WINDOW_RESIZABLE;
            }
            if config.borderless {
                flags |= SDL_WINDOW_BORDERLESS;
            }
            let display = display_id(config.display).unwrap_or_else(|| SDL_GetPrimaryDisplay());
            // Where window sizes are in pixels (Windows, X11), a display
            // set to 150% needs a window 1.5 times as large to show the
            // same number of units. Where they are in points (macOS,
            // Wayland) this scale is 1 and the pixels come for free.
            let content_scale = if config.dpi_scaling {
                let scale = SDL_GetDisplayContentScale(display);
                if scale > 0.0 { scale } else { 1.0 }
            } else {
                1.0
            };
            let mut vsync = true;
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

            SDL_SetWindowMinimumSize(
                window,
                (config.min_width.max(1) as f32 * content_scale).round() as i32,
                (config.min_height.max(1) as f32 * content_scale).round() as i32,
            );
            let mut bounds = SDL_Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            };
            if let Some((x, y)) = config.position {
                SDL_GetDisplayBounds(display, &mut bounds);
                SDL_SetWindowPosition(window, bounds.x + x, bounds.y + y);
            } else if config.centered || display != SDL_GetPrimaryDisplay() {
                let centered = SDL_WINDOWPOS_CENTERED_DISPLAY(display);
                SDL_SetWindowPosition(window, centered, centered);
            }
            if config.fullscreen {
                if config.fullscreen_type == FullscreenType::Exclusive {
                    let mut mode: SDL_DisplayMode = std::mem::zeroed();
                    let (mut width, mut height) = (0, 0);
                    SDL_GetWindowSize(window, &mut width, &mut height);
                    if SDL_GetClosestFullscreenDisplayMode(
                        display, width, height, 0.0, true, &mut mode,
                    ) {
                        SDL_SetWindowFullscreenMode(window, &mode);
                    }
                }
                SDL_SetWindowFullscreen(window, true);
            }
            SDL_ShowWindow(window);
            SDL_SyncWindow(window);

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
                vsync = !SDL_SetGPUSwapchainParameters(
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
                joysticks: HashMap::new(),
                audio: None,
                recordings: HashMap::new(),
                audio_subsystem: None,
                cursors: HashMap::new(),
                image_cursors: HashMap::new(),
                next_image_cursor: 0,
                vsync,
                content_scaling: config.dpi_scaling,
                visible: true,
                vibrator: ptr::null_mut(),
                vibrator_tried: false,
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
            self.recordings.clear();
            self.audio_subsystem = None;
            for (_, gamepad) in self.gamepads.drain() {
                SDL_CloseGamepad(gamepad);
            }
            for (_, joystick) in self.joysticks.drain() {
                SDL_CloseJoystick(joystick);
            }
            for (_, cursor) in self.image_cursors.drain() {
                SDL_DestroyCursor(cursor);
            }
            if !self.vibrator.is_null() {
                SDL_CloseHaptic(self.vibrator);
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

    fn wait_event(&mut self, timeout_ms: Option<u32>) -> Option<BackendEvent> {
        // SAFETY: SDL_GetTicks is a plain query.
        let deadline = timeout_ms.map(|ms| unsafe { SDL_GetTicks() } + u64::from(ms));
        // SAFETY: SDL_WaitEventTimeout fills the event it is given.
        unsafe {
            let mut event: SDL_Event = std::mem::zeroed();
            loop {
                let wait = match deadline {
                    None => -1,
                    Some(deadline) => {
                        let left = deadline.saturating_sub(SDL_GetTicks());
                        left.min(i32::MAX as u64) as i32
                    }
                };
                if !SDL_WaitEventTimeout(&mut event, wait) {
                    return None;
                }
                if let Some(translated) = self.translate(&event) {
                    return Some(translated);
                }
                // Not surfaced; wait on for one that is, unless time is up.
                if deadline.is_some_and(|deadline| SDL_GetTicks() >= deadline) {
                    return None;
                }
            }
        }
    }

    fn start_audio(&mut self, fill: AudioFill) -> Result<(), String> {
        if self.audio.is_some() {
            return Err("audio is already running".to_string());
        }
        self.audio = Some(audio::AudioOutput::open(fill)?);
        Ok(())
    }

    fn recording_devices(&mut self) -> Vec<(RecordingDeviceId, String)> {
        if self.audio_subsystem.is_none() {
            match audio::AudioSubsystem::start() {
                Ok(running) => self.audio_subsystem = Some(running),
                Err(_) => return Vec::new(),
            }
        }
        self.audio_subsystem
            .as_ref()
            .map_or_else(Vec::new, audio::recording_devices)
    }

    fn start_recording(
        &mut self,
        device: RecordingDeviceId,
        rate: u32,
        channels: u16,
        sink: RecordingSink,
    ) -> Result<(), String> {
        // Close any earlier recording first, since a device may only be
        // opened once.
        self.recordings.remove(&device);
        let input = audio::AudioInput::open(device, rate, channels, sink)?;
        self.recordings.insert(device, input);
        Ok(())
    }

    fn stop_recording(&mut self, device: RecordingDeviceId) {
        self.recordings.remove(&device);
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

    fn create_cursor(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        hot_x: u32,
        hot_y: u32,
    ) -> Result<CursorId, String> {
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return Err(format!(
                "a cursor of {width} by {height} pixels needs {} bytes, got {}",
                width as usize * height as usize * 4,
                rgba.len()
            ));
        }
        if hot_x >= width || hot_y >= height {
            return Err(format!(
                "the hot spot ({hot_x}, {hot_y}) is outside the {width} by {height} cursor"
            ));
        }
        // SAFETY: the surface is SDL's own, sized as checked above, and
        // each row copied fits its pitch. SDL keeps what it needs of the
        // surface, so it is released here once the cursor exists.
        unsafe {
            let surface = SDL_CreateSurface(width as i32, height as i32, SDL_PIXELFORMAT_RGBA32);
            if surface.is_null() {
                return Err(sdl_error());
            }
            let pitch = (*surface).pitch as usize;
            let pixels = (*surface).pixels.cast::<u8>();
            let row = width as usize * 4;
            for (y, source) in rgba.chunks_exact(row).enumerate() {
                ptr::copy_nonoverlapping(source.as_ptr(), pixels.add(y * pitch), row);
            }
            let cursor = SDL_CreateColorCursor(surface, hot_x as i32, hot_y as i32);
            let message = if cursor.is_null() {
                sdl_error()
            } else {
                String::new()
            };
            SDL_DestroySurface(surface);
            if cursor.is_null() {
                return Err(message);
            }
            let id = self.next_image_cursor;
            self.next_image_cursor += 1;
            self.image_cursors.insert(id, cursor);
            Ok(CursorId(id))
        }
    }

    fn set_image_cursor(&mut self, cursor: CursorId) -> Result<(), String> {
        let handle = *self
            .image_cursors
            .get(&cursor.0)
            .ok_or_else(|| "that cursor no longer exists".to_string())?;
        // SAFETY: the cursor is alive until `destroy_cursor`.
        if unsafe { SDL_SetCursor(handle) } {
            Ok(())
        } else {
            Err(sdl_error())
        }
    }

    fn destroy_cursor(&mut self, cursor: CursorId) {
        if let Some(handle) = self.image_cursors.remove(&cursor.0) {
            // SAFETY: the cursor came from SDL_CreateColorCursor and is
            // freed once.
            unsafe { SDL_DestroyCursor(handle) };
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

    fn preferred_locales(&mut self) -> Vec<String> {
        let mut count = 0;
        // SAFETY: SDL returns `count` valid locale pointers, whose strings
        // live as long as the array, which is freed once after copying.
        unsafe {
            let locales = SDL_GetPreferredLocales(&mut count);
            if locales.is_null() {
                return Vec::new();
            }
            let mut names = Vec::new();
            for index in 0..count.max(0) as usize {
                let locale = *locales.add(index);
                if locale.is_null() || (*locale).language.is_null() {
                    continue;
                }
                let language = CStr::from_ptr((*locale).language).to_string_lossy();
                let country = if (*locale).country.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr((*locale).country).to_string_lossy())
                };
                // Linux can name a locale twice, through both LANGUAGE
                // and LANG.
                let name = locale_name(&language, country.as_deref());
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            SDL_free(locales.cast());
            names
        }
    }

    fn vibrate(&mut self, seconds: f32) -> bool {
        // SDL offers a phone's own vibration motor as a haptic device
        // only on Android. Elsewhere its haptic devices are joysticks,
        // which have `rumble_gamepad`.
        if !cfg!(target_os = "android") || !seconds.is_finite() || seconds <= 0.0 {
            return false;
        }
        // SAFETY: plain SDL calls on the main thread. The haptic device is
        // opened once, kept until `drop`, and the ID list is freed once.
        unsafe {
            if !self.vibrator_tried {
                self.vibrator_tried = true;
                if SDL_InitSubSystem(SDL_INIT_HAPTIC) {
                    let mut count = 0;
                    let ids = SDL_GetHaptics(&mut count);
                    if !ids.is_null() {
                        if count > 0 {
                            let haptic = SDL_OpenHaptic(*ids);
                            if !haptic.is_null() && SDL_InitHapticRumble(haptic) {
                                self.vibrator = haptic;
                            } else if !haptic.is_null() {
                                SDL_CloseHaptic(haptic);
                            }
                        }
                        SDL_free(ids.cast());
                    }
                }
            }
            if self.vibrator.is_null() {
                return false;
            }
            let milliseconds = (seconds * 1000.0).round().min(u32::MAX as f32) as u32;
            SDL_PlayHapticRumble(self.vibrator, 1.0, milliseconds)
        }
    }

    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<TextureId, String> {
        self.renderer.create_texture(width, height, rgba, false)
    }

    fn create_mipmapped_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<TextureId, String> {
        self.renderer.create_texture(width, height, rgba, true)
    }

    fn set_texture_mipmap_filter(
        &mut self,
        texture: TextureId,
        filter: Option<Filter>,
        sharpness: f32,
    ) {
        self.renderer
            .set_texture_mipmap_filter(texture, filter, sharpness);
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

    fn create_layered_texture(
        &mut self,
        kind: TextureKind,
        width: u32,
        height: u32,
        layers: &[&[u8]],
    ) -> Result<TextureId, String> {
        self.renderer
            .create_layered_texture(kind, width, height, layers)
    }

    fn supports_compressed_format(&self, format: CompressedFormat) -> bool {
        self.renderer.supports_compressed_format(format)
    }

    fn create_compressed_texture(
        &mut self,
        format: CompressedFormat,
        width: u32,
        height: u32,
        levels: &[&[u8]],
    ) -> Result<TextureId, String> {
        self.renderer
            .create_compressed_texture(format, width, height, levels)
    }

    fn create_render_target(&mut self, width: u32, height: u32) -> Result<TextureId, String> {
        self.renderer.create_render_target(width, height, None)
    }

    fn create_mipmapped_render_target(
        &mut self,
        width: u32,
        height: u32,
        automatic: bool,
    ) -> Result<TextureId, String> {
        self.renderer
            .create_render_target(width, height, Some(automatic))
    }

    fn generate_mipmaps(&mut self, texture: TextureId) -> Result<(), String> {
        self.renderer.generate_mipmaps(texture)
    }

    fn set_texture_wrap(&mut self, texture: TextureId, horizontal: Wrap, vertical: Wrap) {
        self.renderer
            .set_texture_wrap(texture, horizontal, vertical);
    }

    fn destroy_texture(&mut self, texture: TextureId) {
        self.renderer.destroy_texture(texture);
    }

    fn texture_memory(&self) -> u64 {
        self.renderer.texture_memory()
    }

    fn create_shader(&mut self, source: &str) -> Result<ShaderInfo, String> {
        self.renderer.create_shader(source)
    }

    fn destroy_shader(&mut self, shader: ShaderId) {
        self.renderer.destroy_shader(shader);
    }

    fn validate_shader(&self, source: &str) -> Result<(), String> {
        shader::compile(source).map(|_| ())
    }

    fn renderer_info(&self) -> RendererInfo {
        self.renderer.renderer_info()
    }

    fn present(&mut self, frame: &Frame) {
        self.renderer.present(self.window, frame);
    }

    fn render_offscreen(&mut self, frame: &Frame) {
        self.renderer.render_offscreen(frame);
    }

    fn read_texture(&mut self, texture: TextureId) -> Result<(u32, u32, Vec<u8>), String> {
        self.renderer.read_texture(texture)
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

    fn title(&self) -> String {
        // SAFETY: the window is alive, and the string SDL returns is
        // copied before anything else can change it.
        unsafe {
            let title = SDL_GetWindowTitle(self.window);
            if title.is_null() {
                String::new()
            } else {
                CStr::from_ptr(title).to_string_lossy().into_owned()
            }
        }
    }

    fn set_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        let scale = self.content_scale();
        // SAFETY: the window is alive.
        unsafe {
            if !SDL_SetWindowSize(
                self.window,
                (width as f32 * scale).round().max(1.0) as i32,
                (height as f32 * scale).round().max(1.0) as i32,
            ) {
                return Err(sdl_error());
            }
            // Wait for the window system, so the new size can be read
            // straight away.
            SDL_SyncWindow(self.window);
        }
        Ok(())
    }

    fn window_position(&self) -> (i32, i32) {
        let (mut x, mut y) = (0, 0);
        // SAFETY: the window is alive and both out-pointers are valid.
        unsafe { SDL_GetWindowPosition(self.window, &mut x, &mut y) };
        (x, y)
    }

    fn set_window_position(&mut self, x: i32, y: i32) {
        // SAFETY: the window is alive.
        unsafe {
            SDL_SetWindowPosition(self.window, x, y);
            SDL_SyncWindow(self.window);
        }
    }

    fn minimize(&mut self) {
        // SAFETY: the window is alive.
        unsafe { SDL_MinimizeWindow(self.window) };
    }

    fn maximize(&mut self) {
        // SAFETY: the window is alive.
        unsafe {
            SDL_MaximizeWindow(self.window);
            SDL_SyncWindow(self.window);
        }
    }

    fn restore(&mut self) {
        // SAFETY: the window is alive.
        unsafe {
            SDL_RestoreWindow(self.window);
            SDL_SyncWindow(self.window);
        }
    }

    fn window_state(&self) -> WindowState {
        // SAFETY: the window is alive.
        let flags = unsafe { SDL_GetWindowFlags(self.window) };
        let minimized = flags & SDL_WINDOW_MINIMIZED != 0;
        WindowState {
            minimized,
            maximized: flags & SDL_WINDOW_MAXIMIZED != 0,
            focused: flags & SDL_WINDOW_INPUT_FOCUS != 0,
            mouse_focused: flags & SDL_WINDOW_MOUSE_FOCUS != 0,
            visible: !minimized && flags & SDL_WINDOW_HIDDEN == 0,
        }
    }

    fn set_vsync(&mut self, vsync: bool) -> Result<(), String> {
        let mode = if vsync {
            SDL_GPU_PRESENTMODE_VSYNC
        } else {
            SDL_GPU_PRESENTMODE_IMMEDIATE
        };
        // SAFETY: the device and the window it has claimed are alive.
        unsafe {
            if !SDL_WindowSupportsGPUPresentMode(self.device, self.window, mode) {
                return Err("this display cannot present that way".to_string());
            }
            if !SDL_SetGPUSwapchainParameters(
                self.device,
                self.window,
                SDL_GPU_SWAPCHAINCOMPOSITION_SDR,
                mode,
            ) {
                return Err(sdl_error());
            }
        }
        self.vsync = vsync;
        Ok(())
    }

    fn vsync(&self) -> bool {
        self.vsync
    }

    fn desktop_size(&self) -> (u32, u32) {
        let scale = self.content_scale();
        // SAFETY: the window is alive, and the mode SDL returns is read
        // before anything else can change it.
        unsafe {
            let mode = SDL_GetDesktopDisplayMode(SDL_GetDisplayForWindow(self.window));
            if mode.is_null() {
                return (0, 0);
            }
            (
                ((*mode).w.max(0) as f32 / scale).round() as u32,
                ((*mode).h.max(0) as f32 / scale).round() as u32,
            )
        }
    }

    fn display_count(&self) -> u32 {
        let mut count = 0;
        // SAFETY: the out-pointer is valid, and the list is freed once.
        unsafe {
            let displays = SDL_GetDisplays(&mut count);
            SDL_free(displays.cast());
        }
        count.max(0) as u32
    }

    fn show_message_box(
        &mut self,
        kind: MessageBoxKind,
        title: &str,
        message: &str,
    ) -> Result<(), String> {
        let flags = match kind {
            MessageBoxKind::Info => SDL_MESSAGEBOX_INFORMATION,
            MessageBoxKind::Warning => SDL_MESSAGEBOX_WARNING,
            MessageBoxKind::Error => SDL_MESSAGEBOX_ERROR,
        };
        let (title, message) = (c_string(title), c_string(message));
        // SAFETY: the window is alive and both strings outlive the call.
        let shown = unsafe {
            SDL_ShowSimpleMessageBox(flags, title.as_ptr(), message.as_ptr(), self.window)
        };
        if shown { Ok(()) } else { Err(sdl_error()) }
    }

    fn show_message_box_with_buttons(
        &mut self,
        kind: MessageBoxKind,
        title: &str,
        message: &str,
        buttons: &[&str],
    ) -> Result<Option<usize>, String> {
        let flags = match kind {
            MessageBoxKind::Info => SDL_MESSAGEBOX_INFORMATION,
            MessageBoxKind::Warning => SDL_MESSAGEBOX_WARNING,
            MessageBoxKind::Error => SDL_MESSAGEBOX_ERROR,
        };
        let (title, message) = (c_string(title), c_string(message));
        let labels: Vec<CString> = buttons.iter().map(|label| c_string(label)).collect();
        let last = labels.len().saturating_sub(1);
        let buttons: Vec<SDL_MessageBoxButtonData> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let mut button_flags = SDL_MessageBoxButtonFlags(0);
                if index == 0 {
                    button_flags |= SDL_MESSAGEBOX_BUTTON_RETURNKEY_DEFAULT;
                }
                if index == last {
                    button_flags |= SDL_MESSAGEBOX_BUTTON_ESCAPEKEY_DEFAULT;
                }
                SDL_MessageBoxButtonData {
                    flags: button_flags,
                    buttonID: index as i32,
                    text: label.as_ptr(),
                }
            })
            .collect();
        let data = SDL_MessageBoxData {
            flags: flags | SDL_MESSAGEBOX_BUTTONS_LEFT_TO_RIGHT,
            window: self.window,
            title: title.as_ptr(),
            message: message.as_ptr(),
            numbuttons: buttons.len() as i32,
            buttons: buttons.as_ptr(),
            colorScheme: ptr::null(),
        };
        let mut chosen = -1;
        // SAFETY: the window is alive, and every string and the button
        // array outlive the call.
        if unsafe { SDL_ShowMessageBox(&data, &mut chosen) } {
            Ok(usize::try_from(chosen).ok())
        } else {
            Err(sdl_error())
        }
    }

    fn request_attention(&mut self, continuous: bool) {
        let operation = if continuous {
            SDL_FLASH_UNTIL_FOCUSED
        } else {
            SDL_FLASH_BRIEFLY
        };
        // SAFETY: the window is alive.
        unsafe { SDL_FlashWindow(self.window, operation) };
    }

    fn layout_key(&self, key: Key) -> Option<Key> {
        // SAFETY: a plain lookup in SDL's current keymap.
        let keycode =
            unsafe { SDL_GetKeyFromScancode(scancode_from_key(key), SDL_KMOD_NONE, false) };
        key_from_keycode(keycode)
    }

    fn layout_position(&self, key: Key) -> Option<Key> {
        // SAFETY: a plain lookup in SDL's current keymap. A null modifier
        // pointer is allowed and means the result's modifiers are unwanted.
        let scancode = unsafe { SDL_GetScancodeFromKey(keycode_from_key(key), ptr::null_mut()) };
        key_from_scancode(scancode)
    }

    fn set_text_input(&mut self, enabled: bool) -> Result<(), String> {
        // SAFETY: the window is alive.
        let done = unsafe {
            if enabled {
                SDL_StartTextInput(self.window)
            } else {
                SDL_StopTextInput(self.window)
            }
        };
        if done { Ok(()) } else { Err(sdl_error()) }
    }

    fn set_text_input_area(&mut self, area: Option<[f32; 4]>) -> Result<(), String> {
        // SDL wants window coordinates; Smitten works in pixels.
        let scale = self.pixel_density();
        let rect = area.map(|[x, y, width, height]| SDL_Rect {
            x: (x / scale).round() as i32,
            y: (y / scale).round() as i32,
            w: (width / scale).round() as i32,
            h: (height / scale).round() as i32,
        });
        let rect_ptr = rect
            .as_ref()
            .map_or(ptr::null(), |rect| rect as *const SDL_Rect);
        // SAFETY: the window is alive and the rectangle, if any, outlives
        // the call. SDL copies it.
        if unsafe { SDL_SetTextInputArea(self.window, rect_ptr, 0) } {
            Ok(())
        } else {
            Err(sdl_error())
        }
    }

    fn text_input_active(&self) -> bool {
        // SAFETY: the window is alive.
        unsafe { SDL_TextInputActive(self.window) }
    }

    fn has_screen_keyboard(&self) -> bool {
        // SAFETY: a plain query.
        unsafe { SDL_HasScreenKeyboardSupport() }
    }

    fn cursor_supported(&self) -> bool {
        // Phones and tablets have no cursor to change.
        !cfg!(any(target_os = "android", target_os = "ios"))
    }

    fn is_gamepad(&self, id: JoystickId) -> bool {
        // SAFETY: a plain query; an unknown id gives false.
        self.gamepads.contains_key(&id.0) || unsafe { SDL_IsGamepad(SDL_JoystickID(id.0)) }
    }

    fn add_gamepad_mappings(&mut self, mappings: &str) -> Result<usize, String> {
        // SAFETY: SDL_GetPlatform returns a constant string.
        let platform = unsafe { CStr::from_ptr(SDL_GetPlatform()) }.to_string_lossy();
        let mut added = 0;
        let mut first_error = None;
        for line in mappings.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // SDL's own file loader skips lines without a platform, which
            // would drop a single mapping written by hand, so lines are
            // added one at a time and only other platforms are skipped.
            let for_platform = line
                .split(',')
                .find_map(|field| field.trim().strip_prefix("platform:"));
            if for_platform.is_some_and(|other| !other.eq_ignore_ascii_case(&platform)) {
                continue;
            }
            let line = c_string(line);
            // SAFETY: the string outlives the call, and SDL copies it.
            if unsafe { SDL_AddGamepadMapping(line.as_ptr()) } >= 0 {
                added += 1;
            } else if first_error.is_none() {
                first_error = Some(sdl_error());
            }
        }
        match first_error {
            Some(message) if added == 0 => Err(message),
            _ => Ok(added),
        }
    }

    fn gamepad_mapping(&self, guid: &str) -> Option<String> {
        let guid = c_string(guid);
        // SAFETY: the GUID string outlives the call, and the mapping SDL
        // returns is copied and then freed once.
        unsafe {
            let mapping = SDL_GetGamepadMappingForGUID(SDL_StringToGUID(guid.as_ptr()));
            if mapping.is_null() {
                return None;
            }
            let text = CStr::from_ptr(mapping).to_string_lossy().into_owned();
            SDL_free(mapping.cast());
            Some(text)
        }
    }

    fn gamepad_can_rumble(&self, id: JoystickId) -> bool {
        let Some(&gamepad) = self.gamepads.get(&id.0) else {
            return false;
        };
        // SAFETY: the gamepad is open, and the property name is a
        // constant C string.
        unsafe {
            SDL_GetBooleanProperty(
                SDL_GetGamepadProperties(gamepad),
                SDL_PROP_GAMEPAD_CAP_RUMBLE_BOOLEAN,
                false,
            )
        }
    }

    fn rumble_gamepad(&mut self, id: JoystickId, low: f32, high: f32, seconds: f32) -> bool {
        let Some(&gamepad) = self.gamepads.get(&id.0) else {
            return false;
        };
        let strength = |value: f32| (value.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16;
        // Float to integer conversion saturates, so a very long or
        // infinite time becomes the longest SDL can express.
        let milliseconds = (seconds.max(0.0) * 1000.0) as u32;
        // SAFETY: the gamepad is open.
        unsafe { SDL_RumbleGamepad(gamepad, strength(low), strength(high), milliseconds) }
    }

    fn set_title(&mut self, title: &str) {
        let title = c_string(title);
        // SAFETY: the window is alive and SDL copies the string.
        unsafe { SDL_SetWindowTitle(self.window, title.as_ptr()) };
    }

    fn set_fullscreen(&mut self, fullscreen: bool, kind: FullscreenType) -> Result<(), String> {
        // The fullscreen mode is chosen even when leaving fullscreen, so
        // that the window remembers which kind to use next time. With no
        // mode chosen, SDL uses the desktop's own resolution, without
        // changing video mode.
        // SAFETY: the window is alive, and the mode passed is read by SDL
        // during the call.
        unsafe {
            match kind {
                FullscreenType::Desktop => {
                    if !SDL_SetWindowFullscreenMode(self.window, ptr::null()) {
                        return Err(sdl_error());
                    }
                }
                FullscreenType::Exclusive => {
                    let (mut width, mut height) = (0, 0);
                    SDL_GetWindowSize(self.window, &mut width, &mut height);
                    let mut mode: SDL_DisplayMode = std::mem::zeroed();
                    if !SDL_GetClosestFullscreenDisplayMode(
                        SDL_GetDisplayForWindow(self.window),
                        width,
                        height,
                        0.0,
                        true,
                        &mut mode,
                    ) || !SDL_SetWindowFullscreenMode(self.window, &mode)
                    {
                        return Err(sdl_error());
                    }
                }
            }
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

    fn fullscreen_type(&self) -> FullscreenType {
        // SAFETY: the window is alive.
        if unsafe { SDL_GetWindowFullscreenMode(self.window) }.is_null() {
            FullscreenType::Desktop
        } else {
            FullscreenType::Exclusive
        }
    }

    fn fullscreen_modes(&self, display: u32) -> Vec<(u32, u32)> {
        let Some(id) = display_id(display) else {
            return Vec::new();
        };
        let scale = self.display_content_scale(id);
        let mut sizes: Vec<(u32, u32)> = Vec::new();
        let mut count = 0;
        // SAFETY: SDL returns `count` valid mode pointers in one
        // allocation, which is read here and then freed once.
        unsafe {
            let modes = SDL_GetFullscreenDisplayModes(id, &mut count);
            if modes.is_null() {
                return Vec::new();
            }
            for index in 0..count.max(0) as usize {
                let mode = &**modes.add(index);
                let size = (
                    (mode.w.max(0) as f32 / scale).round() as u32,
                    (mode.h.max(0) as f32 / scale).round() as u32,
                );
                // SDL lists a size once for each refresh rate and density.
                if !sizes.contains(&size) {
                    sizes.push(size);
                }
            }
            SDL_free(modes.cast());
        }
        sizes
    }

    fn set_borderless(&mut self, borderless: bool) {
        // SAFETY: the window is alive.
        unsafe {
            SDL_SetWindowBordered(self.window, !borderless);
            SDL_SyncWindow(self.window);
        }
    }

    fn is_borderless(&self) -> bool {
        // SAFETY: the window is alive.
        let flags = unsafe { SDL_GetWindowFlags(self.window) };
        flags & SDL_WINDOW_BORDERLESS != 0
    }

    fn set_resizable(&mut self, resizable: bool) {
        // SAFETY: the window is alive.
        unsafe {
            SDL_SetWindowResizable(self.window, resizable);
            SDL_SyncWindow(self.window);
        }
    }

    fn is_resizable(&self) -> bool {
        // SAFETY: the window is alive.
        let flags = unsafe { SDL_GetWindowFlags(self.window) };
        flags & SDL_WINDOW_RESIZABLE != 0
    }

    fn set_minimum_size(&mut self, width: u32, height: u32) {
        let scale = self.content_scale();
        // SAFETY: the window is alive.
        unsafe {
            SDL_SetWindowMinimumSize(
                self.window,
                (width as f32 * scale).round() as i32,
                (height as f32 * scale).round() as i32,
            );
        }
    }

    fn minimum_size(&self) -> (u32, u32) {
        let scale = self.content_scale();
        let (mut width, mut height) = (0, 0);
        // SAFETY: the window is alive and both out-pointers are valid.
        unsafe { SDL_GetWindowMinimumSize(self.window, &mut width, &mut height) };
        (
            (width.max(0) as f32 / scale).round() as u32,
            (height.max(0) as f32 / scale).round() as u32,
        )
    }

    fn window_display(&self) -> u32 {
        // SAFETY: the window is alive.
        let id = unsafe { SDL_GetDisplayForWindow(self.window) };
        display_ids()
            .iter()
            .position(|&each| each == id)
            .unwrap_or(0) as u32
    }

    fn display_name(&self, display: u32) -> Option<String> {
        let id = display_id(display)?;
        // SAFETY: the string SDL returns is copied straight away.
        unsafe {
            let name = SDL_GetDisplayName(id);
            if name.is_null() {
                None
            } else {
                Some(CStr::from_ptr(name).to_string_lossy().into_owned())
            }
        }
    }

    fn display_bounds(&self, display: u32) -> Option<(i32, i32, u32, u32)> {
        let id = display_id(display)?;
        let mut rect = SDL_Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
        // SAFETY: the out-pointer is valid.
        if !unsafe { SDL_GetDisplayBounds(id, &mut rect) } {
            return None;
        }
        Some((rect.x, rect.y, rect.w.max(0) as u32, rect.h.max(0) as u32))
    }

    fn display_desktop_size(&self, display: u32) -> Option<(u32, u32)> {
        let id = display_id(display)?;
        let scale = self.display_content_scale(id);
        // SAFETY: the mode SDL returns is read before anything else can
        // change it.
        unsafe {
            let mode = SDL_GetDesktopDisplayMode(id);
            if mode.is_null() {
                return None;
            }
            Some((
                ((*mode).w.max(0) as f32 / scale).round() as u32,
                ((*mode).h.max(0) as f32 / scale).round() as u32,
            ))
        }
    }

    fn display_dpi_scale(&self, display: u32) -> Option<f32> {
        let id = display_id(display)?;
        // SAFETY: the mode SDL returns is read before anything else can
        // change it.
        let density = unsafe {
            let mode = SDL_GetDesktopDisplayMode(id);
            if mode.is_null() {
                return None;
            }
            (*mode).pixel_density
        };
        // SAFETY: a plain query.
        let content = unsafe { SDL_GetDisplayContentScale(id) };
        let scale = density.max(0.0) * content.max(0.0);
        Some(if scale > 0.0 { scale } else { 1.0 })
    }

    fn center_window(&mut self, display: u32) {
        let id = display_id(display).unwrap_or_else(|| {
            // SAFETY: a plain query.
            unsafe { SDL_GetPrimaryDisplay() }
        });
        let centered = SDL_WINDOWPOS_CENTERED_DISPLAY(id);
        // SAFETY: the window is alive.
        unsafe {
            SDL_SetWindowPosition(self.window, centered, centered);
            SDL_SyncWindow(self.window);
        }
    }

    fn set_window_shown(&mut self, shown: bool) {
        // SAFETY: the window is alive.
        unsafe {
            if shown {
                SDL_ShowWindow(self.window);
            } else {
                SDL_HideWindow(self.window);
            }
            SDL_SyncWindow(self.window);
        }
    }

    fn set_display_sleep(&mut self, enabled: bool) {
        // SAFETY: plain SDL calls; video is initialized.
        unsafe {
            if enabled {
                SDL_EnableScreenSaver();
            } else {
                SDL_DisableScreenSaver();
            }
        }
    }

    fn display_sleep_enabled(&self) -> bool {
        // SAFETY: a plain query.
        unsafe { SDL_ScreenSaverEnabled() }
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

    fn safe_area(&self) -> (i32, i32, u32, u32) {
        let mut rect = SDL_Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
        // SAFETY: the window is alive and the out-pointer is valid.
        if !unsafe { SDL_GetWindowSafeArea(self.window, &mut rect) } {
            let (width, height) = self.size_in_pixels();
            return (0, 0, width, height);
        }
        // SDL reports window coordinates; Smitten works in pixels.
        let scale = self.pixel_density();
        let pixels = |value: i32| (value as f32 * scale).round();
        (
            pixels(rect.x) as i32,
            pixels(rect.y) as i32,
            pixels(rect.w).max(0.0) as u32,
            pixels(rect.h).max(0.0) as u32,
        )
    }

    fn display_orientation(&self) -> DisplayOrientation {
        // SAFETY: the window is alive.
        let orientation =
            unsafe { SDL_GetCurrentDisplayOrientation(SDL_GetDisplayForWindow(self.window)) };
        display_orientation(orientation)
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
            } else if kind == SDL_EVENT_LOW_MEMORY {
                Some(BackendEvent::LowMemory)
            } else if kind == SDL_EVENT_DISPLAY_ORIENTATION {
                // Only the display the game is on matters to it.
                if event.display.displayID != SDL_GetDisplayForWindow(self.window) {
                    return None;
                }
                let orientation = SDL_DisplayOrientation(event.display.data1);
                Some(BackendEvent::DisplayRotated(display_orientation(
                    orientation,
                )))
            } else if kind == SDL_EVENT_WINDOW_FOCUS_GAINED {
                Some(BackendEvent::Focus(true))
            } else if kind == SDL_EVENT_WINDOW_FOCUS_LOST {
                Some(BackendEvent::Focus(false))
            } else if kind == SDL_EVENT_WINDOW_MOUSE_ENTER {
                Some(BackendEvent::MouseFocus(true))
            } else if kind == SDL_EVENT_WINDOW_MOUSE_LEAVE {
                Some(BackendEvent::MouseFocus(false))
            } else if kind == SDL_EVENT_WINDOW_MINIMIZED
                || kind == SDL_EVENT_WINDOW_HIDDEN
                || kind == SDL_EVENT_WINDOW_RESTORED
                || kind == SDL_EVENT_WINDOW_SHOWN
            {
                // Restoring also follows a maximize, so only a real change
                // is passed on.
                let visible = kind == SDL_EVENT_WINDOW_RESTORED || kind == SDL_EVENT_WINDOW_SHOWN;
                if visible == self.visible {
                    return None;
                }
                self.visible = visible;
                Some(BackendEvent::Visible(visible))
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
            } else if kind == SDL_EVENT_TEXT_EDITING {
                if event.edit.text.is_null() {
                    return None;
                }
                let text = CStr::from_ptr(event.edit.text).to_string_lossy();
                Some(BackendEvent::TextEdited {
                    text: text.into_owned(),
                    start: event.edit.start.max(0) as usize,
                    length: event.edit.length.max(0) as usize,
                })
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
            } else if kind == SDL_EVENT_FINGER_DOWN
                || kind == SDL_EVENT_FINGER_MOTION
                || kind == SDL_EVENT_FINGER_UP
                || kind == SDL_EVENT_FINGER_CANCELED
            {
                let finger = &event.tfinger;
                // SDL reports fractions of the window; Smitten works in
                // pixels.
                let (width, height) = self.size_in_pixels();
                let (width, height) = (width as f32, height as f32);
                let id = TouchId(finger.fingerID.0);
                let (x, y) = (finger.x * width, finger.y * height);
                let (dx, dy) = (finger.dx * width, finger.dy * height);
                let pressure = finger.pressure;
                Some(if kind == SDL_EVENT_FINGER_DOWN {
                    BackendEvent::TouchPressed {
                        id,
                        x,
                        y,
                        dx,
                        dy,
                        pressure,
                    }
                } else if kind == SDL_EVENT_FINGER_MOTION {
                    BackendEvent::TouchMoved {
                        id,
                        x,
                        y,
                        dx,
                        dy,
                        pressure,
                    }
                } else {
                    // A canceled touch is over just as a lifted one is.
                    BackendEvent::TouchReleased {
                        id,
                        x,
                        y,
                        dx,
                        dy,
                        pressure,
                    }
                })
            } else if kind == SDL_EVENT_JOYSTICK_ADDED {
                let which = event.jdevice.which;
                if self.joysticks.contains_key(&which.0) {
                    return None;
                }
                let joystick = SDL_OpenJoystick(which);
                if joystick.is_null() {
                    return None;
                }
                self.joysticks.insert(which.0, joystick);
                Some(BackendEvent::JoystickAdded {
                    id: JoystickId(which.0),
                    info: joystick_info(joystick),
                })
            } else if kind == SDL_EVENT_JOYSTICK_REMOVED {
                let which = event.jdevice.which;
                if let Some(gamepad) = self.gamepads.remove(&which.0) {
                    SDL_CloseGamepad(gamepad);
                }
                let joystick = self.joysticks.remove(&which.0)?;
                SDL_CloseJoystick(joystick);
                Some(BackendEvent::JoystickRemoved {
                    id: JoystickId(which.0),
                })
            } else if kind == SDL_EVENT_GAMEPAD_ADDED {
                // This follows the joystick's own event, and also comes
                // when a new mapping turns a joystick into a gamepad. The
                // gamepad must be open for SDL to report its buttons.
                let which = event.gdevice.which;
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.gamepads.entry(which.0)
                {
                    let gamepad = SDL_OpenGamepad(which);
                    if !gamepad.is_null() {
                        entry.insert(gamepad);
                    }
                }
                None
            } else if kind == SDL_EVENT_GAMEPAD_REMOVED {
                if let Some(gamepad) = self.gamepads.remove(&event.gdevice.which.0) {
                    SDL_CloseGamepad(gamepad);
                }
                None
            } else if kind == SDL_EVENT_JOYSTICK_BUTTON_DOWN || kind == SDL_EVENT_JOYSTICK_BUTTON_UP
            {
                let id = JoystickId(event.jbutton.which.0);
                let button = usize::from(event.jbutton.button);
                Some(if event.jbutton.down {
                    BackendEvent::JoystickPressed { id, button }
                } else {
                    BackendEvent::JoystickReleased { id, button }
                })
            } else if kind == SDL_EVENT_JOYSTICK_AXIS_MOTION {
                Some(BackendEvent::JoystickAxis {
                    id: JoystickId(event.jaxis.which.0),
                    axis: usize::from(event.jaxis.axis),
                    value: (f32::from(event.jaxis.value) / 32767.0).clamp(-1.0, 1.0),
                })
            } else if kind == SDL_EVENT_JOYSTICK_HAT_MOTION {
                Some(BackendEvent::JoystickHat {
                    id: JoystickId(event.jhat.which.0),
                    hat: usize::from(event.jhat.hat),
                    direction: JoystickHat::from_bits(event.jhat.value),
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
            } else if kind == SDL_EVENT_DROP_FILE {
                if event.drop.data.is_null() {
                    return None;
                }
                let path =
                    PathBuf::from(CStr::from_ptr(event.drop.data).to_string_lossy().as_ref());
                // SDL reports window coordinates; Smitten works in pixels.
                let scale = self.pixel_density();
                let (x, y) = (event.drop.x * scale, event.drop.y * scale);
                // SDL drops directories as files, so tell them apart here.
                Some(if path.is_dir() {
                    BackendEvent::DirectoryDropped { path, x, y }
                } else {
                    BackendEvent::FileDropped { path, x, y }
                })
            } else {
                None
            }
        }
    }

    /// Pixels per window coordinate, above 1.0 on high-DPI displays.
    /// How many window coordinates make up one unit on the window's
    /// display. See the note on the window's size in `new`.
    fn content_scale(&self) -> f32 {
        if !self.content_scaling {
            return 1.0;
        }
        // SAFETY: the window is alive.
        let scale = unsafe { SDL_GetDisplayContentScale(SDL_GetDisplayForWindow(self.window)) };
        if scale > 0.0 { scale } else { 1.0 }
    }

    /// How many window coordinates make up one unit on `display`, as
    /// `content_scale` is for the window's own display.
    fn display_content_scale(&self, display: SDL_DisplayID) -> f32 {
        if !self.content_scaling {
            return 1.0;
        }
        // SAFETY: a plain query.
        let scale = unsafe { SDL_GetDisplayContentScale(display) };
        if scale > 0.0 { scale } else { 1.0 }
    }

    fn pixel_density(&self) -> f32 {
        // SAFETY: the window is alive.
        let density = unsafe { SDL_GetWindowPixelDensity(self.window) };
        if density > 0.0 { density } else { 1.0 }
    }
}

/// The connected displays, in the order the system lists them.
fn display_ids() -> Vec<SDL_DisplayID> {
    let mut count = 0;
    // SAFETY: SDL returns `count` ids in one allocation, which is copied
    // and then freed once.
    unsafe {
        let displays = SDL_GetDisplays(&mut count);
        if displays.is_null() {
            return Vec::new();
        }
        let ids = std::slice::from_raw_parts(displays, count.max(0) as usize).to_vec();
        SDL_free(displays.cast());
        ids
    }
}

/// The SDL id of the display at `index` in [`display_ids`].
fn display_id(index: u32) -> Option<SDL_DisplayID> {
    display_ids().get(index as usize).copied()
}

fn display_orientation(orientation: SDL_DisplayOrientation) -> DisplayOrientation {
    match orientation {
        SDL_ORIENTATION_LANDSCAPE => DisplayOrientation::Landscape,
        SDL_ORIENTATION_LANDSCAPE_FLIPPED => DisplayOrientation::LandscapeFlipped,
        SDL_ORIENTATION_PORTRAIT => DisplayOrientation::Portrait,
        SDL_ORIENTATION_PORTRAIT_FLIPPED => DisplayOrientation::PortraitFlipped,
        _ => DisplayOrientation::Unknown,
    }
}

/// Joins a language and country code as `"en_US"`, or gives the
/// language alone.
fn locale_name(language: &str, country: Option<&str>) -> String {
    match country {
        Some(country) if !country.is_empty() => format!("{language}_{country}"),
        _ => language.to_string(),
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

/// Reads what SDL knows about an open joystick.
///
/// # Safety
/// `joystick` must be open.
unsafe fn joystick_info(joystick: *mut SDL_Joystick) -> JoystickInfo {
    // SAFETY: the joystick is open, the name SDL returns is copied before
    // anything else can change it, and the GUID buffer is large enough for
    // 32 hex digits and a NUL as SDL documents.
    unsafe {
        let name = SDL_GetJoystickName(joystick);
        let name = if name.is_null() {
            "Joystick".to_string()
        } else {
            CStr::from_ptr(name).to_string_lossy().into_owned()
        };
        let mut guid = [0 as std::ffi::c_char; 33];
        SDL_GUIDToString(
            SDL_GetJoystickGUID(joystick),
            guid.as_mut_ptr(),
            guid.len() as i32,
        );
        let guid = CStr::from_ptr(guid.as_ptr()).to_string_lossy().into_owned();
        let count = |n: i32| usize::try_from(n).unwrap_or(0);
        JoystickInfo {
            name,
            guid,
            vendor_id: SDL_GetJoystickVendor(joystick),
            product_id: SDL_GetJoystickProduct(joystick),
            product_version: SDL_GetJoystickProductVersion(joystick),
            axis_count: count(SDL_GetNumJoystickAxes(joystick)),
            button_count: count(SDL_GetNumJoystickButtons(joystick)),
            hat_count: count(SDL_GetNumJoystickHats(joystick)),
        }
    }
}

/// Every key Smitten names, with its SDL scancode.
fn all_keys() -> impl Iterator<Item = (SDL_Scancode, Key)> {
    (0..SDL_SCANCODE_COUNT.0)
        .map(SDL_Scancode)
        .filter_map(|scancode| Some((scancode, key_from_scancode(scancode)?)))
}

fn scancode_from_key(key: Key) -> SDL_Scancode {
    static SCANCODES: std::sync::OnceLock<HashMap<Key, SDL_Scancode>> = std::sync::OnceLock::new();
    SCANCODES
        .get_or_init(|| all_keys().map(|(scancode, key)| (key, scancode)).collect())
        .get(&key)
        .copied()
        .unwrap_or(SDL_SCANCODE_UNKNOWN)
}

/// The keycode a US layout gives the key. Keys that type a character use
/// that character, lowercase; the rest use their scancode with SDL's
/// scancode bit set.
fn keycode_from_key(key: Key) -> SDL_Keycode {
    use Key::*;
    let character = match key {
        A | B | C | D | E | F | G | H | I | J | K | L | M | N | O | P | Q | R | S | T | U | V
        | W | X | Y | Z => Some(b'a' + (scancode_from_key(key).0 - SDL_SCANCODE_A.0) as u8),
        Num0 => Some(b'0'),
        Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9 => {
            Some(b'1' + (scancode_from_key(key).0 - SDL_SCANCODE_1.0) as u8)
        }
        Space => Some(b' '),
        Return => Some(b'\r'),
        Escape => Some(0x1b),
        Tab => Some(b'\t'),
        Backspace => Some(0x08),
        Delete => Some(0x7f),
        Minus => Some(b'-'),
        Equals => Some(b'='),
        LeftBracket => Some(b'['),
        RightBracket => Some(b']'),
        Backslash => Some(b'\\'),
        Semicolon => Some(b';'),
        Apostrophe => Some(b'\''),
        Grave => Some(b'`'),
        Comma => Some(b','),
        Period => Some(b'.'),
        Slash => Some(b'/'),
        Exclaim => Some(b'!'),
        DoubleQuote => Some(b'"'),
        Hash => Some(b'#'),
        Dollar => Some(b'$'),
        Ampersand => Some(b'&'),
        LeftParen => Some(b'('),
        RightParen => Some(b')'),
        Asterisk => Some(b'*'),
        Plus => Some(b'+'),
        Colon => Some(b':'),
        Less => Some(b'<'),
        Greater => Some(b'>'),
        Question => Some(b'?'),
        At => Some(b'@'),
        Caret => Some(b'^'),
        Underscore => Some(b'_'),
        _ => None,
    };
    match character {
        Some(character) => SDL_Keycode(u32::from(character)),
        None => SDL_Keycode(scancode_from_key(key).0 as u32 | SDLK_SCANCODE_MASK.0),
    }
}

fn key_from_keycode(keycode: SDL_Keycode) -> Option<Key> {
    if keycode.0 == 0 {
        return None;
    }
    // Keys that type no character carry their scancode.
    if keycode.0 & SDLK_SCANCODE_MASK.0 != 0 {
        let key = key_from_scancode(SDL_Scancode((keycode.0 & !SDLK_SCANCODE_MASK.0) as _))?;
        return (keycode_from_key(key) == keycode).then_some(key);
    }
    all_keys()
        .map(|(_, key)| key)
        .chain(LAYOUT_ONLY_KEYS)
        .find(|&key| keycode_from_key(key) == keycode)
}

/// Keys that some layouts have but no US key sits where they do, so they
/// have no scancode and are found only by their keycode.
const LAYOUT_ONLY_KEYS: [Key; 16] = {
    use Key::*;
    [
        Exclaim,
        DoubleQuote,
        Hash,
        Dollar,
        Ampersand,
        LeftParen,
        RightParen,
        Asterisk,
        Plus,
        Colon,
        Less,
        Greater,
        Question,
        At,
        Caret,
        Underscore,
    ]
};

fn key_from_scancode(scancode: SDL_Scancode) -> Option<Key> {
    use Key::*;

    const LETTERS: [Key; 26] = [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    ];
    // SDL orders the digit row 1 through 9 and then 0.
    const DIGITS: [Key; 10] = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0];
    const FUNCTION: [Key; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    const MORE_FUNCTION: [Key; 12] = [F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24];
    // Like the digit row, SDL orders the keypad digits 1 through 9 and then 0.
    const KEYPAD_DIGITS: [Key; 10] = [Kp1, Kp2, Kp3, Kp4, Kp5, Kp6, Kp7, Kp8, Kp9, Kp0];
    const INTERNATIONAL: [Key; 9] = [
        International1,
        International2,
        International3,
        International4,
        International5,
        International6,
        International7,
        International8,
        International9,
    ];
    const LANG: [Key; 9] = [
        Lang1, Lang2, Lang3, Lang4, Lang5, Lang6, Lang7, Lang8, Lang9,
    ];

    let code = scancode.0;
    let in_run = |first: SDL_Scancode, keys: &[Key]| {
        let offset = code - first.0;
        (offset >= 0 && (offset as usize) < keys.len()).then(|| keys[offset as usize])
    };

    if let Some(key) = in_run(SDL_SCANCODE_A, &LETTERS)
        .or_else(|| in_run(SDL_SCANCODE_1, &DIGITS))
        .or_else(|| in_run(SDL_SCANCODE_F1, &FUNCTION))
        .or_else(|| in_run(SDL_SCANCODE_F13, &MORE_FUNCTION))
        .or_else(|| in_run(SDL_SCANCODE_KP_1, &KEYPAD_DIGITS))
        .or_else(|| in_run(SDL_SCANCODE_INTERNATIONAL1, &INTERNATIONAL))
        .or_else(|| in_run(SDL_SCANCODE_LANG1, &LANG))
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
        (SDL_SCANCODE_KP_PERIOD, KpPeriod),
        (SDL_SCANCODE_KP_COMMA, KpComma),
        (SDL_SCANCODE_KP_DIVIDE, KpDivide),
        (SDL_SCANCODE_KP_MULTIPLY, KpMultiply),
        (SDL_SCANCODE_KP_MINUS, KpMinus),
        (SDL_SCANCODE_KP_PLUS, KpPlus),
        (SDL_SCANCODE_KP_ENTER, KpEnter),
        (SDL_SCANCODE_KP_EQUALS, KpEquals),
        (SDL_SCANCODE_NUMLOCKCLEAR, NumLock),
        (SDL_SCANCODE_SCROLLLOCK, ScrollLock),
        (SDL_SCANCODE_PRINTSCREEN, PrintScreen),
        (SDL_SCANCODE_SYSREQ, SysReq),
        (SDL_SCANCODE_PAUSE, Pause),
        (SDL_SCANCODE_APPLICATION, Application),
        (SDL_SCANCODE_MENU, Menu),
        (SDL_SCANCODE_CLEAR, Clear),
        (SDL_SCANCODE_HELP, Help),
        (SDL_SCANCODE_POWER, Power),
        (SDL_SCANCODE_MODE, Mode),
        (SDL_SCANCODE_EXECUTE, Execute),
        (SDL_SCANCODE_SELECT, Select),
        (SDL_SCANCODE_STOP, Stop),
        (SDL_SCANCODE_AGAIN, Again),
        (SDL_SCANCODE_UNDO, Undo),
        (SDL_SCANCODE_CUT, Cut),
        (SDL_SCANCODE_COPY, Copy),
        (SDL_SCANCODE_PASTE, Paste),
        (SDL_SCANCODE_FIND, Find),
        (SDL_SCANCODE_CANCEL, Cancel),
        (SDL_SCANCODE_MUTE, Mute),
        (SDL_SCANCODE_VOLUMEUP, VolumeUp),
        (SDL_SCANCODE_VOLUMEDOWN, VolumeDown),
        (SDL_SCANCODE_MEDIA_PLAY_PAUSE, MediaPlayPause),
        (SDL_SCANCODE_MEDIA_STOP, MediaStop),
        (SDL_SCANCODE_MEDIA_NEXT_TRACK, MediaNextTrack),
        (SDL_SCANCODE_MEDIA_PREVIOUS_TRACK, MediaPreviousTrack),
        (SDL_SCANCODE_MEDIA_SELECT, MediaSelect),
        (SDL_SCANCODE_MEDIA_EJECT, MediaEject),
        (SDL_SCANCODE_AC_SEARCH, AppSearch),
        (SDL_SCANCODE_AC_HOME, AppHome),
        (SDL_SCANCODE_AC_BACK, AppBack),
        (SDL_SCANCODE_AC_FORWARD, AppForward),
        (SDL_SCANCODE_AC_REFRESH, AppRefresh),
        (SDL_SCANCODE_AC_BOOKMARKS, AppBookmarks),
        (SDL_SCANCODE_NONUSBACKSLASH, NonUsBackslash),
        (SDL_SCANCODE_NONUSHASH, NonUsHash),
    ];
    named
        .iter()
        .find(|(sdl, _)| sdl.0 == code)
        .map(|&(_, key)| key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_one_scancode_and_one_keycode() {
        let keys: Vec<Key> = all_keys().map(|(_, key)| key).collect();
        let distinct: std::collections::HashSet<Key> = keys.iter().copied().collect();
        assert_eq!(keys.len(), distinct.len(), "two scancodes share a key");
        for (scancode, key) in all_keys() {
            assert_eq!(scancode_from_key(key).0, scancode.0, "{key:?}");
            assert_eq!(
                key_from_keycode(keycode_from_key(key)),
                Some(key),
                "{key:?}"
            );
        }
        for key in LAYOUT_ONLY_KEYS {
            assert_eq!(scancode_from_key(key).0, SDL_SCANCODE_UNKNOWN.0, "{key:?}");
            assert_eq!(
                key_from_keycode(keycode_from_key(key)),
                Some(key),
                "{key:?}"
            );
        }
    }

    #[test]
    fn keypad_keys_are_their_own_keys() {
        assert_eq!(key_from_scancode(SDL_SCANCODE_KP_0), Some(Key::Kp0));
        assert_eq!(key_from_scancode(SDL_SCANCODE_KP_1), Some(Key::Kp1));
        assert_eq!(key_from_scancode(SDL_SCANCODE_KP_9), Some(Key::Kp9));
        assert_eq!(key_from_scancode(SDL_SCANCODE_KP_ENTER), Some(Key::KpEnter));
        assert_eq!(key_from_scancode(SDL_SCANCODE_F15), Some(Key::F15));
        assert_eq!(key_from_keycode(SDLK_KP_PLUS), Some(Key::KpPlus));
        assert_eq!(key_from_keycode(SDLK_EXCLAIM), Some(Key::Exclaim));
    }

    #[test]
    fn locales_are_named_as_in_love() {
        assert_eq!(locale_name("en", Some("US")), "en_US");
        assert_eq!(locale_name("fr", None), "fr");
        assert_eq!(locale_name("de", Some("")), "de");
    }
}

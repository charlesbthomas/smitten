//! Mouse state, the equivalent of `love.mouse`.
//!
//! Positions are in the same coordinates `graphics` draws to the screen
//! in: units from the top-left corner of the window, or of the design
//! area when the game has a design size.

use std::path::Path;
use std::rc::Rc;

use smitten_core::backend::CursorId;

use crate::context;
use crate::graphics::{ImageData, ImageError};
use crate::{Cursor, MouseButton};

/// The pointer's position as `(x, y)`.
pub fn position() -> (f32, f32) {
    context::with(|ctx| ctx.input.mouse_position)
}

/// The pointer's horizontal position.
pub fn x() -> f32 {
    position().0
}

/// The pointer's vertical position.
pub fn y() -> f32 {
    position().1
}

/// Returns whether `button` is currently held.
pub fn is_down(button: MouseButton) -> bool {
    context::with(|ctx| ctx.input.mouse_buttons.contains(&button))
}

/// Shows or hides the cursor while it is over the window.
pub fn set_visible(visible: bool) {
    context::with(|ctx| {
        ctx.input.mouse_visible = visible;
        ctx.backend.set_mouse_visible(visible);
    });
}

/// Returns whether the cursor is shown.
pub fn is_visible() -> bool {
    context::with(|ctx| ctx.input.mouse_visible)
}

/// Moves the pointer to `(x, y)`, in the same coordinates as
/// [`position`]. A `mouse_moved` callback follows.
pub fn set_position(x: f32, y: f32) {
    context::with(|ctx| {
        let view = ctx.gfx.screen.view();
        ctx.backend
            .warp_mouse(view.x + x * view.scale, view.y + y * view.scale);
    });
}

/// Moves the pointer horizontally, keeping its vertical position.
pub fn set_x(x: f32) {
    set_position(x, self::y());
}

/// Moves the pointer vertically, keeping its horizontal position.
pub fn set_y(y: f32) {
    set_position(self::x(), y);
}

/// Chooses one of the operating system's cursors, the equivalent of
/// `love.mouse.setCursor(love.mouse.getSystemCursor(name))`. It replaces
/// an [`ImageCursor`].
pub fn set_cursor(cursor: Cursor) {
    let replaced = context::with(|ctx| match ctx.backend.set_cursor(cursor) {
        Ok(()) => {
            ctx.input.cursor = cursor;
            ctx.input.image_cursor.take()
        }
        // A system without that cursor keeps the one it had.
        Err(message) => {
            eprintln!("smitten: could not set the cursor: {message}");
            None
        }
    });
    // Dropped outside the framework state, which freeing it needs.
    drop(replaced);
}

/// Returns the system cursor last chosen with [`set_cursor`]. While an
/// [`ImageCursor`] is shown, [`image_cursor`] returns it.
pub fn cursor() -> Cursor {
    context::with(|ctx| ctx.input.cursor)
}

/// A cursor drawn from an image, the equivalent of the `Cursor` that
/// `love.mouse.newCursor` returns.
///
/// Make one with [`new_cursor`] and show it with [`set_image_cursor`].
/// Clones share the same cursor, which is freed when the last clone is
/// dropped and it is no longer shown.
#[derive(Clone)]
pub struct ImageCursor {
    inner: Rc<CursorInner>,
}

struct CursorInner {
    id: CursorId,
    size: (u32, u32),
    hot_spot: (u32, u32),
}

impl Drop for CursorInner {
    fn drop(&mut self) {
        let id = self.id;
        context::try_with(|ctx| ctx.backend.destroy_cursor(id));
    }
}

impl ImageCursor {
    /// Makes a cursor from pixels in memory. `(hot_x, hot_y)` is the pixel
    /// that points, counted from the image's top-left corner, and must lie
    /// inside the image.
    ///
    /// Each pixel of the image covers one pixel of the screen, whatever
    /// the display's scale.
    pub fn from_image_data(
        data: &ImageData,
        hot_x: u32,
        hot_y: u32,
    ) -> Result<ImageCursor, ImageError> {
        let (width, height) = (data.width(), data.height());
        let id = context::with(|ctx| {
            ctx.backend
                .create_cursor(width, height, data.as_bytes(), hot_x, hot_y)
        })
        .map_err(ImageError::Upload)?;
        Ok(ImageCursor {
            inner: Rc::new(CursorInner {
                id,
                size: (width, height),
                hot_spot: (hot_x, hot_y),
            }),
        })
    }

    /// The image's size in pixels, as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        self.inner.size
    }

    /// The pixel that points, as `(x, y)` from the top-left corner.
    pub fn hot_spot(&self) -> (u32, u32) {
        self.inner.hot_spot
    }
}

impl PartialEq for ImageCursor {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl std::fmt::Debug for ImageCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageCursor")
            .field("dimensions", &self.inner.size)
            .field("hot_spot", &self.inner.hot_spot)
            .finish()
    }
}

/// Loads a cursor from an image file, the equivalent of
/// `love.mouse.newCursor(path, hot_x, hot_y)`. `(hot_x, hot_y)` is the
/// pixel that points. Panics if the file cannot be loaded; see
/// [`try_new_cursor`].
pub fn new_cursor(path: impl AsRef<Path>, hot_x: u32, hot_y: u32) -> ImageCursor {
    try_new_cursor(path, hot_x, hot_y).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads a cursor from an image file, returning an error if it fails.
pub fn try_new_cursor(
    path: impl AsRef<Path>,
    hot_x: u32,
    hot_y: u32,
) -> Result<ImageCursor, ImageError> {
    let data = crate::graphics::try_new_image_data(path)?;
    ImageCursor::from_image_data(&data, hot_x, hot_y)
}

/// Shows `cursor` while the pointer is over the window, the equivalent of
/// `love.mouse.setCursor(cursor)`. [`set_cursor`] goes back to a system
/// cursor.
pub fn set_image_cursor(cursor: &ImageCursor) {
    let replaced = context::with(|ctx| match ctx.backend.set_image_cursor(cursor.inner.id) {
        Ok(()) => ctx.input.image_cursor.replace(cursor.clone()),
        Err(message) => {
            eprintln!("smitten: could not set the cursor: {message}");
            None
        }
    });
    drop(replaced);
}

/// The image cursor being shown, or `None` while a system cursor is.
pub fn image_cursor() -> Option<ImageCursor> {
    context::with(|ctx| ctx.input.image_cursor.clone())
}

/// Turns relative mode on or off. While it is on the cursor is hidden and
/// kept in the window, and `mouse_moved` keeps reporting movement however
/// far the mouse travels. Use it for a camera steered by the mouse.
pub fn set_relative_mode(relative: bool) {
    context::with(|ctx| match ctx.backend.set_relative_mouse(relative) {
        Ok(()) => ctx.input.mouse_relative = relative,
        Err(message) => eprintln!("smitten: could not change relative mouse mode: {message}"),
    });
}

/// Returns whether relative mode is on.
pub fn relative_mode() -> bool {
    context::with(|ctx| ctx.input.mouse_relative)
}

/// Keeps the pointer inside the window, or lets it leave again.
pub fn set_grabbed(grabbed: bool) {
    context::with(|ctx| match ctx.backend.set_mouse_grabbed(grabbed) {
        Ok(()) => ctx.input.mouse_grabbed = grabbed,
        Err(message) => eprintln!("smitten: could not grab the mouse: {message}"),
    });
}

/// Returns whether the pointer is kept inside the window.
pub fn is_grabbed() -> bool {
    context::with(|ctx| ctx.input.mouse_grabbed)
}

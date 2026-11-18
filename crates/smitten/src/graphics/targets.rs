//! Drawing into canvases, and what is drawn to.

use super::*;

/// Sends everything drawn from now on to `canvas`, or back to the screen
/// for `None`.
///
/// Positions are then in units from the canvas's top-left corner. The
/// screen becomes the target again at the start of every `draw`.
///
/// Panics for a depth canvas, which holds no colors to draw to; draw
/// with one through [`set_canvas_with_depth`].
pub fn set_canvas(canvas: Option<&Canvas>) {
    if let Some(canvas) = canvas
        && canvas.format().is_depth()
    {
        panic!(
            "a {} canvas holds depth, not colors; draw with it through set_canvas_with_depth",
            canvas.format()
        );
    }
    context::with(|ctx| ctx.gfx.set_canvas(canvas.cloned()));
}

/// Returns the canvas being drawn to, or `None` for the screen. With
/// several canvases, it is the first.
pub fn canvas() -> Option<Canvas> {
    context::with(|ctx| ctx.gfx.canvas.clone())
}

/// Sends everything drawn from now on to all of `canvases` at once, the
/// equivalent of `love.graphics.setCanvas(canvas1, canvas2, ...)`, or
/// back to the screen for none. Up to [`MAX_CANVASES`] can be used.
///
/// The built-in shader, and a shader with an `effect` function, draw the
/// same color to every canvas. A shader with an `effects` function draws
/// its own color to each; see [`Shader`].
///
/// Panics if the canvases are not all the same size in pixels, are
/// multisampled differently, or are too many, which is always a bug in
/// the game. Use [`try_set_canvases`] to handle that instead.
#[doc(alias = "love.graphics.setCanvas")]
pub fn set_canvases(canvases: &[&Canvas]) {
    try_set_canvases(canvases).unwrap_or_else(|message| panic!("{message}"));
}

/// The most canvases [`set_canvases`] can draw to at once.
pub const MAX_CANVASES: usize = smitten_core::backend::MAX_TARGETS;

/// As [`set_canvases`], returning an error rather than panicking.
pub fn try_set_canvases(canvases: &[&Canvas]) -> Result<(), GraphicsError> {
    let targets: Vec<CanvasTarget> = canvases
        .iter()
        .map(|&canvas| CanvasTarget::new(canvas))
        .collect();
    try_set_canvas_targets(&targets)
}

/// A canvas to draw to, and which part of it: one layer of an array
/// canvas, one face of a cube canvas, or one slice of a volume canvas, at
/// one mipmap level. The equivalent of the table LÖVE's `setCanvas`
/// takes, `{canvas, layer = …, mipmap = …}`.
///
/// ```no_run
/// # use smitten::graphics::{self, Canvas, CanvasTarget};
/// # let cube: Canvas = todo!();
/// // Draw the +y face of a cube canvas.
/// graphics::set_canvas_targets(&[CanvasTarget::new(&cube).layer(2)]);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CanvasTarget<'a> {
    /// The canvas to draw to.
    pub canvas: &'a Canvas,
    /// The layer, cube face (+x, -x, +y, -y, +z, -z) or volume slice.
    pub layer: u32,
    /// The mipmap level, 0 for the full-size picture. Drawing to a
    /// smaller level needs [`MipmapMode::Manual`].
    pub mipmap: u32,
}

impl<'a> CanvasTarget<'a> {
    /// The whole of `canvas`, or its first layer.
    pub fn new(canvas: &'a Canvas) -> Self {
        Self {
            canvas,
            layer: 0,
            mipmap: 0,
        }
    }

    /// Draws to layer `layer` instead.
    pub fn layer(self, layer: u32) -> Self {
        Self { layer, ..self }
    }

    /// Draws to mipmap level `mipmap` instead.
    pub fn mipmap(self, mipmap: u32) -> Self {
        Self { mipmap, ..self }
    }

    fn slice(&self) -> TargetSlice {
        TargetSlice {
            layer: self.layer,
            mipmap: self.mipmap,
        }
    }
}

/// Sends everything drawn from now on to these parts of canvases, as
/// [`set_canvases`] does for whole ones. Positions are then in units of
/// the mipmap level drawn to, which is half the size for each level.
///
/// Panics if a layer or level does not exist, if the parts are not all
/// the same size in pixels or multisampled alike, or for a smaller level
/// of a canvas with [`MipmapMode::Auto`]. Use [`try_set_canvas_targets`]
/// to handle that instead.
pub fn set_canvas_targets(targets: &[CanvasTarget]) {
    try_set_canvas_targets(targets).unwrap_or_else(|message| panic!("{message}"));
}

/// As [`set_canvas_targets`], returning an error rather than panicking.
pub fn try_set_canvas_targets(targets: &[CanvasTarget]) -> Result<(), GraphicsError> {
    set_targets_and_depth(targets, None)
}

/// Sends everything drawn from now on to these canvases, as
/// [`set_canvas_targets`] does, testing and writing depth and stencil
/// values in `depth_stencil` rather than in the canvases' own, the
/// equivalent of LÖVE's `setCanvas{canvas, depthstencil = depth}`.
///
/// `depth_stencil` is a canvas made with a depth format, such as
/// [`PixelFormat::Depth24Stencil8`],
/// the same size in pixels as what is drawn to and multisampled alike.
/// Unlike a canvas's own values, it can be shared between canvases,
/// cleared with [`clear`] and [`ClearValues`], and, made readable, sampled afterwards, for
/// instance as a shadow map. Without a stencil part, stencil drawing and
/// tests do nothing. [`depth_stencil_canvas`] returns it.
///
/// Panics as [`set_canvas_targets`] does, or if `depth_stencil` is not a
/// depth canvas that matches the targets. Use
/// [`try_set_canvas_with_depth`] to handle that instead.
pub fn set_canvas_with_depth(targets: &[CanvasTarget], depth_stencil: &Canvas) {
    try_set_canvas_with_depth(targets, depth_stencil).unwrap_or_else(|message| panic!("{message}"));
}

/// As [`set_canvas_with_depth`], returning an error rather than
/// panicking.
pub fn try_set_canvas_with_depth(
    targets: &[CanvasTarget],
    depth_stencil: &Canvas,
) -> Result<(), GraphicsError> {
    set_targets_and_depth(targets, Some(depth_stencil))
}

/// The depth canvas being drawn with, set by [`set_canvas_with_depth`],
/// or `None` when the canvases use their own depth and stencil values.
pub fn depth_stencil_canvas() -> Option<Canvas> {
    context::with(|ctx| ctx.gfx.depth_canvas.clone())
}

fn set_targets_and_depth(
    targets: &[CanvasTarget],
    depth: Option<&Canvas>,
) -> Result<(), GraphicsError> {
    let Some(first) = targets.first() else {
        if depth.is_some() {
            return Err(GraphicsError::new(
                "a depth canvas can only be drawn with color canvases, not the screen",
            ));
        }
        set_canvas(None);
        return Ok(());
    };
    if let Some(depth) = depth {
        if !depth.format().is_depth() {
            return Err(format!(
                "a {} canvas cannot hold depth; make it with a depth format",
                depth.format()
            )
            .into());
        }
        if depth.pixel_dimensions() != first.canvas_pixels() {
            let (width, height) = first.canvas_pixels();
            let (depth_width, depth_height) = depth.pixel_dimensions();
            return Err(format!(
                "the depth canvas must be the size of what is drawn to: {width} by {height} \
                 pixels, not {depth_width} by {depth_height}"
            )
            .into());
        }
        if depth.msaa() != first.canvas.msaa() {
            return Err(GraphicsError::new(
                "the depth canvas must be multisampled like the canvases drawn to",
            ));
        }
    }
    if targets.len() > MAX_CANVASES {
        return Err(format!(
            "at most {MAX_CANVASES} canvases can be drawn to at once, not {}",
            targets.len()
        )
        .into());
    }
    for (index, target) in targets.iter().enumerate() {
        let canvas = target.canvas;
        if canvas.format().is_depth() {
            return Err(format!(
                "a {} canvas holds depth, not colors; draw with it through set_canvas_with_depth",
                canvas.format()
            )
            .into());
        }
        if target.layer >= canvas.layer_count() {
            return Err(format!(
                "layer {} does not exist: the canvas has {}",
                target.layer,
                canvas.layer_count()
            )
            .into());
        }
        if target.mipmap >= canvas.mipmap_count() {
            return Err(format!(
                "mipmap level {} does not exist: the canvas has {}",
                target.mipmap,
                canvas.mipmap_count()
            )
            .into());
        }
        if target.mipmap > 0 && canvas.mipmap_mode() == MipmapMode::Auto {
            return Err(GraphicsError::new(
                "a canvas whose mipmaps are made automatically can only be drawn to at level 0; \
                 make it with MipmapMode::Manual",
            ));
        }
        if target.canvas_pixels() != first.canvas_pixels() {
            let (width, height) = first.canvas_pixels();
            let (other_width, other_height) = target.canvas_pixels();
            return Err(format!(
                "canvases drawn to at once must be the same size: the first is \
                 {width} by {height} pixels, but canvas {index} is {other_width} by {other_height}"
            )
            .into());
        }
        if canvas.msaa() != first.canvas.msaa() {
            return Err(GraphicsError::new(
                "canvases drawn to at once must be multisampled alike",
            ));
        }
        if targets[..index].iter().any(|other| {
            other.canvas.texture() == canvas.texture() && other.slice() == target.slice()
        }) {
            return Err(GraphicsError::new(
                "the same canvas cannot be drawn to twice at once",
            ));
        }
    }
    let mut slices = [TargetSlice::default(); MAX_CANVASES];
    for (slot, target) in slices.iter_mut().zip(targets) {
        *slot = target.slice();
    }
    let rest = targets[1..]
        .iter()
        .map(|target| target.canvas.clone())
        .collect();
    context::with(|ctx| {
        ctx.gfx.set_canvas_list(CanvasList {
            canvas: Some(first.canvas.clone()),
            extra: rest,
            slices,
            depth: depth.cloned(),
        })
    });
    Ok(())
}

impl CanvasTarget<'_> {
    /// The size in pixels of the part drawn to.
    fn canvas_pixels(&self) -> (u32, u32) {
        let (width, height) = self.canvas.pixel_dimensions();
        (
            (width >> self.mipmap).max(1),
            (height >> self.mipmap).max(1),
        )
    }
}

/// Every canvas being drawn to, the first being [`canvas`]. Empty for the
/// screen.
pub fn canvases() -> Vec<Canvas> {
    context::with(|ctx| {
        ctx.gfx
            .canvas
            .iter()
            .chain(&ctx.gfx.extra_canvases)
            .cloned()
            .collect()
    })
}

/// Takes a picture of the window once the current frame has been drawn,
/// and calls `callback` with it, as LÖVE's `captureScreenshot` does.
///
/// The picture is the whole window in pixels, including any bars around a
/// design size, and is fully opaque. `callback` runs at the end of the
/// frame, after `draw` returns. Called outside `draw`, the picture is of
/// the next frame.
///
/// Reading pixels back waits for the GPU, so this is too slow to do
/// every frame.
///
/// ```no_run
/// use smitten::graphics;
///
/// graphics::capture_screenshot(|picture| {
///     picture.save("screenshot.png").expect("could not save the screenshot");
/// });
/// ```
pub fn capture_screenshot(callback: impl FnOnce(ImageData) + 'static) {
    context::with(|ctx| ctx.gfx.screenshots.push(Box::new(callback)));
}

/// Takes a picture of the window once the current frame has been drawn
/// and writes it as a PNG file in the save directory. A failure to write
/// the file is reported on the terminal. See [`capture_screenshot`].
pub fn save_screenshot(path: impl AsRef<Path>) {
    let path = path.as_ref().to_path_buf();
    capture_screenshot(move |picture| {
        if let Err(error) = picture.save(&path) {
            eprintln!(
                "smitten: could not save the screenshot {}: {error}",
                path.display()
            );
        }
    });
}

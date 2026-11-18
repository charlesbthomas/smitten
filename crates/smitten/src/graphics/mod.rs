//! Drawing, the equivalent of `love.graphics`.
//!
//! Positions are in pixels from the top-left corner of the window, with y
//! pointing down. Everything drawn is tinted by the current color and moved
//! by the current transform, both of which are set with functions here.
//!
//! Drawing normally goes to the screen. A [`Canvas`] is an off-screen
//! image to draw to instead, for effects, lighting, or rendering a
//! low-resolution game and scaling it up.
//!
//! A [`Shader`] replaces the way pixels are colored, for effects such as
//! grayscale, outlines or distortion.

mod batch;
mod canvas;
mod compressed;
mod draw;
mod error;
pub(crate) mod image;
mod image_data;
mod layered;
mod mesh;
mod particles;
mod pixels;
mod screen;
mod shader;
mod shapes;
mod sprite_batch;
mod state;
mod stats;
pub(crate) mod text;
mod text_object;
mod transform;
#[cfg(feature = "video")]
mod video;

use glam::Vec2;
use std::path::Path;

use smitten_core::backend::{
    Backend, BufferId, ColorMask, Frame, MeshDraw, Pass, RenderTargetDesc, Scissor, TargetSlice,
    TextureId, Vertex,
};

use crate::context;
use smitten_core::{DepthMode, PixelFormat, StencilMode};

use crate::{
    Blend, BlendAlphaMode, BlendMode, Color, CompareMode, CullMode, Filter, StencilAction,
    VertexWinding,
};
use batch::Batch;
pub(crate) use screen::Screen;

/// What [`crate::reload`] needs from the asset types.
pub(crate) mod reloading {
    pub(crate) use super::image::{Inner as ImageInner, reload as reload_image};
    pub(crate) use super::shader::{Inner as ShaderInner, reload as reload_shader};
    pub(crate) use super::text::{Inner as FontInner, reload as reload_font};
}

pub use canvas::{Canvas, CanvasSettings, MipmapMode};
pub use compressed::CompressedImageData;
pub use draw::{DrawParams, Drawable, ImageRegion, TextSpans};
pub use error::GraphicsError;
pub use image::{Image, ImageError, ImageSettings, Quad};
pub use image_data::{ImageData, ImageFormat, is_compressed};
pub use layered::{ArrayImage, CubeImage, VolumeImage};
pub use mesh::{Mesh, MeshDrawMode, MeshVertex, VertexAttribute};
pub use particles::{AreaSpreadDistribution, EmissionArea, ParticleInsertMode, ParticleSystem};
pub use shader::{MatrixLayout, Shader, ShaderError, Texture, UniformValue};
pub use shapes::{LineJoin, LineStyle};
pub use smitten_core::backend::{
    CompressedFormat, IndexDataType, RendererInfo, TextureKind, VertexDataType,
    VertexStep as VertexAttributeStep,
};
pub use sprite_batch::{SpriteBatch, SpriteBatchUsage};
use state::StackType;
pub use stats::Stats;
pub use text::{Align, Font, FontError, FontSettings, Hinting};
pub use text_object::Text;
pub use transform::Transform;
#[cfg(feature = "video")]
pub use video::Video;

/// Whether a shape is filled in or drawn as an outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawMode {
    Fill,
    Line,
}

/// How the ends of an arc are joined, as LÖVE's `ArcType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArcType {
    /// Both ends are joined to the center, like a slice of pie.
    #[default]
    Pie,
    /// The ends are left apart. Filled, it looks the same as `Closed`.
    Open,
    /// The ends are joined to each other by a straight line.
    Closed,
}

pub(crate) struct GraphicsState {
    background: Color,
    color: Color,
    line_width: f32,
    line_join: LineJoin,
    line_style: LineStyle,
    point_size: f32,
    pub(crate) default_filter: Filter,
    transform: Transform,
    stack: Vec<state::Saved>,
    batch: Batch,
    /// The font `print` uses. Created on first use so games that never
    /// print text pay nothing for it.
    font: Option<Font>,
    /// Where drawing goes, or `None` for the screen.
    canvas: Option<Canvas>,
    /// The canvases drawn to at the same time as `canvas`, if any.
    extra_canvases: Vec<Canvas>,
    /// The layer and mipmap level drawn to on each canvas in use.
    canvas_slices: [TargetSlice; MAX_CANVASES],
    /// The depth canvas drawn with, in place of the canvases' own depth
    /// and stencil values.
    depth_canvas: Option<Canvas>,
    /// The custom shader in use, if any.
    shader: Option<Shader>,
    /// Reused between draws of large things so they do not allocate.
    scratch: Vec<Vertex>,
    /// How units map onto the window.
    pub(crate) screen: Screen,
    /// Where the game's picture is drawn before it is enlarged to the
    /// window, when the design size asks for pixel-perfect scaling.
    surface: Option<Canvas>,
    /// The scissor the game asked for, in units of the current target.
    scissor: Option<(i32, i32, u32, u32)>,
    /// Who wants a copy of the frame being drawn.
    screenshots: Vec<ScreenshotCallback>,
    /// Which triangles of meshes are left out.
    mesh_cull: CullMode,
    /// Which winding faces the viewer.
    front_face: VertexWinding,
    /// Counts the frames handed to the backend.
    frame_number: u64,
    /// GPU buffers no longer wanted, freed once the frame that may still
    /// draw from them has been handed to the backend.
    retired_buffers: Vec<BufferId>,
}

/// The canvases drawn to at once, as [`try_set_canvas_targets`] and
/// [`set_canvas_with_depth`] choose them.
#[derive(Clone, Default)]
pub(crate) struct CanvasList {
    /// The first, or `None` for the screen.
    canvas: Option<Canvas>,
    extra: Vec<Canvas>,
    /// The layer and mipmap level drawn to on each.
    slices: [TargetSlice; MAX_CANVASES],
    /// The depth canvas, only with a `canvas`.
    depth: Option<Canvas>,
}

/// Called with the finished frame by [`capture_screenshot`].
pub(crate) type ScreenshotCallback = Box<dyn FnOnce(ImageData)>;

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            background: Color::BLACK,
            color: Color::WHITE,
            line_width: 1.0,
            line_join: LineJoin::default(),
            line_style: LineStyle::default(),
            point_size: 1.0,
            default_filter: Filter::default(),
            transform: Transform::IDENTITY,
            stack: Vec::new(),
            batch: Batch::default(),
            font: None,
            canvas: None,
            extra_canvases: Vec::new(),
            canvas_slices: Default::default(),
            depth_canvas: None,
            shader: None,
            scratch: Vec::new(),
            screen: Screen::default(),
            surface: None,
            scissor: None,
            screenshots: Vec::new(),
            mesh_cull: CullMode::default(),
            front_face: VertexWinding::default(),
            frame_number: 0,
            retired_buffers: Vec::new(),
        }
    }
}

impl GraphicsState {
    /// Resets what LÖVE resets before each `draw`: the screen and the
    /// transform. The color, line width, blend mode and scissor carry over
    /// between frames.
    ///
    /// Anything queued for a canvas before now, in `load` or `update`, is
    /// kept and drawn with this frame.
    pub(crate) fn begin_frame(&mut self, backend: &mut dyn Backend) {
        self.screen
            .update(backend.size_in_pixels(), backend.dpi_scale());
        if let Some((width, height)) = self.screen.pixel_perfect()
            && self.surface.is_none()
        {
            // If the GPU refuses, the game draws straight to the window.
            self.surface = Canvas::create(
                backend,
                width,
                height,
                Filter::Nearest,
                CanvasSettings {
                    dpi_scale: Some(1.0),
                    ..CanvasSettings::default()
                },
            )
            .ok();
        }
        if self.surface.is_some() {
            // The window itself only shows the surface, between black bars.
            self.batch.set_target(None);
            self.batch.clear_target(Color::BLACK);
        }
        self.set_canvas(None);
        self.batch.clear_target(self.background);
        self.transform = Transform::IDENTITY;
        self.stack.clear();
    }

    /// Called after `draw`: puts the game's picture in the window when a
    /// design size is in use.
    pub(crate) fn finish_frame(&mut self) {
        if self.screen.design.is_none() {
            return;
        }
        self.batch.set_target(None);
        let quad = |[left, top, right, bottom]: [f32; 4], color: [f32; 4]| {
            let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
                position: [x, y],
                uv: [u, v],
                color,
            };
            [
                corner(left, top, 0.0, 0.0),
                corner(right, top, 1.0, 0.0),
                corner(right, bottom, 1.0, 1.0),
                corner(left, top, 0.0, 0.0),
                corner(right, bottom, 1.0, 1.0),
                corner(left, bottom, 0.0, 1.0),
            ]
        };
        if let Some(surface) = &self.surface {
            let view = self.screen.view();
            let (width, height) = surface.dimensions();
            let area = [
                view.x,
                view.y,
                view.x + width as f32 * view.scale,
                view.y + height as f32 * view.scale,
            ];
            self.batch.push_plain(
                Some(surface.texture()),
                Filter::Nearest,
                BlendMode::Replace,
                &quad(area, [1.0; 4]),
            );
        } else {
            // The game drew at full resolution and may have spilled past
            // its area, so cover the rest of the window.
            for bar in self.screen.bars() {
                self.batch.push_plain(
                    None,
                    Filter::default(),
                    BlendMode::Replace,
                    &quad(bar, [0.0, 0.0, 0.0, 1.0]),
                );
            }
        }
    }

    /// Puts drawing back to how it starts out, for the error screen: the
    /// game may have panicked half way through a frame, with a canvas, a
    /// shader or a scissor in force. A design size is dropped too, since
    /// it may be too small to read text in.
    ///
    /// Returns what the game left in use. Drop it once the framework state
    /// is no longer borrowed.
    pub(crate) fn reset_for_error_screen(&mut self, background: Color) -> impl Sized + use<> {
        self.batch.clear();
        self.batch.set_shader(None);
        self.batch.blend = BlendMode::default();
        self.batch.blend_alpha = BlendAlphaMode::default();
        self.batch.stencil = StencilMode::Off;
        self.batch.color_mask = ColorMask::ALL;
        self.batch.depth = DepthMode::default();
        self.batch.wireframe = false;
        self.scissor = None;
        self.screen.design = None;
        let leftovers = (
            self.surface.take(),
            self.canvas.take(),
            std::mem::take(&mut self.extra_canvases),
            self.depth_canvas.take(),
            self.shader.take(),
            self.font.take(),
        );
        self.set_canvas(None);
        self.background = background;
        self.color = Color::WHITE;
        self.font = Some(Font::fallback_at(
            text::ERROR_FONT_SIZE,
            self.default_filter,
            self.screen.draw_scale(),
        ));
        leftovers
    }

    /// Queues text at `(x, y)` in the current font, for the framework's
    /// own messages.
    pub(crate) fn print_plain(
        &mut self,
        backend: &mut dyn Backend,
        text: &str,
        x: f32,
        y: f32,
        wrap: Option<(f32, Align)>,
    ) {
        let local = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
        self.push_text(backend, text, wrap, &local);
    }

    /// Draws what is queued for canvases now, ahead of the rest of the
    /// frame, so that their pixels are up to date.
    fn flush_canvases(&mut self, backend: &mut dyn Backend) {
        if self.batch.passes.iter().any(|pass| pass.target.is_some()) {
            backend.render_offscreen(&self.frame());
            self.batch.forget_canvas_passes();
        }
    }

    /// Copies a canvas's pixels back from the GPU.
    pub(crate) fn read_canvas(
        &mut self,
        backend: &mut dyn Backend,
        canvas: &Canvas,
        slice: TargetSlice,
    ) -> Result<ImageData, GraphicsError> {
        self.flush_canvases(backend);
        let (width, height, pixels) = backend.read_texture_slice(canvas.texture(), slice)?;
        ImageData::from_raw(width, height, canvas.format(), pixels)
            .map_err(|error| GraphicsError::new(error.to_string()))
    }

    /// Called after `finish_frame`: hands a copy of the frame to everyone
    /// who asked for a screenshot. Returns the callbacks to call, each
    /// with its copy, once the framework state is no longer borrowed.
    pub(crate) fn take_screenshots(
        &mut self,
        backend: &mut dyn Backend,
    ) -> Vec<(ScreenshotCallback, ImageData)> {
        if self.screenshots.is_empty() {
            return Vec::new();
        }
        let callbacks = std::mem::take(&mut self.screenshots);
        match self.read_screen(backend) {
            Ok(data) => callbacks
                .into_iter()
                .map(|callback| (callback, data.clone()))
                .collect(),
            Err(message) => {
                eprintln!("smitten: could not take a screenshot: {message}");
                Vec::new()
            }
        }
    }

    /// Draws the queued frame into a texture instead of the window and
    /// copies it back. The window's own pixels cannot be read, and this
    /// way the picture is the same whatever is covering the window.
    fn read_screen(&mut self, backend: &mut dyn Backend) -> Result<ImageData, GraphicsError> {
        let (width, height) = backend.size_in_pixels();
        // Multisampled and encoded as the screen is, so the picture
        // matches it.
        let target = backend.create_render_target_with(&RenderTargetDesc {
            format: normal_format(backend),
            samples: backend.screen_msaa(),
            ..RenderTargetDesc::new(width, height)
        })?;
        let passes: Vec<Pass> = self
            .batch
            .passes
            .iter()
            .map(|pass| Pass {
                target: pass.target.or(Some(target)),
                ..*pass
            })
            .collect();
        backend.render_offscreen(&Frame {
            passes: &passes,
            ..self.frame()
        });
        let result = backend.read_texture(target);
        backend.destroy_texture(target);
        // The canvases have had their draws; only the screen's are left
        // for the window.
        self.batch.forget_canvas_passes();
        let (width, height, mut pixels) = result?;
        // The window is opaque whatever was drawn to it.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        Ok(ImageData::from_rgba(width, height, pixels))
    }

    /// Called once the frame has been handed to the backend.
    pub(crate) fn end_frame(&mut self, backend: &mut dyn Backend) {
        self.batch.clear();
        self.set_canvas(None);
        for buffer in self.retired_buffers.drain(..) {
            backend.destroy_buffer(buffer);
        }
        self.frame_number += 1;
    }

    /// Which frame is being queued, counting from 0.
    pub(crate) fn frame_number(&self) -> u64 {
        self.frame_number
    }

    /// Frees GPU buffers once nothing queued can draw from them.
    pub(crate) fn retire_buffers(&mut self, buffers: &[BufferId]) {
        self.retired_buffers.extend_from_slice(buffers);
    }

    /// How units of the current target become its pixels.
    fn base(&self) -> Transform {
        match &self.canvas {
            Some(canvas) => {
                let mut transform = Transform::IDENTITY;
                transform.scale(canvas.dpi_scale(), canvas.dpi_scale());
                transform
            }
            None if self.surface.is_some() => Transform::IDENTITY,
            None => self.screen.view().transform(),
        }
    }

    /// Pixels per unit on the current target.
    fn target_scale(&self) -> f32 {
        match &self.canvas {
            Some(canvas) => canvas.dpi_scale(),
            None if self.surface.is_some() => 1.0,
            None => self.screen.view().scale,
        }
    }

    /// The game's transform followed by the step from units to pixels.
    fn device(&self) -> Transform {
        self.base().then_inner(&self.transform)
    }

    /// Hands the game's scissor to the batch in pixels of the target.
    fn apply_scissor(&mut self) {
        let base = self.base();
        self.batch.scissor = self.scissor.map(|(x, y, width, height)| {
            let Vec2 { x: left, y: top } = base.apply((x as f32, y as f32));
            let Vec2 {
                x: right,
                y: bottom,
            } = base.apply((x as f32 + width as f32, y as f32 + height as f32));
            let (left, top) = (left.round(), top.round());
            Scissor {
                x: left as i32,
                y: top as i32,
                width: (right.round() - left).max(0.0) as u32,
                height: (bottom.round() - top).max(0.0) as u32,
            }
        });
    }

    /// The size in units of what is being drawn to.
    fn target_size(&self) -> (u32, u32) {
        match &self.canvas {
            Some(canvas) => {
                let (w, h) = canvas.dimensions();
                let level = self.canvas_slices[0].mipmap;
                ((w >> level).max(1), (h >> level).max(1))
            }
            None => self.screen.size(),
        }
    }

    /// The size in pixels of what is being drawn to.
    fn target_pixel_size(&self) -> (u32, u32) {
        match &self.canvas {
            Some(canvas) => {
                let (w, h) = canvas.pixel_dimensions();
                let level = self.canvas_slices[0].mipmap;
                ((w >> level).max(1), (h >> level).max(1))
            }
            None => self.screen.pixel_size(),
        }
    }

    pub(crate) fn frame(&self) -> Frame<'_> {
        Frame {
            vertices: &self.batch.vertices,
            meshes: &self.batch.meshes,
            calls: &self.batch.calls,
            uniforms: &self.batch.uniforms,
            textures: &self.batch.textures,
            passes: &self.batch.passes,
        }
    }

    fn set_shader(&mut self, shader: Option<Shader>) {
        match &shader {
            Some(shader) => shader.with_data(|data, textures| {
                self.batch.set_shader(Some((shader.id(), data, textures)))
            }),
            None => self.batch.set_shader(None),
        }
        self.shader = shader;
    }

    /// Picks up new values sent to `shader` if it is the one in use.
    pub(crate) fn refresh_shader(&mut self, shader: &Shader) {
        if self.shader.as_ref().is_some_and(|active| active.is(shader)) {
            shader.with_data(|data, textures| {
                self.batch.set_shader(Some((shader.id(), data, textures)))
            });
        }
    }

    /// The canvases drawn to now and the slice of each, for putting back
    /// later with `set_canvas_list`.
    pub(crate) fn canvas_list(&self) -> CanvasList {
        CanvasList {
            canvas: self.canvas.clone(),
            extra: self.extra_canvases.clone(),
            slices: self.canvas_slices,
            depth: self.depth_canvas.clone(),
        }
    }

    fn set_canvas(&mut self, canvas: Option<Canvas>) {
        self.set_canvas_list(CanvasList {
            canvas,
            ..CanvasList::default()
        });
    }

    /// Draws to `list.canvas` and, at the same time, to each of
    /// `list.extra`, with `list.depth`, which the caller has checked match
    /// it.
    pub(crate) fn set_canvas_list(&mut self, list: CanvasList) {
        let CanvasList {
            canvas,
            extra,
            slices,
            depth,
        } = list;
        let (slices, depth) = if canvas.is_some() {
            (slices, depth)
        } else {
            (Default::default(), None)
        };
        let target = canvas.as_ref().or(self.surface.as_ref());
        let mut targets = batch::Targets::single(target.map(|canvas| canvas.texture()));
        for (slot, canvas) in targets.extra.iter_mut().zip(&extra) {
            *slot = Some(canvas.texture());
        }
        targets.slices = slices;
        targets.depth = depth.as_ref().map(|depth| depth.texture());
        self.batch.set_targets(targets);
        self.canvas = canvas;
        self.extra_canvases = extra;
        self.canvas_slices = slices;
        self.depth_canvas = depth;
        self.apply_scissor();
    }

    /// Queues untextured triangles given in local coordinates.
    /// How lines are drawn right now.
    fn stroke(&self) -> shapes::Stroke {
        let fringe = match self.line_style {
            LineStyle::Rough => 0.0,
            LineStyle::Smooth => {
                // The size of one pixel of the target in the game's
                // coordinates, taken from how much the transform enlarges
                // areas.
                let device = self.device();
                let Vec2 { x: ox, y: oy } = device.apply((0.0, 0.0));
                let Vec2 { x: ax, y: ay } = device.apply((1.0, 0.0));
                let Vec2 { x: bx, y: by } = device.apply((0.0, 1.0));
                let area = ((ax - ox) * (by - oy) - (bx - ox) * (ay - oy)).abs();
                if area > 1e-12 { 1.0 / area.sqrt() } else { 0.0 }
            }
        };
        shapes::Stroke {
            width: self.line_width,
            join: self.line_join,
            fringe,
        }
    }

    /// Queues triangles in the current color, faded by each corner's
    /// coverage.
    fn push_covered(&mut self, triangles: &[shapes::Covered]) {
        let color = color_array(self.color);
        let device = self.device();
        let vertices: Vec<Vertex> = triangles
            .iter()
            .map(|&(x, y, coverage)| {
                let Vec2 { x, y } = device.apply((x, y));
                Vertex {
                    position: [x, y],
                    uv: [0.0, 0.0],
                    color: [color[0], color[1], color[2], color[3] * coverage],
                }
            })
            .collect();
        self.batch.push(None, Filter::default(), &vertices);
    }

    /// Queues a filled shape in the current color. Its texture
    /// coordinates run from 0 to 1 across the box around it, which only
    /// a custom shader can tell.
    fn push_shape(&mut self, triangles: &[(f32, f32)]) {
        let color = color_array(self.color);
        let device = self.device();
        let uv_of = shapes::spread_uv(triangles);
        let vertices: Vec<Vertex> = triangles
            .iter()
            .map(|&(x, y)| {
                let uv = uv_of(x, y);
                let Vec2 { x, y } = device.apply((x, y));
                Vertex {
                    position: [x, y],
                    uv,
                    color,
                }
            })
            .collect();
        self.batch.push(None, Filter::default(), &vertices);
    }

    /// Queues the part of `image` covered by `quad`, placed by `local`.
    fn push_image(&mut self, image: &Image, quad: &Quad, local: &Transform) {
        assert!(
            image.texture_kind() == TextureKind::Flat,
            "a {:?} canvas cannot be drawn like an image; draw a layer with \
             Canvas::layer or sample it in a shader",
            image.texture_kind()
        );
        self.push_texture(image.texture(), 0, image.filter(), quad, local);
    }

    /// Queues the part of layer `layer` of `texture` covered by `quad`,
    /// placed by `local`.
    fn push_texture(
        &mut self,
        texture: TextureId,
        layer: u32,
        filter: Filter,
        quad: &Quad,
        local: &Transform,
    ) {
        let transform = self.device().then_inner(local);
        let color = color_array(self.color);
        let (u0, v0) = (quad.x / quad.sheet_width, quad.y / quad.sheet_height);
        let (u1, v1) = (
            (quad.x + quad.width) / quad.sheet_width,
            (quad.y + quad.height) / quad.sheet_height,
        );
        let corner = |x: f32, y: f32, u: f32, v: f32| {
            let Vec2 { x, y } = transform.apply((x, y));
            Vertex {
                position: [x, y],
                uv: [u, v],
                color,
            }
        };
        let top_left = corner(0.0, 0.0, u0, v0);
        let top_right = corner(quad.width, 0.0, u1, v0);
        let bottom_right = corner(quad.width, quad.height, u1, v1);
        let bottom_left = corner(0.0, quad.height, u0, v1);
        self.batch.push_layer(
            Some(texture),
            layer,
            filter,
            &[
                top_left,
                top_right,
                bottom_right,
                top_left,
                bottom_right,
                bottom_left,
            ],
        );
    }
}

impl GraphicsState {
    /// Queues every particle of `system`, placed by `local`.
    fn push_particles(&mut self, system: &ParticleSystem, local: &Transform) {
        let transform = self.device().then_inner(local);
        let vertices = system.vertices(&transform, self.color);
        let image = system.texture();
        self.batch
            .push(Some(image.texture()), image.filter(), &vertices);
    }

    /// Queues every sprite of `sprites`, placed by `local`.
    fn push_sprite_batch(
        &mut self,
        backend: &mut dyn Backend,
        sprites: &SpriteBatch,
        local: &Transform,
    ) {
        let transform = self.device().then_inner(local);
        if sprites.is_layered() {
            assert!(
                self.shader.is_none(),
                "a sprite batch of an ArrayImage cannot be drawn with a custom shader active; \
                 send the array image to the shader with Shader::send_texture instead"
            );
        }
        // Attached attributes are only on the GPU, so the sprites are
        // drawn from GPU buffers as a mesh.
        if let Some(mesh) = sprites.attached_mesh() {
            let (texture, filter) = sprites.texture();
            let mut retired = Vec::new();
            let uploaded = mesh.upload(backend, self.frame_number, &mut retired, 1);
            self.retired_buffers.extend(retired);
            match uploaded {
                Ok(Some(geometry)) => {
                    let draw = MeshDraw {
                        layout: geometry.layout,
                        buffers: geometry.buffers,
                        vertex_count: geometry.vertex_count,
                        indices: geometry.indices,
                        index_type: geometry.index_type,
                        first_index: 0,
                        index_count: geometry.index_count,
                        instance_count: 1,
                        transform: transform.gpu_matrix(),
                        tint: color_array(self.color),
                    };
                    if sprites.is_layered() {
                        for (layer, range) in sprites.layer_runs() {
                            self.batch.push_mesh_layer(
                                Some(texture),
                                layer,
                                filter,
                                MeshDraw {
                                    first_index: range.start as u32,
                                    index_count: range.len() as u32,
                                    ..draw
                                },
                            );
                        }
                    } else {
                        self.batch.push_mesh(Some(texture), filter, draw);
                    }
                }
                Ok(None) => {}
                Err(message) => {
                    eprintln!("smitten: could not send a sprite batch to the GPU: {message}");
                }
            }
            return;
        }
        let mut vertices = std::mem::take(&mut self.scratch);
        vertices.clear();
        sprites.write_vertices(&transform, self.color, &mut vertices);
        let (texture, filter) = sprites.texture();
        if sprites.is_layered() {
            for (layer, range) in sprites.layer_runs() {
                self.batch
                    .push_layer(Some(texture), layer, filter, &vertices[range]);
            }
        } else {
            self.batch.push(Some(texture), filter, &vertices);
        }
        self.scratch = vertices;
    }

    /// Queues the triangles of `mesh`, placed by `local`.
    /// Queues `instances` copies of `mesh`, placed by `local`. Drawn
    /// without the GPU's help, as points are, it is drawn once.
    fn push_mesh(
        &mut self,
        backend: &mut dyn Backend,
        mesh: &Mesh,
        local: &Transform,
        instances: u32,
    ) {
        if instances == 0 {
            return;
        }
        let transform = self.device().then_inner(local);
        let (texture, filter) = match mesh.texture() {
            Some(image) => (Some(image.texture()), image.filter()),
            None => (None, Filter::default()),
        };
        // Points are squares that do not turn with the transform, which
        // the GPU cannot draw from the mesh's own vertices, so they are
        // made here like other shapes.
        if mesh.draw_mode() != MeshDrawMode::Points {
            let mut retired = Vec::new();
            let uploaded = mesh.upload(backend, self.frame_number, &mut retired, instances);
            self.retired_buffers.extend(retired);
            match uploaded {
                Ok(Some(geometry)) => {
                    self.batch.cull = self.mesh_cull.culled(self.front_face);
                    self.batch.push_mesh(
                        texture,
                        filter,
                        MeshDraw {
                            layout: geometry.layout,
                            buffers: geometry.buffers,
                            vertex_count: geometry.vertex_count,
                            indices: geometry.indices,
                            index_type: geometry.index_type,
                            first_index: 0,
                            index_count: geometry.index_count,
                            instance_count: instances,
                            transform: transform.gpu_matrix(),
                            tint: color_array(self.color),
                        },
                    );
                    self.batch.cull = None;
                    return;
                }
                Ok(None) => return,
                // Drawn the slower way instead.
                Err(message) => eprintln!("smitten: could not send a mesh to the GPU: {message}"),
            }
        }
        let mut vertices = std::mem::take(&mut self.scratch);
        vertices.clear();
        let point_size = self.point_size * self.target_scale();
        mesh.write_vertices(&transform, self.color, point_size, &mut vertices);
        self.batch.cull = self.mesh_cull.culled(self.front_face);
        self.batch.push(texture, filter, &vertices);
        self.batch.cull = None;
        self.scratch = vertices;
    }

    /// Queues a laid-out [`Text`], placed by `local`.
    fn push_text_object(&mut self, backend: &mut dyn Backend, text: &Text, local: &Transform) {
        let transform = self.device().then_inner(local);
        let tint = color_array(self.color);
        let filter = text.font().filter();
        let mut vertices = std::mem::take(&mut self.scratch);
        let batch = &mut self.batch;
        text.with_runs(backend, |texture, run| {
            vertices.clear();
            vertices.extend(run.iter().map(|vertex| {
                let Vec2 { x, y } = transform.apply((vertex.position[0], vertex.position[1]));
                let [r, g, b, a] = vertex.color;
                Vertex {
                    position: [x, y],
                    uv: vertex.uv,
                    color: [r * tint[0], g * tint[1], b * tint[2], a * tint[3]],
                }
            }));
            batch.push(Some(texture), filter, &vertices);
        });
        self.scratch = vertices;
    }

    /// Queues `text` in the current font, placed by `local`.
    fn push_text(
        &mut self,
        backend: &mut dyn Backend,
        text: &str,
        wrap: Option<(f32, Align)>,
        local: &Transform,
    ) {
        self.push_colored_text(backend, &[(Color::WHITE, text)], wrap, local);
    }

    /// Queues text made of `segments`, each tinted by its color as well as
    /// the current one, in the current font, placed by `local`.
    fn push_colored_text(
        &mut self,
        backend: &mut dyn Backend,
        segments: &[(Color, &str)],
        wrap: Option<(f32, Align)>,
        local: &Transform,
    ) {
        let text = text::joined(segments);
        let mut colors = text::GlyphColors::new(segments);
        let (default_filter, scale) = (self.default_filter, self.screen.draw_scale());
        let font = self
            .font
            .get_or_insert_with(|| Font::fallback(default_filter, scale))
            .clone();
        let transform = self.device().then_inner(local);
        let tint = self.color;
        let filter = font.filter();

        // Glyphs usually share one atlas page, so collect each run of
        // quads on the same page and queue it in one go.
        let mut run: Vec<Vertex> = Vec::with_capacity(text.len() * 6);
        let mut run_texture: Option<TextureId> = None;
        let batch = &mut self.batch;
        font.layout(backend, &text, wrap, |character, texture, rect, uv| {
            let color = color_array(multiply(tint, colors.color(character)));
            if run_texture != Some(texture) {
                batch.push(run_texture, filter, &run);
                run.clear();
                run_texture = Some(texture);
            }
            let corner = |x: f32, y: f32, u: f32, v: f32| {
                let Vec2 { x, y } = transform.apply((x, y));
                Vertex {
                    position: [x, y],
                    uv: [u, v],
                    color,
                }
            };
            let top_left = corner(rect[0], rect[1], uv[0], uv[1]);
            let top_right = corner(rect[2], rect[1], uv[2], uv[1]);
            let bottom_right = corner(rect[2], rect[3], uv[2], uv[3]);
            let bottom_left = corner(rect[0], rect[3], uv[0], uv[3]);
            run.extend_from_slice(&[
                top_left,
                top_right,
                bottom_right,
                top_left,
                bottom_right,
                bottom_left,
            ]);
        });
        batch.push(run_texture, filter, &run);
    }
}

fn multiply(a: Color, b: Color) -> Color {
    Color::rgba(a.r * b.r, a.g * b.g, a.b * b.b, a.a * b.a)
}

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn whole_image(image: &Image) -> Quad {
    let (width, height) = (image.width() as f32, image.height() as f32);
    Quad::new((0.0, 0.0), (width, height), (width, height))
}

// ---------------------------------------------------------------------------
// Screen

/// Clears the screen, or the active canvas, the equivalent of
/// `love.graphics.clear`. The scissor does not limit it.
///
/// A [`Color`] fills the picture with it, replacing whatever was there,
/// resets the stencil values to 0 and the depth values to 1.
/// [`ClearValues`] picks which of those to clear and to what.
///
/// ```no_run
/// use smitten::{graphics, Color};
/// use smitten::graphics::ClearValues;
///
/// graphics::clear(Color::BLACK);
/// // Reset only the stencil, keeping the picture.
/// graphics::clear(ClearValues::NONE.stencil(0));
/// ```
pub fn clear(values: impl Into<ClearValues>) {
    let ClearValues {
        color,
        stencil,
        depth,
    } = values.into();
    context::with(|ctx| ctx.gfx.batch.clear_target_ex(color, stencil, depth));
}

/// What [`clear`] clears: the picture, the stencil values and the depth
/// values, each to its own value, or left as they are when `None`. LÖVE's
/// `clear(color, stencil, depth)` and `clear(false, ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClearValues {
    /// The color to fill the picture with.
    pub color: Option<Color>,
    /// The value to set every stencil value to.
    pub stencil: Option<u8>,
    /// The value to set every depth value to, from 0 to 1.
    pub depth: Option<f32>,
}

impl ClearValues {
    /// Clears nothing, to add to with the other methods.
    pub const NONE: ClearValues = ClearValues {
        color: None,
        stencil: None,
        depth: None,
    };

    /// Also fills the picture with `color`.
    #[must_use]
    pub fn color(self, color: Color) -> Self {
        Self {
            color: Some(color),
            ..self
        }
    }

    /// Also sets every stencil value to `value`.
    #[must_use]
    pub fn stencil(self, value: u8) -> Self {
        Self {
            stencil: Some(value),
            ..self
        }
    }

    /// Also sets every depth value to `value`, from 0 to 1.
    #[must_use]
    pub fn depth(self, value: f32) -> Self {
        Self {
            depth: Some(value),
            ..self
        }
    }
}

impl From<Color> for ClearValues {
    fn from(color: Color) -> Self {
        ClearValues::NONE.color(color).stencil(0).depth(1.0)
    }
}

/// Fills each canvas being drawn to with its own color, the equivalent of
/// `love.graphics.clear(color1, color2, ...)` with several canvases set.
/// `colors[0]` is for the first canvas; a canvas with `None`, or with no
/// entry, keeps its picture. `stencil` sets every stencil value and
/// `depth` every depth value, if given. The scissor does not limit it.
pub fn clear_canvases(colors: &[Option<Color>], stencil: Option<u8>, depth: Option<f32>) {
    let mut each = [None; MAX_CANVASES];
    for (slot, color) in each.iter_mut().zip(colors) {
        *slot = *color;
    }
    context::with(|ctx| ctx.gfx.batch.clear_targets(each, stencil, depth));
}

/// Tells the GPU that the contents of the screen, or of the active
/// canvas, are no longer needed, the equivalent of `love.graphics.discard`.
///
/// LÖVE offers this as a speed hint for mobile GPUs: what is there
/// afterwards is undefined until it is drawn over or cleared. Smitten's
/// targets always keep their contents, so this does nothing, and code
/// ported from LÖVE can keep calling it.
pub fn discard() {}

/// Sets the color every frame is cleared to before `draw` runs.
pub fn set_background_color(color: Color) {
    context::with(|ctx| ctx.gfx.background = color);
}

/// Returns the current background color.
pub fn background_color() -> Color {
    context::with(|ctx| ctx.gfx.background)
}

/// Width of the screen, or of the active canvas.
pub fn width() -> u32 {
    dimensions().0
}

/// Height of the screen, or of the active canvas.
pub fn height() -> u32 {
    dimensions().1
}

/// Size of the screen, or of the active canvas, as `(width, height)`.
///
/// This is the space the game draws in. It is measured in units, which
/// are pixels on an ordinary display and larger on a high-DPI one (see
/// [`Config::dpi_scaling`](crate::Config::dpi_scaling)). With a
/// [`Config::design_size`](crate::Config::design_size) it is always that
/// size. [`pixel_dimensions`] gives the real pixels.
pub fn dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.gfx.target_size())
}

/// Width in real pixels of the screen, or of the active canvas.
pub fn pixel_width() -> u32 {
    pixel_dimensions().0
}

/// Height in real pixels of the screen, or of the active canvas.
pub fn pixel_height() -> u32 {
    pixel_dimensions().1
}

/// Size in real pixels of the screen, or of the active canvas, the
/// equivalent of `love.graphics.getPixelDimensions`. Shaders see
/// positions in these pixels.
pub fn pixel_dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.gfx.target_pixel_size())
}

// ---------------------------------------------------------------------------
// Canvases

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

// ---------------------------------------------------------------------------
// Stencil

/// Marks the pixels covered by what `draw` draws, so that
/// [`set_stencil_test`] can limit later drawing to them, or keep it away
/// from them. Nothing `draw` draws is visible itself.
///
/// Every pixel has a stencil value from 0 to 255. This first resets them
/// all to 0, then sets the covered pixels to 1. Use [`stencil_with`] for
/// other values or to build on earlier marks.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode};
/// use smitten::CompareMode;
///
/// // Draw a picture only inside a circle.
/// graphics::stencil(|| graphics::circle(DrawMode::Fill, (200.0, 200.0), 80.0));
/// graphics::set_stencil_test(CompareMode::Greater, 0);
/// // ... draw the picture ...
/// graphics::clear_stencil_test();
/// ```
///
/// The screen and each canvas have their own stencil values. The screen's
/// are also reset at the start of every frame.
pub fn stencil(draw: impl FnOnce()) {
    stencil_with(StencilWrite::default(), draw);
}

/// Changes the stencil values of the pixels covered by what `draw` draws,
/// as `write` says, the equivalent of LÖVE's
/// `stencil(fn, action, value, keepvalues)`.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode, StencilWrite};
/// use smitten::StencilAction;
///
/// // Count how many circles cover each pixel.
/// graphics::stencil_with(
///     StencilWrite { action: StencilAction::Increment, ..StencilWrite::default() },
///     || graphics::circle(DrawMode::Fill, (200.0, 200.0), 80.0),
/// );
/// ```
pub fn stencil_with(write: StencilWrite, draw: impl FnOnce()) {
    let StencilWrite {
        action,
        value,
        keep_values,
    } = write;
    let previous = context::with(|ctx| {
        let batch = &mut ctx.gfx.batch;
        if !keep_values {
            batch.clear_stencil();
        }
        let previous = (batch.stencil, batch.stencil_value);
        batch.stencil = StencilMode::Write(action);
        batch.stencil_value = value;
        previous
    });
    draw();
    context::with(|ctx| (ctx.gfx.batch.stencil, ctx.gfx.batch.stencil_value) = previous);
}

/// How [`stencil_with`] changes the stencil values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StencilWrite {
    /// How the values change: [`StencilAction::Replace`] sets them to
    /// `value`, while the other actions count up, count down (stopping
    /// at 255 and 0, or wrapping around with the `Wrap` actions) or flip
    /// them and ignore `value`. Defaults to `Replace`.
    pub action: StencilAction,
    /// The value `Replace` sets. Defaults to 1.
    pub value: u8,
    /// Whether to build on the existing values instead of resetting them
    /// all to 0 first. Defaults to `false`.
    pub keep_values: bool,
}

impl Default for StencilWrite {
    fn default() -> Self {
        StencilWrite {
            action: StencilAction::Replace,
            value: 1,
            keep_values: false,
        }
    }
}

/// Draws only the pixels whose stencil value compares to `value` as
/// `compare` says, until [`clear_stencil_test`] is called.
///
/// `set_stencil_test(CompareMode::Greater, 0)` draws only where
/// [`stencil`] marked, and `set_stencil_test(CompareMode::Equal, 0)` only
/// where it did not.
pub fn set_stencil_test(compare: CompareMode, value: u8) {
    context::with(|ctx| {
        ctx.gfx.batch.stencil = StencilMode::Test(compare);
        ctx.gfx.batch.stencil_value = value;
    });
}

/// Goes back to drawing everywhere.
pub fn clear_stencil_test() {
    context::with(|ctx| ctx.gfx.batch.stencil = StencilMode::Off);
}

/// Returns the stencil test as `(compare, value)`, if one is set.
pub fn stencil_test() -> Option<(CompareMode, u8)> {
    context::with(|ctx| match ctx.gfx.batch.stencil {
        StencilMode::Test(compare) => Some((compare, ctx.gfx.batch.stencil_value)),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// Shaders

/// Draws everything from now on with `shader`, or with the built-in
/// shader for `None`.
///
/// The shader stays set until changed, even across frames.
pub fn set_shader(shader: Option<&Shader>) {
    context::with(|ctx| ctx.gfx.set_shader(shader.cloned()));
}

/// Returns the shader in use, or `None` for the built-in one.
pub fn shader() -> Option<Shader> {
    context::with(|ctx| ctx.gfx.shader.clone())
}

// ---------------------------------------------------------------------------
// Drawing state

/// Sets the color used to draw shapes and to tint images.
///
/// White draws images unchanged. The color stays set until changed, even
/// across frames.
pub fn set_color(color: Color) {
    context::with(|ctx| ctx.gfx.color = color);
}

/// Returns the current drawing color.
pub fn color() -> Color {
    context::with(|ctx| ctx.gfx.color)
}

/// Sets how wide outlines and lines are, in pixels.
pub fn set_line_width(width: f32) {
    context::with(|ctx| ctx.gfx.line_width = width);
}

/// Returns the current line width.
pub fn line_width() -> f32 {
    context::with(|ctx| ctx.gfx.line_width)
}

/// Sets how wide and tall each point drawn by [`points`] is, in units.
pub fn set_point_size(size: f32) {
    context::with(|ctx| ctx.gfx.point_size = size);
}

/// Returns the current point size.
pub fn point_size() -> f32 {
    context::with(|ctx| ctx.gfx.point_size)
}

/// Sets how what is drawn is combined with what is already there, the
/// equivalent of LÖVE's `setBlendMode`. A [`BlendMode`] on its own uses
/// its usual alpha mode; [`BlendMode::with_alpha`] chooses another.
///
/// ```no_run
/// use smitten::{graphics, BlendAlphaMode, BlendMode};
///
/// # let canvas = smitten::graphics::Canvas::new(64, 64).unwrap();
/// graphics::set_blend_mode(BlendMode::Add);
/// // Add a canvas full of soft glows onto the screen.
/// graphics::set_blend_mode(BlendMode::Add.with_alpha(BlendAlphaMode::Premultiplied));
/// graphics::draw(&canvas, (0.0, 0.0));
/// ```
///
/// Panics for [`BlendMode::Multiply`], [`BlendMode::Lighten`] or
/// [`BlendMode::Darken`] with [`BlendAlphaMode::AlphaMultiply`], as LÖVE
/// raises an error: those modes only work premultiplied.
/// [`BlendMode::Premultiplied`] is always premultiplied whatever the
/// alpha mode says.
pub fn set_blend_mode(blend: impl Into<Blend>) {
    let Blend { mode, alpha } = blend.into();
    assert!(
        !(mode.needs_premultiplied()
            && mode != BlendMode::Premultiplied
            && alpha == BlendAlphaMode::AlphaMultiply),
        "the {mode:?} blend mode only works with BlendAlphaMode::Premultiplied"
    );
    context::with(|ctx| {
        ctx.gfx.batch.blend = mode;
        ctx.gfx.batch.blend_alpha = alpha;
    });
}

/// Returns the current blend mode and alpha mode, the equivalent of
/// LÖVE's `getBlendMode`. [`BlendMode::Premultiplied`] comes back as
/// itself, with [`BlendAlphaMode::Premultiplied`].
pub fn blend_mode() -> Blend {
    context::with(|ctx| {
        let mode = ctx.gfx.batch.blend;
        let alpha = if mode == BlendMode::Premultiplied {
            BlendAlphaMode::Premultiplied
        } else {
            ctx.gfx.batch.blend_alpha
        };
        mode.with_alpha(alpha)
    })
}

/// Limits drawing to a rectangle of the screen or the active canvas.
///
/// The rectangle is in units and is not affected by the transform. It
/// stays set until changed or removed with [`clear_scissor`].
pub fn set_scissor(x: i32, y: i32, width: u32, height: u32) {
    context::with(|ctx| {
        ctx.gfx.scissor = Some((x, y, width, height));
        ctx.gfx.apply_scissor();
    });
}

/// Removes the scissor so drawing can reach everywhere again.
pub fn clear_scissor() {
    context::with(|ctx| {
        ctx.gfx.scissor = None;
        ctx.gfx.apply_scissor();
    });
}

/// Limits drawing to the part of the current scissor that is also inside
/// the given rectangle, the equivalent of `love.graphics.intersectScissor`.
/// With no scissor set, it is the same as [`set_scissor`].
///
/// This suits nested panels: each can narrow where it draws without
/// knowing where its parent allows.
pub fn intersect_scissor(x: i32, y: i32, width: u32, height: u32) {
    context::with(|ctx| {
        let rect = (x, y, width, height);
        ctx.gfx.scissor = Some(match ctx.gfx.scissor {
            Some(current) => intersect(current, rect),
            None => rect,
        });
        ctx.gfx.apply_scissor();
    });
}

/// The overlap of two rectangles, empty at the corner of the first one's
/// overlap if they do not meet.
fn intersect(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> (i32, i32, u32, u32) {
    let right = |(x, _, w, _): (i32, i32, u32, u32)| i64::from(x) + i64::from(w);
    let bottom = |(_, y, _, h): (i32, i32, u32, u32)| i64::from(y) + i64::from(h);
    let left = a.0.max(b.0);
    let top = a.1.max(b.1);
    let width = (right(a).min(right(b)) - i64::from(left)).max(0) as u32;
    let height = (bottom(a).min(bottom(b)) - i64::from(top)).max(0) as u32;
    (left, top, width, height)
}

/// Returns the scissor rectangle as `(x, y, width, height)`, if one is set.
pub fn scissor() -> Option<(i32, i32, u32, u32)> {
    context::with(|ctx| ctx.gfx.scissor)
}

/// Sets the filter that newly created images start with.
///
/// Call this with [`Filter::Nearest`] before loading images in a pixel art
/// game.
pub fn set_default_filter(filter: Filter) {
    context::with(|ctx| ctx.gfx.default_filter = filter);
}

/// Sets how the corners of lines and outlines are drawn. They start out
/// mitered.
pub fn set_line_join(join: LineJoin) {
    context::with(|ctx| ctx.gfx.line_join = join);
}

/// Returns how the corners of lines and outlines are drawn.
pub fn line_join() -> LineJoin {
    context::with(|ctx| ctx.gfx.line_join)
}

/// Sets whether the edges of lines and outlines are softened. They start
/// out smooth, as in LÖVE. Pixel art usually wants [`LineStyle::Rough`].
///
/// Filled shapes are not affected: their edges are always hard.
pub fn set_line_style(style: LineStyle) {
    context::with(|ctx| ctx.gfx.line_style = style);
}

/// Returns whether the edges of lines and outlines are softened.
pub fn line_style() -> LineStyle {
    context::with(|ctx| ctx.gfx.line_style)
}

/// Chooses which color channels drawing may change, the equivalent of
/// `love.graphics.setColorMask`. A channel left out keeps what is already
/// there. Clearing the screen or a canvas ignores the mask.
pub fn set_color_mask(r: bool, g: bool, b: bool, a: bool) {
    context::with(|ctx| ctx.gfx.batch.color_mask = ColorMask { r, g, b, a });
}

/// Which color channels drawing may change, as `(r, g, b, a)`.
pub fn color_mask() -> (bool, bool, bool, bool) {
    context::with(|ctx| {
        let mask = ctx.gfx.batch.color_mask;
        (mask.r, mask.g, mask.b, mask.a)
    })
}

/// Chooses how drawing uses depth, the equivalent of
/// `love.graphics.setDepthMode`. Each pixel drawn is kept only where its
/// depth compares with the depth already at that pixel as `compare` says,
/// and with `write` its depth replaces it. `(CompareMode::Always, false)`,
/// the default, leaves depth out.
///
/// Depth comes from the z of a mesh's 3-component `VertexPosition`, or
/// from a [`Shader`]'s `position`: z from -10 to 10 maps to depth 1 to 0,
/// as in LÖVE, so a larger z is nearer. Everything else is drawn at z 0,
/// depth 0.5. The screen and every canvas have depth values of their own,
/// which [`clear`] resets to 1.
///
/// ```no_run
/// use smitten::{CompareMode, graphics};
///
/// graphics::set_depth_mode(CompareMode::Less, true);
/// // Draw meshes in any order; nearer pixels win.
/// graphics::set_depth_mode(CompareMode::Always, false);
/// ```
pub fn set_depth_mode(compare: CompareMode, write: bool) {
    context::with(|ctx| ctx.gfx.batch.depth = DepthMode { compare, write });
}

/// How drawing uses depth, as `(compare, write)`. See [`set_depth_mode`].
pub fn depth_mode() -> (CompareMode, bool) {
    context::with(|ctx| {
        let depth = ctx.gfx.batch.depth;
        (depth.compare, depth.write)
    })
}

/// Draws only the edges of every triangle from now on, one pixel wide,
/// to see how shapes, images and text are built. For debugging, as in
/// LÖVE.
pub fn set_wireframe(enabled: bool) {
    context::with(|ctx| ctx.gfx.batch.wireframe = enabled);
}

/// Whether wireframe drawing is on.
pub fn is_wireframe() -> bool {
    context::with(|ctx| ctx.gfx.batch.wireframe)
}

/// What the frame has drawn so far and how many images, canvases and
/// fonts the game holds, the equivalent of `love.graphics.getStats`. Call
/// it at the end of `draw` to see the whole frame.
pub fn stats() -> Stats {
    use stats::{Kind, alive};
    context::with(|ctx| {
        let counts = ctx.gfx.batch.counts;
        Stats {
            draw_calls: counts.draw_calls,
            draw_calls_batched: counts.draw_calls_batched,
            canvas_switches: counts.canvas_switches,
            shader_switches: counts.shader_switches,
            texture_memory: ctx.backend.texture_memory(),
            images: alive(Kind::Image),
            canvases: alive(Kind::Canvas),
            fonts: alive(Kind::Font),
        }
    })
}

/// Returns the filter that newly created images start with.
pub fn default_filter() -> Filter {
    context::with(|ctx| ctx.gfx.default_filter)
}

/// Sets which triangles of meshes are left out, the equivalent of
/// `love.graphics.setMeshCullMode`. Only [meshes](Mesh) are affected, as
/// in LÖVE.
///
/// Leaving out the back faces of a closed 3D-looking shape skips the
/// triangles that face away. [`set_front_face_winding`] says which faces
/// are the front.
pub fn set_mesh_cull_mode(mode: CullMode) {
    context::with(|ctx| ctx.gfx.mesh_cull = mode);
}

/// Which triangles of meshes are left out.
pub fn mesh_cull_mode() -> CullMode {
    context::with(|ctx| ctx.gfx.mesh_cull)
}

/// Sets which way round a triangle's corners go, on screen, when it faces
/// the viewer, the equivalent of `love.graphics.setFrontFaceWinding`. It
/// starts out counter-clockwise, as in LÖVE. A transform that flips the
/// picture flips the winding too.
pub fn set_front_face_winding(winding: VertexWinding) {
    context::with(|ctx| ctx.gfx.front_face = winding);
}

/// Which way round a triangle's corners go when it faces the viewer.
pub fn front_face_winding() -> VertexWinding {
    context::with(|ctx| ctx.gfx.front_face)
}

/// The graphics API, driver and GPU doing the drawing, the equivalent of
/// `love.graphics.getRendererInfo`. Useful in bug reports.
pub fn renderer_info() -> RendererInfo {
    context::with(|ctx| ctx.backend.renderer_info())
}

/// Checks WGSL shader source without creating a shader, the equivalent of
/// `love.graphics.validateShader`. The error is the message
/// [`Shader::new`] would give.
pub fn validate_shader(source: &str) -> Result<(), ShaderError> {
    context::with(|ctx| ctx.backend.validate_shader(source)).map_err(ShaderError::Compile)
}

/// Shows what has been drawn since the last present, the equivalent of
/// `love.graphics.present`, then starts the next frame: the screen is
/// cleared to [`background_color`] and the transform is reset.
///
/// A custom main loop made with [`crate::run_loop`] calls this once a
/// frame. With [`crate::Game`], Smitten presents after `draw` by itself,
/// so calling it there shows a half-drawn frame, as in LÖVE.
pub fn present() {
    present_frame();
    context::with(|ctx| ctx.gfx.begin_frame(&mut *ctx.backend));
}

/// Hands the frame to the window, or draws it offscreen while the window
/// is closed, and runs the screenshot callbacks it asked for.
pub(crate) fn present_frame() {
    let screenshots = context::with(|ctx| {
        ctx.gfx.finish_frame();
        let screenshots = ctx.gfx.take_screenshots(&mut *ctx.backend);
        if ctx.window_closed {
            ctx.backend.render_offscreen(&ctx.gfx.frame());
        } else {
            ctx.backend.present(&ctx.gfx.frame());
        }
        ctx.gfx.end_frame(&mut *ctx.backend);
        screenshots
    });
    for (callback, picture) in screenshots {
        callback(picture);
    }
}

/// Whether drawing reaches the screen, the equivalent of
/// `love.graphics.isActive`. It is `false` while the window is closed with
/// [`crate::window::close`]; drawing still works then, but only canvases
/// are worth drawing to. A custom main loop can skip drawing the screen
/// while it is `false`.
pub fn is_active() -> bool {
    context::with(|ctx| !ctx.window_closed)
}

/// Does nothing, the equivalent of `love.graphics.flushBatch`. Smitten
/// sends a whole frame to the GPU at once when it is presented, and keeps
/// draws in order without help, so there is nothing to flush early.
pub fn flush_batch() {}

/// Whether colors are blended in linear light, the equivalent of
/// `love.graphics.isGammaCorrect`. True when
/// [`Config::gamma_correct`](crate::Config::gamma_correct) was set and the
/// display allows it.
pub fn is_gamma_correct() -> bool {
    context::with(|ctx| ctx.backend.is_gamma_correct())
}

/// The format of an ordinary image or canvas: sRGB-encoded with
/// gamma-correct rendering, as LÖVE's `normal` format is.
pub(crate) fn normal_format(backend: &dyn Backend) -> PixelFormat {
    if backend.is_gamma_correct() {
        PixelFormat::Srgba8
    } else {
        PixelFormat::Rgba8
    }
}

// ---------------------------------------------------------------------------
// Transform stack

/// The most [`push`]es that can wait for a [`pop`] at once, the size of
/// LÖVE's stack.
pub const MAX_STACK_DEPTH: usize = 64;

/// Saves the current transform so [`pop`] can restore it.
///
/// Panics if more than [`MAX_STACK_DEPTH`] pushes are waiting for a
/// [`pop`], which is almost always a missing `pop` in the game.
pub fn push() {
    push_kind(StackType::Transform);
}

/// Saves the transform and every other drawing setting so [`pop`] can
/// restore them, the equivalent of `love.graphics.push("all")`.
///
/// That is the color, the background color, the font, the blend mode,
/// the shader, the canvas, the scissor, the stencil test, the line width,
/// join and style, the point size, the color mask, wireframe drawing, the
/// default filter, and the mesh cull mode and front face winding. A helper
/// can then change any of them without the caller having to put them
/// back.
///
/// ```no_run
/// use smitten::{graphics, BlendMode, Color};
///
/// fn draw_glow() {
///     graphics::push_all();
///     graphics::set_blend_mode(BlendMode::Add);
///     graphics::set_color(Color::rgb(1.0, 0.8, 0.3));
///     // ... draw ...
///     graphics::pop();
/// }
/// ```
pub fn push_all() {
    push_kind(StackType::All);
}

/// Saves what `kind` says, for [`pop`] to restore.
fn push_kind(kind: StackType) {
    context::with(|ctx| {
        assert!(
            ctx.gfx.stack.len() < MAX_STACK_DEPTH,
            "graphics::push was called more than {MAX_STACK_DEPTH} times without a graphics::pop"
        );
        ctx.gfx.push_state(kind);
    });
}

/// Restores what the matching [`push`] or [`push_all`] saved.
///
/// Panics if there is no matching `push`, which is always a bug in the game.
pub fn pop() {
    context::with(|ctx| ctx.gfx.pop_state());
}

/// Runs `draw` between a [`push`] and its [`pop`], so changes it makes to
/// the transform are undone afterwards, and returns what `draw` returns.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode};
///
/// graphics::scoped(|| {
///     graphics::translate((100.0, 100.0));
///     graphics::rotate(0.5);
///     graphics::rectangle(DrawMode::Fill, (-10.0, -10.0), (20.0, 20.0));
/// });
/// // Back to the transform from before.
/// ```
pub fn scoped<R>(draw: impl FnOnce() -> R) -> R {
    push();
    let result = draw();
    pop();
    result
}

/// Runs `draw` between a [`push_all`] and its [`pop`], so every drawing
/// setting it changes is put back afterwards, and returns what `draw`
/// returns.
pub fn scoped_all<R>(draw: impl FnOnce() -> R) -> R {
    push_all();
    let result = draw();
    pop();
    result
}

/// How many [`push`]es are waiting for a [`pop`], the equivalent of
/// `love.graphics.getStackDepth`. It is 0 at the start of every `draw`.
pub fn stack_depth() -> usize {
    context::with(|ctx| ctx.gfx.stack.len())
}

/// Puts every drawing setting back to how a game starts, the equivalent
/// of `love.graphics.reset`.
///
/// The color becomes white and the background black, drawing goes back to
/// the screen with the built-in shader and font, the scissor and stencil
/// test are removed, the blend mode, line and point settings, color mask,
/// wireframe drawing and default filter go back to their defaults, and the
/// transform to the origin. What [`push`] saved is kept.
pub fn reset() {
    context::with(|ctx| ctx.gfx.reset_state());
}

/// Moves everything drawn afterwards by `(dx, dy)`.
pub fn translate(delta: impl Into<Vec2>) {
    let Vec2 { x: dx, y: dy } = delta.into();
    context::with(|ctx| ctx.gfx.transform.translate((dx, dy)));
}

/// Rotates everything drawn afterwards by `angle` radians around the
/// current origin, clockwise on screen.
pub fn rotate(angle: f32) {
    context::with(|ctx| ctx.gfx.transform.rotate(angle));
}

/// Scales everything drawn afterwards.
pub fn scale(sx: f32, sy: f32) {
    context::with(|ctx| ctx.gfx.transform.scale(sx, sy));
}

/// Resets the transform so positions are plain window pixels again.
pub fn origin() {
    context::with(|ctx| ctx.gfx.transform = Transform::IDENTITY);
}

/// Shears everything drawn afterwards: x gains `kx` times y, and y gains
/// `ky` times x.
pub fn shear(shear: impl Into<Vec2>) {
    let Vec2 { x: kx, y: ky } = shear.into();
    context::with(|ctx| ctx.gfx.transform.shear((kx, ky)));
}

/// Applies `transform` on top of the current transform, as if its calls
/// had been made here.
pub fn apply_transform(transform: &Transform) {
    context::with(|ctx| ctx.gfx.transform.apply_transform(transform));
}

/// Replaces the current transform with `transform`.
pub fn replace_transform(transform: &Transform) {
    context::with(|ctx| ctx.gfx.transform = *transform);
}

/// The current transform, as set by [`translate`], [`apply_transform`]
/// and the rest. [`push`] and [`pop`] save and restore it.
pub fn transform() -> Transform {
    context::with(|ctx| ctx.gfx.transform)
}

/// Where the point `(x, y)` lands on the screen, in units, under the
/// current transform.
pub fn transform_point(position: impl Into<Vec2>) -> Vec2 {
    let Vec2 { x, y } = position.into();
    context::with(|ctx| ctx.gfx.transform.transform_point((x, y)))
}

/// Which point the current transform puts at `(x, y)` on the screen, such
/// as where the mouse is in a scrolled and zoomed world.
pub fn inverse_transform_point(position: impl Into<Vec2>) -> Vec2 {
    let Vec2 { x, y } = position.into();
    context::with(|ctx| ctx.gfx.transform.inverse_transform_point((x, y)))
}

// ---------------------------------------------------------------------------
// Shapes

/// Draws a rectangle with its top-left corner at `(x, y)`.
pub fn rectangle(mode: DrawMode, position: impl Into<Vec2>, size: impl Into<Vec2>) {
    let Vec2 { x, y } = position.into();
    let Vec2 {
        x: width,
        y: height,
    } = size.into();
    polygon(
        mode,
        &[
            (x, y),
            (x + width, y),
            (x + width, y + height),
            (x, y + height),
        ],
    );
}

/// Draws a circle centered on `(x, y)`.
pub fn circle(mode: DrawMode, center: impl Into<Vec2>, radius: f32) {
    let Vec2 { x, y } = center.into();
    // More pixels per unit need more corners to look round.
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(mode, &shapes::circle_points(x, y, radius, scale));
}

/// Draws a polygon through `points`. Filling only works for convex polygons.
///
/// Under a custom shader, a filled shape's `uv` runs from `(0, 0)` at the
/// top-left of the box around the shape to `(1, 1)` at its bottom-right,
/// as it does across an image. So a shader can paint a rectangle, circle
/// or any other filled shape with no image at all. This holds for every
/// filled shape, not only polygons.
pub fn polygon(mode: DrawMode, points: &[impl Into<Vec2> + Copy]) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let triangles = match mode {
            DrawMode::Fill => shapes::fill_convex(points),
            DrawMode::Line => {
                let triangles = shapes::stroke(points, true, &ctx.gfx.stroke());
                return ctx.gfx.push_covered(&triangles);
            }
        };
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws a rectangle with rounded corners. `rx` and `ry` are the corner
/// radii along each axis, as in the long form of `love.graphics.rectangle`.
pub fn rounded_rectangle(
    mode: DrawMode,
    position: impl Into<Vec2>,
    size: impl Into<Vec2>,
    radii: impl Into<Vec2>,
) {
    let Vec2 { x, y } = position.into();
    let Vec2 {
        x: width,
        y: height,
    } = size.into();
    let Vec2 { x: rx, y: ry } = radii.into();
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(
        mode,
        &shapes::rounded_rectangle_points(x, y, width, height, rx, ry, scale),
    );
}

/// Draws an ellipse centered on `(x, y)` with radii `rx` and `ry`.
pub fn ellipse(mode: DrawMode, center: impl Into<Vec2>, radii: impl Into<Vec2>) {
    let Vec2 { x, y } = center.into();
    let Vec2 { x: rx, y: ry } = radii.into();
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(mode, &shapes::ellipse_points(x, y, rx, ry, scale));
}

/// Draws part of a circle centered on `(x, y)`, from `angle1` to `angle2`
/// in radians. Angles grow clockwise from the right, as in LÖVE.
pub fn arc(
    mode: DrawMode,
    arc_type: ArcType,
    center: impl Into<Vec2>,
    radius: f32,
    angle1: f32,
    angle2: f32,
) {
    let Vec2 { x, y } = center.into();
    context::with(|ctx| {
        let scale = ctx.gfx.target_scale();
        let mut path = shapes::arc_points(x, y, radius, radius, angle1, angle2, scale);
        if arc_type == ArcType::Pie {
            // The center comes first so that filling fans out from it.
            path.insert(0, (x, y));
        }
        match mode {
            DrawMode::Fill => ctx.gfx.push_shape(&shapes::fill_convex(&path)),
            DrawMode::Line => {
                let closed = arc_type != ArcType::Open;
                let triangles = shapes::stroke(&path, closed, &ctx.gfx.stroke());
                ctx.gfx.push_covered(&triangles);
            }
        }
    });
}

/// Draws a square of the current point size centered on each point.
pub fn points(points: &[impl Into<Vec2> + Copy]) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let half = ctx.gfx.point_size / 2.0;
        let mut triangles = Vec::with_capacity(points.len() * 6);
        for &(x, y) in points {
            let (left, top, right, bottom) = (x - half, y - half, x + half, y + half);
            triangles.extend_from_slice(&[
                (left, top),
                (right, top),
                (right, bottom),
                (left, top),
                (right, bottom),
                (left, bottom),
            ]);
        }
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws the shadow that a convex shape casts away from a light at
/// `(light_x, light_y)`, in the current color: every edge of `polygon`
/// that faces away from the light, stretched `length` further from it.
///
/// The shape itself is not covered, as if light falls on its top. The
/// corners may be listed clockwise or counter-clockwise. A `length`
/// longer than the screen is wide gives shadows that run off its edge.
///
/// For lighting, draw the shadows of everything solid inside
/// [`stencil`], then draw the light's glow only where the stencil is
/// still zero. The `lighting` example does exactly this.
///
/// LÖVE has no such call; games there work the corners out by hand.
pub fn shadow(light: impl Into<Vec2>, polygon: &[impl Into<Vec2> + Copy], length: f32) {
    let polygon = crate::math::tuples(polygon);
    let polygon = &polygon[..];
    let Vec2 {
        x: light_x,
        y: light_y,
    } = light.into();
    context::with(|ctx| {
        let triangles = shapes::shadow((light_x, light_y), polygon, length);
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws connected lines through `points` in order, with joined corners.
/// The equivalent of `love.graphics.line` with more than two points.
pub fn polyline(points: &[impl Into<Vec2> + Copy]) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let triangles = shapes::stroke(points, false, &ctx.gfx.stroke());
        ctx.gfx.push_covered(&triangles);
    });
}

/// Draws a dashed line through `points` in order: dashes `dash` long
/// with gaps of `gap` between them, both measured along the line. The
/// dashes use the current line width, join and style.
///
/// LÖVE has no dashed lines; this saves working the pieces out by hand.
pub fn dashed_polyline(points: &[impl Into<Vec2> + Copy], dash: f32, gap: f32) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let stroke = ctx.gfx.stroke();
        let mut triangles = Vec::new();
        for piece in shapes::dashes(points, dash, gap) {
            triangles.extend(shapes::stroke(&piece, false, &stroke));
        }
        ctx.gfx.push_covered(&triangles);
    });
}

/// Draws a straight dashed line between two points. See
/// [`dashed_polyline`].
pub fn dashed_line(from: impl Into<Vec2>, to: impl Into<Vec2>, dash: f32, gap: f32) {
    let Vec2 { x: x1, y: y1 } = from.into();
    let Vec2 { x: x2, y: y2 } = to.into();
    dashed_polyline(&[(x1, y1), (x2, y2)], dash, gap);
}

/// Draws a straight line between two points.
pub fn line(from: impl Into<Vec2>, to: impl Into<Vec2>) {
    let Vec2 { x: x1, y: y1 } = from.into();
    let Vec2 { x: x2, y: y2 } = to.into();
    context::with(|ctx| {
        let triangles = shapes::stroke(&[(x1, y1), (x2, y2)], false, &ctx.gfx.stroke());
        ctx.gfx.push_covered(&triangles);
    });
}

// ---------------------------------------------------------------------------
// Images

impl Image {
    /// Loads an image file: PNG, JPEG or TGA, or DDS or KTX for compressed
    /// pixels.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes, which
    /// normally means inside the game's `assets` folder.
    pub fn load(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        image::load(path.as_ref(), ImageSettings::default())
    }
}

impl Image {
    /// Loads an image file with options such as mipmaps, the equivalent of
    /// `love.graphics.newImage(path, settings)`.
    pub fn load_with(path: impl AsRef<Path>, settings: ImageSettings) -> Result<Image, ImageError> {
        image::load(path.as_ref(), settings)
    }
}

impl ImageData {
    /// Loads an image file into memory, where its pixels can be read and
    /// changed. See [`ImageData`].
    pub fn load(path: impl AsRef<Path>) -> Result<ImageData, ImageError> {
        let (width, height, rgba) = image::load_rgba(path.as_ref())?;
        Ok(ImageData::from_rgba(width, height, rgba))
    }
}

/// Draws `item` placed by `params`, the equivalent of
/// `love.graphics.draw`. A position is enough for `params`, and
/// [`DrawParams`] adds rotation, scale, an origin, shear and a
/// [`Transform`].
///
/// `item` is any [`Drawable`]: an [`Image`] or [`Canvas`], part of one
/// from [`Image::region`], a layer from [`ArrayImage::layer`], a
/// [`SpriteBatch`], [`Mesh`], [`ParticleSystem`], [`Text`] or video.
///
/// ```no_run
/// use smitten::graphics::{self, DrawParams, Image, Quad};
///
/// # fn demo(sheet: &Image) {
/// graphics::draw(sheet, (10.0, 10.0));
/// let frame = Quad::new((0.0, 0.0), (16.0, 16.0), (64.0, 64.0));
/// graphics::draw(&sheet.region(&frame), DrawParams::at((40.0, 10.0)).scale(4.0));
/// # }
/// ```
#[doc(alias = "drawLayer")]
pub fn draw(item: &impl Drawable, params: impl Into<DrawParams>) {
    item.draw_transformed(&params.into().to_transform());
}

impl draw::sealed::Sealed for Image {
    fn draw_transformed(&self, local: &Transform) {
        let quad = whole_image(self);
        context::with(|ctx| ctx.gfx.push_image(self, &quad, local));
    }
}
impl Drawable for Image {}

impl draw::sealed::Sealed for Canvas {
    fn draw_transformed(&self, local: &Transform) {
        let image: &Image = self;
        draw::sealed::Sealed::draw_transformed(image, local);
    }
}
impl Drawable for Canvas {}

impl draw::sealed::Sealed for ImageRegion<'_> {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_image(self.image, self.quad, local));
    }
}
impl Drawable for ImageRegion<'_> {}

#[cfg(feature = "video")]
impl draw::sealed::Sealed for Video {
    /// Drawing moves the picture on to the video's position.
    fn draw_transformed(&self, local: &Transform) {
        draw::sealed::Sealed::draw_transformed(self.image(), local);
    }
}
#[cfg(feature = "video")]
impl Drawable for Video {}

impl draw::sealed::Sealed for SpriteBatch {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_sprite_batch(&mut *ctx.backend, self, local));
    }
}
impl Drawable for SpriteBatch {}

impl draw::sealed::Sealed for Mesh {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_mesh(&mut *ctx.backend, self, local, 1));
    }
}
impl Drawable for Mesh {}

impl draw::sealed::Sealed for ParticleSystem {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_particles(self, local));
    }
}
impl Drawable for ParticleSystem {}

impl draw::sealed::Sealed for Text {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_text_object(ctx.backend.as_mut(), self, local));
    }
}
impl Drawable for Text {}

/// Draws `instances` copies of a mesh at once, placed by `params`, as
/// LÖVE's `drawInstanced`. Each copy is drawn in the same place, so give
/// them something to tell them apart: attributes attached with
/// [`VertexAttributeStep::PerInstance`] through
/// [`Mesh::attach_attribute_with`], which advance once per copy, or
/// `instance_index` in a [`Shader`]'s `position`, which counts the copies
/// from 0.
///
/// Panics if a per-instance attribute's mesh has fewer vertices than
/// `instances`. A mesh drawn as [`MeshDrawMode::Points`] is drawn once.
pub fn draw_instanced(mesh: &Mesh, instances: u32, params: impl Into<DrawParams>) {
    let local = params.into().to_transform();
    context::with(|ctx| {
        ctx.gfx
            .push_mesh(&mut *ctx.backend, mesh, &local, instances)
    });
}

// ---------------------------------------------------------------------------
// Text

impl Font {
    /// Loads a TrueType or OpenType font file at `size` pixels, or an
    /// AngelCode BMFont if the file name ends in `.fnt` (whose own size is
    /// used then, and whose page images are found next to it).
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes, which
    /// normally means inside the game's `assets` folder.
    pub fn load(path: impl AsRef<Path>, size: f32) -> Result<Font, FontError> {
        Font::load_with(path, size, FontSettings::default())
    }
}

impl Font {
    /// Loads a font file with the full set of LÖVE's `newFont` arguments:
    /// its size, and how its outlines are fitted to pixels and how sharp
    /// its glyphs are, as [`FontSettings`] says.
    pub fn load_with(
        path: impl AsRef<Path>,
        size: f32,
        settings: FontSettings,
    ) -> Result<Font, FontError> {
        let path = path.as_ref();
        let is_bmfont = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fnt"));
        let font = if is_bmfont {
            Font::read_bmfont(path)?
        } else {
            let bytes =
                crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
            Font::from_bytes_with(&bytes, size, settings)?
        };
        font.watch(path);
        Ok(font)
    }
}

impl Font {
    /// Loads an AngelCode BMFont from its `.fnt` file, whose page images are
    /// found next to it. Only the text form of `.fnt` files is read, as in
    /// LÖVE. [`Font::load`] does the same for a file ending in `.fnt`.
    pub fn load_bmfont(path: impl AsRef<Path>) -> Result<Font, FontError> {
        let path = path.as_ref();
        let font = Font::read_bmfont(path)?;
        font.watch(path);
        Ok(font)
    }
}

impl Font {
    /// Loads a bitmap font: an image holding the characters of `glyphs` in a
    /// row, separated by columns in the color of its top-left pixel. The
    /// equivalent of `love.graphics.newImageFont`. See [`Font::from_image`].
    ///
    /// Pixel fonts usually want `font.set_filter(Filter::Nearest)` so they
    /// stay crisp when enlarged.
    pub fn load_image_font(path: impl AsRef<Path>, glyphs: &str) -> Result<Font, FontError> {
        Font::load_image_font_with(path, glyphs, 0.0)
    }
}

impl Font {
    /// Loads a bitmap font with `extra_spacing` units added after each
    /// character, which may be negative: LÖVE's `newImageFont` with its
    /// `extraspacing` argument.
    pub fn load_image_font_with(
        path: impl AsRef<Path>,
        glyphs: &str,
        extra_spacing: f32,
    ) -> Result<Font, FontError> {
        let path = path.as_ref();
        let bytes =
            crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
        let font =
            Font::from_image(&bytes, glyphs, extra_spacing).map_err(|error| match error {
                FontError::Parse(message) => {
                    FontError::Parse(format!("{}: {message}", path.display()))
                }
                other => other,
            })?;
        font.watch(path);
        Ok(font)
    }
}

/// Sets the font used by [`print()`] and [`printf`].
pub fn set_font(font: &Font) {
    context::with(|ctx| ctx.gfx.font = Some(font.clone()));
}

/// Returns the current font. Until one is set, this is the built-in font
/// at 12 pixels, as in LÖVE.
pub fn font() -> Font {
    context::with(|ctx| {
        let (default_filter, scale) = (ctx.gfx.default_filter, ctx.gfx.screen.draw_scale());
        ctx.gfx
            .font
            .get_or_insert_with(|| Font::fallback(default_filter, scale))
            .clone()
    })
}

/// Draws text in the current font and color, placed by `params`, the
/// equivalent of `love.graphics.print`. Line breaks in the text start new
/// lines. The text can be a string or colored parts, as [`TextSpans`]
/// describes.
pub fn print<'a>(text: impl Into<TextSpans<'a>>, params: impl Into<DrawParams>) {
    let local = params.into().to_transform();
    text.into().with_segments(|segments| {
        context::with(|ctx| {
            ctx.gfx
                .push_colored_text(ctx.backend.as_mut(), segments, None, &local);
        });
    });
}

/// Draws text wrapped to `limit` pixels wide and aligned within that
/// width, placed by `params`, the equivalent of `love.graphics.printf`.
pub fn printf<'a>(
    text: impl Into<TextSpans<'a>>,
    params: impl Into<DrawParams>,
    limit: f32,
    align: Align,
) {
    let local = params.into().to_transform();
    text.into().with_segments(|segments| {
        context::with(|ctx| {
            ctx.gfx
                .push_colored_text(ctx.backend.as_mut(), segments, Some((limit, align)), &local);
        });
    });
}

/// One layer of an [`ArrayImage`] or array [`Canvas`], drawn by [`draw`]
/// as LÖVE's `drawLayer` does. Made with [`ArrayImage::layer`] or
/// [`Canvas::layer`].
///
/// Drawing panics if a custom shader is active: a shader reads an array
/// image by declaring a `texture_2d_array<f32>` and being sent it with
/// [`Shader::send_texture`].
#[derive(Debug, Clone, Copy)]
pub struct Layer<'a> {
    texture: TextureId,
    filter: Filter,
    layer: u32,
    quad: Option<&'a Quad>,
    size: (u32, u32),
}

impl<'a> Layer<'a> {
    fn of(image: &impl Layered, layer: u32) -> Self {
        assert!(
            layer < image.layers(),
            "layer {layer} does not exist: there are {} layers",
            image.layers()
        );
        Layer {
            texture: image.layer_texture(),
            filter: image.layer_filter(),
            layer,
            quad: None,
            size: image.layer_size(),
        }
    }

    /// Just the part of the layer that `quad` describes.
    #[must_use]
    pub fn region(self, quad: &'a Quad) -> Layer<'a> {
        Layer {
            quad: Some(quad),
            ..self
        }
    }
}

impl ArrayImage {
    /// Layer `layer` of the image, counting from 0, to draw with [`draw`].
    ///
    /// Panics if there is no such layer.
    #[doc(alias = "drawLayer")]
    pub fn layer(&self, layer: u32) -> Layer<'_> {
        Layer::of(self, layer)
    }
}

impl Canvas {
    /// Layer `layer` of an array canvas, counting from 0, to draw with
    /// [`draw`].
    ///
    /// Panics if the canvas was not made with [`TextureKind::Array`], or
    /// there is no such layer.
    pub fn layer(&self, layer: u32) -> Layer<'_> {
        Layer::of(self, layer)
    }
}

impl draw::sealed::Sealed for Layer<'_> {
    fn draw_transformed(&self, local: &Transform) {
        let whole;
        let quad = match self.quad {
            Some(quad) => quad,
            None => {
                let (width, height) = (self.size.0 as f32, self.size.1 as f32);
                whole = Quad::new((0.0, 0.0), (width, height), (width, height));
                &whole
            }
        };
        context::with(|ctx| {
            assert!(
                ctx.gfx.shader.is_none(),
                "an array image layer cannot be drawn with a custom shader active; \
                 send the array image to the shader with Shader::send_texture instead"
            );
            ctx.gfx
                .push_texture(self.texture, self.layer, self.filter, quad, local);
        });
    }
}
impl Drawable for Layer<'_> {}

/// Something with layers that [`Layer`] can draw one of: an
/// [`ArrayImage`], or a [`Canvas`] made with [`TextureKind::Array`].
trait Layered {
    fn layer_texture(&self) -> TextureId;
    fn layer_filter(&self) -> Filter;
    fn layer_size(&self) -> (u32, u32);
    fn layers(&self) -> u32;
}

impl Layered for ArrayImage {
    fn layer_texture(&self) -> TextureId {
        self.texture()
    }
    fn layer_filter(&self) -> Filter {
        self.filter()
    }
    fn layer_size(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
    fn layers(&self) -> u32 {
        self.layer_count()
    }
}

impl Layered for Canvas {
    fn layer_texture(&self) -> TextureId {
        assert!(
            self.texture_type() == TextureKind::Array,
            "Canvas::layer needs an array canvas, not a {:?} one",
            self.texture_type()
        );
        self.texture()
    }
    fn layer_filter(&self) -> Filter {
        self.filter()
    }
    fn layer_size(&self) -> (u32, u32) {
        self.dimensions()
    }
    fn layers(&self) -> u32 {
        self.layer_count()
    }
}

/// Whether the GPU can draw images compressed in `format`, the
/// equivalent of `love.graphics.getImageFormats`.
pub fn is_compressed_format_supported(format: CompressedFormat) -> bool {
    context::with(|ctx| ctx.backend.supports_compressed_format(format))
}

/// Whether the GPU can draw images in `format`, made with
/// [`Image::from_image_data`]. The uncompressed half of
/// `love.graphics.getImageFormats`.
pub fn is_image_format_supported(format: PixelFormat) -> bool {
    format.is_color() && context::with(|ctx| ctx.backend.supports_pixel_format(format, false))
}

/// The uncompressed formats images can have on this GPU, in the order
/// [`PixelFormat::ALL`] lists them.
pub fn image_formats() -> Vec<PixelFormat> {
    PixelFormat::ALL
        .into_iter()
        .filter(|&format| is_image_format_supported(format))
        .collect()
}

/// Whether a canvas can be made in `format` on this GPU, the equivalent
/// of looking `format` up in `love.graphics.getCanvasFormats`. With
/// `readable`, it must also be drawable and readable by shaders, as a
/// canvas usually is.
///
/// A depth canvas is readable when shaders can sample it; LÖVE's depth
/// canvases are not by default.
pub fn is_canvas_format_supported(format: PixelFormat, readable: bool) -> bool {
    context::with(|ctx| {
        ctx.backend.supports_pixel_format(format, true)
            && (!readable || ctx.backend.supports_pixel_format(format, false))
    })
}

/// The texture shapes this GPU supports, the equivalent of
/// `love.graphics.getTextureTypes`. Every SDL GPU backend supports all
/// four, for images and canvases alike.
pub fn texture_types() -> [(TextureKind, bool); 4] {
    [
        (TextureKind::Flat, true),
        (TextureKind::Array, true),
        (TextureKind::Cube, true),
        (TextureKind::Volume, true),
    ]
}

/// The formats canvases can have on this GPU, the equivalent of
/// `love.graphics.getCanvasFormats(readable)`, in the order
/// [`PixelFormat::ALL`] lists them.
pub fn canvas_formats(readable: bool) -> Vec<PixelFormat> {
    PixelFormat::ALL
        .into_iter()
        .filter(|&format| is_canvas_format_supported(format, readable))
        .collect()
}

/// An optional graphics feature, the equivalent of LÖVE's
/// `GraphicsFeature`. Ask about one with [`is_supported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GraphicsFeature {
    /// [`Wrap::ClampZero`](crate::Wrap::ClampZero) as its own mode, rather
    /// than falling back to [`Wrap::Clamp`](crate::Wrap::Clamp).
    ClampZero,
    /// The [`BlendMode::Lighten`] and
    /// [`BlendMode::Darken`] blend modes.
    Lighten,
    /// Drawing to several canvases at once that have different formats.
    MultiCanvasFormats,
    /// Textures whose sides are not powers of two, with mipmaps and every
    /// wrap mode.
    FullNpot,
    /// Full-precision floats in fragment shaders.
    PixelShaderHighp,
    /// The derivative functions in shaders, such as `dpdx`.
    ShaderDerivatives,
    /// What LÖVE allows only with GLSL 3: integer types in shaders and
    /// between [`Shader`] stages, sampling textures in `position`, and
    /// `instance_index`. WGSL always has them.
    Glsl3,
    /// Drawing many copies of a mesh at once with [`draw_instanced`].
    Instancing,
}

/// Whether the GPU supports `feature`, the equivalent of the table
/// `love.graphics.getSupported` returns.
pub fn is_supported(feature: GraphicsFeature) -> bool {
    match feature {
        // SDL's GPU API has no border colors to build it from.
        GraphicsFeature::ClampZero => false,
        // Every GPU SDL's GPU API runs on has these.
        GraphicsFeature::MultiCanvasFormats
        | GraphicsFeature::Lighten
        | GraphicsFeature::FullNpot
        | GraphicsFeature::PixelShaderHighp
        | GraphicsFeature::ShaderDerivatives
        | GraphicsFeature::Glsl3
        | GraphicsFeature::Instancing => true,
    }
}

/// The largest sizes and counts the GPU allows, the equivalent of the
/// table `love.graphics.getSystemLimits` returns. See [`system_limits`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct SystemLimits {
    /// The widest point [`set_point_size`] can make. Points are drawn as
    /// squares, so there is no limit.
    pub point_size: f32,
    /// The widest and tallest an [`Image`] or [`Canvas`] can be.
    pub texture_size: u32,
    /// The widest, tallest and deepest a [`VolumeImage`] can be.
    pub volume_texture_size: u32,
    /// The widest and tallest each face of a [`CubeImage`] can be.
    pub cube_texture_size: u32,
    /// The most layers an [`ArrayImage`] can have.
    pub texture_layers: u32,
    /// The most canvases that can be drawn to at once with
    /// [`set_canvases`].
    pub multi_canvas: u32,
    /// The most samples per pixel a multisampled canvas can have, 1 if
    /// there is no multisampling.
    pub canvas_msaa: u32,
    /// The most anisotropic filtering a texture can have. Smitten does
    /// not filter anisotropically yet, so this is 1.
    pub anisotropy: f32,
}

/// The largest sizes and counts the GPU allows, the equivalent of
/// `love.graphics.getSystemLimits`.
///
/// SDL's GPU API does not report most limits, so the texture sizes are
/// the ones every GPU it supports on desktops has: Direct3D 12 feature
/// level 11_0 promises them, and Vulkan and Metal desktop GPUs match. A
/// phone's GPU may allow less. `canvas_msaa` is asked of the GPU.
pub fn system_limits() -> SystemLimits {
    SystemLimits {
        point_size: f32::INFINITY,
        texture_size: 16384,
        volume_texture_size: 2048,
        cube_texture_size: 16384,
        texture_layers: 2048,
        multi_canvas: MAX_CANVASES as u32,
        canvas_msaa: context::with(|ctx| ctx.backend.max_msaa()),
        anisotropy: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::intersect;

    #[test]
    fn scissors_intersect_to_their_overlap() {
        assert_eq!(
            intersect((0, 0, 100, 100), (50, 20, 100, 30)),
            (50, 20, 50, 30)
        );
        assert_eq!(
            intersect((10, 10, 20, 20), (0, 0, 100, 100)),
            (10, 10, 20, 20)
        );
    }

    #[test]
    fn scissors_that_do_not_meet_leave_nothing() {
        let (_, _, width, height) = intersect((0, 0, 10, 10), (20, 20, 10, 10));
        assert_eq!((width, height), (0, 0));
    }
}

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
mod frame;
pub(crate) mod image;
mod image_data;
mod images;
mod layered;
mod mesh;
mod particles;
mod pixels;
mod printing;
mod screen;
mod settings;
mod shader;
mod shading;
mod shape_drawing;
mod shapes;
mod sprite_batch;
mod state;
mod stats;
mod stenciling;
mod targets;
pub(crate) mod text;
mod text_object;
mod transform;
mod transform_stack;
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
pub use frame::*;
pub use image::{Image, ImageError, ImageSettings, Quad};
pub use image_data::{ImageData, ImageFormat, is_compressed};
pub use images::*;
pub use layered::{ArrayImage, CubeImage, VolumeImage};
pub use mesh::{Mesh, MeshDrawMode, MeshVertex, VertexAttribute};
pub use particles::{AreaSpreadDistribution, EmissionArea, ParticleInsertMode, ParticleSystem};
pub use printing::*;
pub use settings::*;
pub use shader::{MatrixLayout, Shader, ShaderError, Texture, UniformValue};
pub use shading::*;
pub use shape_drawing::*;
pub use shapes::{LineJoin, LineStyle};
pub use smitten_core::backend::{
    CompressedFormat, IndexDataType, RendererInfo, TextureKind, VertexDataType,
    VertexStep as VertexAttributeStep,
};
pub use sprite_batch::{BatchTexture, SpriteBatch, SpriteBatchUsage};
use state::StackType;
pub use stats::Stats;
pub use stenciling::*;
pub use targets::*;
pub use text::{Align, Font, FontError, FontSettings, Hinting};
pub use text_object::Text;
pub use transform::Transform;
pub use transform_stack::*;
#[cfg(feature = "video")]
pub use video::Video;

/// Whether a shape is filled in or drawn as an outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawMode {
    /// Filled in.
    Fill,
    /// Only the outline, as wide as the line width.
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
            let (texture, filter) = sprites.gpu_texture();
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
        let (texture, filter) = sprites.gpu_texture();
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

#[cfg(test)]
mod tests {
    use super::settings::intersect;

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

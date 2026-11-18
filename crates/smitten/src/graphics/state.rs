//! Saving and restoring the drawing state, for `push_all`, `pop` and
//! `reset`.

use smitten_core::backend::ColorMask;
use smitten_core::{DepthMode, StencilMode};

use super::{CanvasList, Font, GraphicsState, LineJoin, LineStyle, Shader, Transform};
use crate::{BlendAlphaMode, BlendMode, Color, CullMode, Filter, VertexWinding};

/// What a push saves, for the matching pop to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum StackType {
    /// Only the transform, as [`push`](super::push) saves.
    #[default]
    Transform,
    /// The transform and every other drawing setting, as
    /// [`push_all`](super::push_all) saves.
    All,
}

/// One entry of the stack that `push` adds to and `pop` takes from.
pub(super) enum Saved {
    Transform(Transform),
    All(Box<DrawState>),
}

/// Everything `push_all` saves: the same settings as LÖVE's
/// `push("all")`.
pub(super) struct DrawState {
    transform: Transform,
    color: Color,
    background: Color,
    font: Option<Font>,
    blend: BlendMode,
    blend_alpha: BlendAlphaMode,
    shader: Option<Shader>,
    canvases: CanvasList,
    scissor: Option<(i32, i32, u32, u32)>,
    stencil: StencilMode,
    stencil_value: u8,
    line_width: f32,
    line_join: LineJoin,
    line_style: LineStyle,
    point_size: f32,
    color_mask: ColorMask,
    depth: DepthMode,
    wireframe: bool,
    default_filter: Filter,
    mesh_cull: CullMode,
    front_face: VertexWinding,
}

impl DrawState {
    /// The settings a game starts with, which `reset` goes back to.
    fn initial() -> DrawState {
        DrawState {
            transform: Transform::IDENTITY,
            color: Color::WHITE,
            background: Color::BLACK,
            font: None,
            blend: BlendMode::default(),
            blend_alpha: BlendAlphaMode::default(),
            shader: None,
            canvases: CanvasList::default(),
            scissor: None,
            stencil: StencilMode::Off,
            stencil_value: 0,
            line_width: 1.0,
            line_join: LineJoin::default(),
            line_style: LineStyle::default(),
            point_size: 1.0,
            color_mask: ColorMask::ALL,
            depth: DepthMode::default(),
            wireframe: false,
            default_filter: Filter::default(),
            mesh_cull: CullMode::default(),
            front_face: VertexWinding::default(),
        }
    }
}

impl GraphicsState {
    pub(super) fn save_state(&self) -> DrawState {
        DrawState {
            transform: self.transform,
            color: self.color,
            background: self.background,
            font: self.font.clone(),
            blend: self.batch.blend,
            blend_alpha: self.batch.blend_alpha,
            shader: self.shader.clone(),
            canvases: self.canvas_list(),
            scissor: self.scissor,
            stencil: self.batch.stencil,
            stencil_value: self.batch.stencil_value,
            line_width: self.line_width,
            line_join: self.line_join,
            line_style: self.line_style,
            point_size: self.point_size,
            color_mask: self.batch.color_mask,
            depth: self.batch.depth,
            wireframe: self.batch.wireframe,
            default_filter: self.default_filter,
            mesh_cull: self.mesh_cull,
            front_face: self.front_face,
        }
    }

    pub(super) fn restore_state(&mut self, state: DrawState) {
        self.transform = state.transform;
        self.color = state.color;
        self.background = state.background;
        // A font that was never chosen is made on first use, so keep the
        // one made since rather than making another.
        if state.font.is_some() {
            self.font = state.font;
        }
        self.batch.blend = state.blend;
        self.batch.blend_alpha = state.blend_alpha;
        let shader_changed = match (&self.shader, &state.shader) {
            (Some(now), Some(then)) => !now.is(then),
            (None, None) => false,
            _ => true,
        };
        if shader_changed {
            self.set_shader(state.shader);
        }
        self.scissor = state.scissor;
        // Also puts the scissor back, in pixels of the restored target.
        self.set_canvas_list(state.canvases);
        self.batch.stencil = state.stencil;
        self.batch.stencil_value = state.stencil_value;
        self.line_width = state.line_width;
        self.line_join = state.line_join;
        self.line_style = state.line_style;
        self.point_size = state.point_size;
        self.batch.color_mask = state.color_mask;
        self.batch.depth = state.depth;
        self.batch.wireframe = state.wireframe;
        self.default_filter = state.default_filter;
        self.mesh_cull = state.mesh_cull;
        self.front_face = state.front_face;
    }

    pub(super) fn push_state(&mut self, kind: StackType) {
        let saved = match kind {
            StackType::Transform => Saved::Transform(self.transform),
            StackType::All => Saved::All(Box::new(self.save_state())),
        };
        self.stack.push(saved);
    }

    pub(super) fn pop_state(&mut self) {
        match self
            .stack
            .pop()
            .expect("graphics::pop was called without a matching graphics::push")
        {
            Saved::Transform(transform) => self.transform = transform,
            Saved::All(state) => self.restore_state(*state),
        }
    }

    /// Puts every drawing setting back to how a game starts, as LÖVE's
    /// `reset`. The font goes back to the built-in one.
    pub(super) fn reset_state(&mut self) {
        self.font = None;
        self.restore_state(DrawState::initial());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompareMode;

    #[test]
    fn push_saves_only_the_transform() {
        let mut gfx = GraphicsState::default();
        gfx.push_state(StackType::Transform);
        gfx.transform.translate((10.0, 0.0));
        gfx.color = Color::rgb(1.0, 0.0, 0.0);
        gfx.pop_state();
        assert_eq!(gfx.transform, Transform::IDENTITY);
        assert_eq!(gfx.color, Color::rgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn push_all_saves_every_setting() {
        let mut gfx = GraphicsState::default();
        gfx.push_state(StackType::All);
        gfx.transform.translate((10.0, 0.0));
        gfx.color = Color::rgb(1.0, 0.0, 0.0);
        gfx.background = Color::WHITE;
        gfx.batch.blend = BlendMode::Add;
        gfx.batch.blend_alpha = BlendAlphaMode::Premultiplied;
        gfx.scissor = Some((1, 2, 3, 4));
        gfx.batch.stencil = StencilMode::Test(CompareMode::Greater);
        gfx.batch.stencil_value = 3;
        gfx.line_width = 4.0;
        gfx.line_join = LineJoin::Bevel;
        gfx.line_style = LineStyle::Rough;
        gfx.point_size = 5.0;
        gfx.batch.color_mask = ColorMask {
            r: false,
            g: true,
            b: true,
            a: true,
        };
        gfx.batch.wireframe = true;
        gfx.batch.depth = DepthMode {
            compare: CompareMode::Less,
            write: true,
        };
        gfx.default_filter = Filter::Nearest;
        gfx.mesh_cull = CullMode::Back;
        gfx.front_face = VertexWinding::Clockwise;
        gfx.pop_state();

        assert_eq!(gfx.transform, Transform::IDENTITY);
        assert_eq!(gfx.color, Color::WHITE);
        assert_eq!(gfx.background, Color::BLACK);
        assert_eq!(gfx.batch.blend, BlendMode::Alpha);
        assert_eq!(gfx.batch.blend_alpha, BlendAlphaMode::AlphaMultiply);
        assert_eq!(gfx.scissor, None);
        assert_eq!(gfx.batch.scissor, None);
        assert_eq!(gfx.batch.stencil, StencilMode::Off);
        assert_eq!(gfx.line_width, 1.0);
        assert_eq!(gfx.line_join, LineJoin::default());
        assert_eq!(gfx.line_style, LineStyle::default());
        assert_eq!(gfx.point_size, 1.0);
        assert_eq!(gfx.batch.color_mask, ColorMask::ALL);
        assert_eq!(gfx.batch.depth, DepthMode::default());
        assert!(!gfx.batch.wireframe);
        assert_eq!(gfx.default_filter, Filter::Linear);
        assert_eq!(gfx.mesh_cull, CullMode::None);
        assert_eq!(gfx.front_face, VertexWinding::CounterClockwise);
    }

    #[test]
    fn pushes_of_both_kinds_nest() {
        let mut gfx = GraphicsState::default();
        gfx.push_state(StackType::All);
        gfx.color = Color::rgb(0.0, 1.0, 0.0);
        gfx.push_state(StackType::Transform);
        gfx.color = Color::rgb(0.0, 0.0, 1.0);
        gfx.transform.scale(2.0, 2.0);
        assert_eq!(gfx.stack.len(), 2);

        gfx.pop_state();
        assert_eq!(gfx.transform, Transform::IDENTITY);
        assert_eq!(gfx.color, Color::rgb(0.0, 0.0, 1.0));
        gfx.pop_state();
        assert_eq!(gfx.color, Color::WHITE);
        assert!(gfx.stack.is_empty());
    }

    #[test]
    fn reset_goes_back_to_the_defaults_and_keeps_the_stack() {
        let mut gfx = GraphicsState::default();
        gfx.push_state(StackType::Transform);
        gfx.color = Color::rgb(1.0, 0.0, 0.0);
        gfx.batch.blend = BlendMode::Multiply;
        gfx.transform.rotate(1.0);
        gfx.line_width = 3.0;
        gfx.reset_state();
        assert_eq!(gfx.color, Color::WHITE);
        assert_eq!(gfx.batch.blend, BlendMode::Alpha);
        assert_eq!(gfx.transform, Transform::IDENTITY);
        assert_eq!(gfx.line_width, 1.0);
        assert_eq!(gfx.stack.len(), 1);
    }

    #[test]
    #[should_panic(expected = "without a matching graphics::push")]
    fn pop_without_push_panics() {
        GraphicsState::default().pop_state();
    }
}

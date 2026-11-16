//! Collects a frame's triangles and groups them into as few draw calls as
//! the draw order allows.

use smitten_core::backend::{DrawCall, Pass, Scissor, ShaderId, TextureId, Vertex};
use smitten_core::{BlendMode, Color, Filter, StencilMode};

#[derive(Default)]
pub(crate) struct Batch {
    pub vertices: Vec<Vertex>,
    pub calls: Vec<DrawCall>,
    pub passes: Vec<Pass>,
    /// Where draws go: a canvas's texture, or `None` for the screen.
    target: Option<TextureId>,
    /// Applied to everything queued from now on.
    pub blend: BlendMode,
    pub scissor: Option<Scissor>,
    pub stencil: StencilMode,
    pub stencil_value: u8,
    /// Uniform data for the queued calls that use a custom shader.
    pub uniforms: Vec<u8>,
    shader: Option<ShaderId>,
    /// The active shader's uniform data, copied into `uniforms` the first
    /// time something is drawn with it.
    shader_data: Vec<u8>,
    /// Where that copy is, once it has been made.
    shader_offset: Option<u32>,
}

impl Batch {
    /// Forgets everything queued. The target, blend mode and scissor stay.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.calls.clear();
        self.passes.clear();
        self.uniforms.clear();
        self.shader_offset = None;
    }

    /// Draws what is queued from now on with `shader` and this uniform
    /// data, or with the built-in shader for `None`.
    pub fn set_shader(&mut self, shader: Option<(ShaderId, &[u8])>) {
        self.shader_data.clear();
        self.shader_offset = None;
        self.shader = shader.map(|(id, data)| {
            self.shader_data.extend_from_slice(data);
            id
        });
    }

    /// Sends what is queued from now on to `target`.
    pub fn set_target(&mut self, target: Option<TextureId>) {
        self.target = target;
    }

    /// Fills the current target with `color`, replacing its contents.
    pub fn clear_target(&mut self, color: Color) {
        // Draws still waiting to reach this target would be painted over,
        // so drop them rather than send them to the GPU.
        if let Some(last) = self.passes.last()
            && last.target == self.target
        {
            let first_call = last.first_call as usize;
            if let Some(call) = self.calls.get(first_call) {
                self.vertices.truncate(call.first_vertex as usize);
            }
            self.calls.truncate(first_call);
            self.passes.pop();
        }
        self.passes.push(Pass {
            target: self.target,
            clear: Some(color),
            clear_stencil: true,
            first_call: self.calls.len() as u32,
            call_count: 0,
        });
    }

    /// Resets the current target's stencil values to zero, keeping its
    /// picture.
    pub fn clear_stencil(&mut self) {
        self.passes.push(Pass {
            target: self.target,
            clear: None,
            clear_stencil: true,
            first_call: self.calls.len() as u32,
            call_count: 0,
        });
    }

    /// Queues triangles, three vertices each, to draw on top of everything
    /// queued so far.
    pub fn push(&mut self, texture: Option<TextureId>, filter: Filter, triangles: &[Vertex]) {
        debug_assert!(triangles.len().is_multiple_of(3));
        if triangles.is_empty() {
            return;
        }
        assert!(
            texture.is_none() || texture != self.target,
            "a canvas cannot be drawn onto itself"
        );
        // Untextured geometry ignores the filter, so normalize it to let
        // all shapes share a draw call.
        let filter = if texture.is_some() {
            filter
        } else {
            Filter::default()
        };

        if self
            .passes
            .last()
            .is_none_or(|pass| pass.target != self.target)
        {
            self.passes.push(Pass {
                target: self.target,
                clear: None,
                clear_stencil: false,
                first_call: self.calls.len() as u32,
                call_count: 0,
            });
        }
        let pass = self.passes.last_mut().expect("a pass was just ensured");

        let uniform_offset = match (self.shader, self.shader_offset) {
            (None, _) => 0,
            (Some(_), Some(offset)) => offset,
            (Some(_), None) => {
                let offset = self.uniforms.len() as u32;
                self.uniforms.extend_from_slice(&self.shader_data);
                self.shader_offset = Some(offset);
                offset
            }
        };

        let first_vertex = self.vertices.len() as u32;
        let vertex_count = triangles.len() as u32;
        self.vertices.extend_from_slice(triangles);

        match self.calls.last_mut() {
            Some(last)
                if pass.call_count > 0
                    && last.texture == texture
                    && last.filter == filter
                    && last.blend == self.blend
                    && last.scissor == self.scissor
                    && last.stencil == self.stencil
                    && last.stencil_value == self.stencil_value
                    && last.shader == self.shader
                    && last.uniform_offset == uniform_offset =>
            {
                last.vertex_count += vertex_count;
            }
            _ => {
                self.calls.push(DrawCall {
                    texture,
                    filter,
                    blend: self.blend,
                    scissor: self.scissor,
                    stencil: self.stencil,
                    stencil_value: self.stencil_value,
                    shader: self.shader,
                    uniform_offset,
                    first_vertex,
                    vertex_count,
                });
                pass.call_count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIANGLE: [Vertex; 3] = [Vertex {
        position: [0.0, 0.0],
        uv: [0.0, 0.0],
        color: [1.0; 4],
    }; 3];

    #[test]
    fn consecutive_draws_of_one_texture_share_a_call() {
        let mut batch = Batch::default();
        let texture = Some(TextureId(1));
        batch.push(texture, Filter::Linear, &TRIANGLE);
        batch.push(texture, Filter::Linear, &TRIANGLE);

        assert_eq!(batch.calls.len(), 1);
        assert_eq!(batch.calls[0].first_vertex, 0);
        assert_eq!(batch.calls[0].vertex_count, 6);
        assert_eq!(batch.vertices.len(), 6);
    }

    #[test]
    fn a_different_texture_starts_a_new_call_and_keeps_draw_order() {
        let mut batch = Batch::default();
        batch.push(Some(TextureId(1)), Filter::Linear, &TRIANGLE);
        batch.push(Some(TextureId(2)), Filter::Linear, &TRIANGLE);
        batch.push(Some(TextureId(1)), Filter::Linear, &TRIANGLE);

        let textures: Vec<_> = batch.calls.iter().map(|call| call.texture).collect();
        assert_eq!(
            textures,
            [Some(TextureId(1)), Some(TextureId(2)), Some(TextureId(1))]
        );
        assert_eq!(batch.calls[2].first_vertex, 6);
    }

    #[test]
    fn a_different_filter_starts_a_new_call() {
        let mut batch = Batch::default();
        let texture = Some(TextureId(1));
        batch.push(texture, Filter::Linear, &TRIANGLE);
        batch.push(texture, Filter::Nearest, &TRIANGLE);

        assert_eq!(batch.calls.len(), 2);
    }

    #[test]
    fn shapes_share_a_call_whatever_filter_is_passed() {
        let mut batch = Batch::default();
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Nearest, &TRIANGLE);

        assert_eq!(batch.calls.len(), 1);
    }

    #[test]
    fn clear_forgets_everything() {
        let mut batch = Batch::default();
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.clear();

        assert!(batch.vertices.is_empty());
        assert!(batch.calls.is_empty());
        assert!(batch.passes.is_empty());
    }

    #[test]
    fn a_different_blend_mode_or_scissor_starts_a_new_call() {
        let mut batch = Batch::default();
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.blend = BlendMode::Add;
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.scissor = Some(Scissor {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        batch.push(None, Filter::Linear, &TRIANGLE);

        let blends: Vec<_> = batch.calls.iter().map(|call| call.blend).collect();
        assert_eq!(blends, [BlendMode::Alpha, BlendMode::Add, BlendMode::Add]);
        assert_eq!(batch.calls[1].vertex_count, 6);
        assert!(batch.calls[2].scissor.is_some());
        assert_eq!(batch.passes.len(), 1);
        assert_eq!(batch.passes[0].call_count, 3);
    }

    #[test]
    fn each_change_of_target_starts_a_pass_that_keeps_its_contents() {
        let mut batch = Batch::default();
        let canvas = Some(TextureId(7));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_target(canvas);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_target(None);
        batch.push(canvas, Filter::Linear, &TRIANGLE);

        let targets: Vec<_> = batch.passes.iter().map(|pass| pass.target).collect();
        assert_eq!(targets, [None, canvas, None]);
        assert!(batch.passes.iter().all(|pass| pass.clear.is_none()));
        // The same state on a different target must not share a call.
        let ranges: Vec<_> = batch
            .passes
            .iter()
            .map(|pass| (pass.first_call, pass.call_count))
            .collect();
        assert_eq!(ranges, [(0, 1), (1, 1), (2, 1)]);
        assert_eq!(batch.calls[1].vertex_count, 6);
    }

    #[test]
    fn switching_target_without_drawing_queues_nothing() {
        let mut batch = Batch::default();
        batch.set_target(Some(TextureId(7)));
        batch.set_target(None);

        assert!(batch.passes.is_empty());
    }

    #[test]
    fn clearing_drops_draws_it_would_paint_over() {
        let mut batch = Batch::default();
        let canvas = Some(TextureId(7));
        batch.set_target(canvas);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_target(None);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(Some(TextureId(1)), Filter::Linear, &TRIANGLE);
        batch.clear_target(Color::BLACK);
        batch.push(None, Filter::Linear, &TRIANGLE);

        // The canvas pass is untouched; the screen pass starts again.
        assert_eq!(batch.passes.len(), 2);
        assert_eq!(batch.passes[0].target, canvas);
        assert_eq!(batch.passes[1].clear, Some(Color::BLACK));
        assert_eq!(
            (batch.passes[1].first_call, batch.passes[1].call_count),
            (1, 1)
        );
        assert_eq!(batch.calls.len(), 2);
        assert_eq!(batch.calls[1].first_vertex, 3);
        assert_eq!(batch.vertices.len(), 6);
    }

    #[test]
    fn clearing_a_target_that_is_drawn_to_later_still_clears_it() {
        let mut batch = Batch::default();
        batch.set_target(Some(TextureId(7)));
        batch.clear_target(Color::WHITE);

        assert_eq!(batch.passes.len(), 1);
        assert_eq!(batch.passes[0].clear, Some(Color::WHITE));
        assert_eq!(batch.passes[0].call_count, 0);
    }

    #[test]
    fn a_shader_and_each_change_to_its_values_start_a_new_call() {
        let mut batch = Batch::default();
        let shader = ShaderId(3);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_shader(Some((shader, &[1, 1, 1, 1])));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_shader(Some((shader, &[2, 2, 2, 2])));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_shader(None);
        batch.push(None, Filter::Linear, &TRIANGLE);

        let calls: Vec<_> = batch
            .calls
            .iter()
            .map(|call| (call.shader, call.uniform_offset, call.vertex_count))
            .collect();
        assert_eq!(
            calls,
            [
                (None, 0, 3),
                (Some(shader), 0, 6),
                (Some(shader), 4, 3),
                (None, 0, 3)
            ]
        );
        assert_eq!(batch.uniforms, [1, 1, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn values_set_but_never_drawn_with_are_not_queued() {
        let mut batch = Batch::default();
        batch.set_shader(Some((ShaderId(3), &[1, 1, 1, 1])));
        batch.set_shader(Some((ShaderId(3), &[2, 2, 2, 2])));
        batch.push(None, Filter::Linear, &TRIANGLE);

        assert_eq!(batch.uniforms, [2, 2, 2, 2]);
    }

    #[test]
    fn a_shader_stays_active_after_the_frame_is_cleared() {
        let mut batch = Batch::default();
        batch.set_shader(Some((ShaderId(3), &[1, 1, 1, 1])));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.clear();
        batch.push(None, Filter::Linear, &TRIANGLE);

        assert_eq!(batch.calls[0].shader, Some(ShaderId(3)));
        assert_eq!(batch.calls[0].uniform_offset, 0);
        assert_eq!(batch.uniforms, [1, 1, 1, 1]);
    }

    #[test]
    fn stencil_state_starts_a_new_call_and_clearing_it_a_new_pass() {
        use smitten_core::{CompareMode, StencilAction};

        let mut batch = Batch::default();
        batch.clear_target(Color::BLACK);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.clear_stencil();
        batch.stencil = StencilMode::Write(StencilAction::Replace);
        batch.stencil_value = 1;
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.stencil = StencilMode::Test(CompareMode::Equal);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.stencil_value = 2;
        batch.push(None, Filter::Linear, &TRIANGLE);

        let passes: Vec<_> = batch
            .passes
            .iter()
            .map(|pass| (pass.clear.is_some(), pass.clear_stencil, pass.call_count))
            .collect();
        // Clearing the stencil keeps the picture drawn so far.
        assert_eq!(passes, [(true, true, 1), (false, true, 3)]);
        let counts: Vec<_> = batch.calls.iter().map(|call| call.vertex_count).collect();
        assert_eq!(counts, [3, 3, 6, 3]);
    }

    #[test]
    #[should_panic(expected = "cannot be drawn onto itself")]
    fn drawing_a_canvas_onto_itself_panics() {
        let mut batch = Batch::default();
        batch.set_target(Some(TextureId(7)));
        batch.push(Some(TextureId(7)), Filter::Linear, &TRIANGLE);
    }
}

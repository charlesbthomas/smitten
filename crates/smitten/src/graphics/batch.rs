//! Collects a frame's triangles and groups them into as few draw calls as
//! the draw order allows.

use smitten_core::backend::{
    BoundTexture, ColorMask, DrawCall, MAX_TARGETS, MeshDraw, Pass, Scissor, ShaderId, TargetSlice,
    TextureId, Vertex,
};
use smitten_core::{BlendAlphaMode, BlendMode, Color, Filter, StencilMode, VertexWinding};

use super::stats::FrameCounts;

/// Where draws go: a canvas's texture, or `None` for the screen, and the
/// canvases drawn to at the same time as it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Targets {
    pub first: Option<TextureId>,
    pub extra: [Option<TextureId>; MAX_TARGETS - 1],
    /// The part of each target drawn to, `first` then `extra`.
    pub slices: [TargetSlice; MAX_TARGETS],
}

impl Targets {
    /// One target, or the screen for `None`.
    pub fn single(target: Option<TextureId>) -> Self {
        Self {
            first: target,
            extra: [None; MAX_TARGETS - 1],
            slices: [TargetSlice::default(); MAX_TARGETS],
        }
    }

    /// Whether `texture` is one of these targets.
    fn contains(&self, texture: Option<TextureId>) -> bool {
        self.first == texture
            || self
                .extra
                .iter()
                .any(|extra| extra.is_some() && *extra == texture)
    }

    /// Whether `pass` draws to these targets.
    fn matches(&self, pass: &Pass) -> bool {
        pass.target == self.first && pass.extra_targets == self.extra && pass.slices == self.slices
    }

    /// A pass to these targets that clears nothing.
    fn pass(&self, first_call: u32) -> Pass {
        Pass {
            target: self.first,
            extra_targets: self.extra,
            clear: None,
            extra_clears: [None; MAX_TARGETS - 1],
            slices: self.slices,
            clear_stencil: None,
            first_call,
            call_count: 0,
        }
    }
}

/// A custom shader with its uniform data and the textures sent to it.
pub(crate) type ShaderState<'a> = (ShaderId, &'a [u8], &'a [Option<BoundTexture>]);

#[derive(Default)]
pub(crate) struct Batch {
    pub vertices: Vec<Vertex>,
    /// Geometry for the queued calls that draw from GPU buffers.
    pub meshes: Vec<MeshDraw>,
    pub calls: Vec<DrawCall>,
    pub passes: Vec<Pass>,
    /// Where draws go.
    target: Targets,
    /// Applied to everything queued from now on.
    pub blend: BlendMode,
    pub blend_alpha: BlendAlphaMode,
    pub scissor: Option<Scissor>,
    pub stencil: StencilMode,
    pub stencil_value: u8,
    pub color_mask: ColorMask,
    pub wireframe: bool,
    /// The on-screen winding of triangles to leave out.
    pub cull: Option<VertexWinding>,
    /// Uniform data for the queued calls that use a custom shader.
    pub uniforms: Vec<u8>,
    /// Textures for the queued calls whose shader declares its own.
    pub textures: Vec<Option<BoundTexture>>,
    shader: Option<ShaderId>,
    /// The active shader's uniform data and textures, copied into
    /// `uniforms` and `textures` the first time something is drawn with
    /// it.
    shader_data: Vec<u8>,
    shader_textures: Vec<Option<BoundTexture>>,
    /// Where those copies are, once they have been made.
    shader_offset: Option<(u32, u32)>,
    /// What has been queued since the frame began, for the game's
    /// statistics. Unlike the queue, it survives drawing canvases early.
    pub counts: FrameCounts,
}

impl Batch {
    /// Forgets everything queued. The target, blend mode and scissor stay.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.meshes.clear();
        self.calls.clear();
        self.passes.clear();
        self.uniforms.clear();
        self.textures.clear();
        self.shader_offset = None;
        self.counts = FrameCounts::default();
    }

    /// Forgets the passes queued for canvases, once they have been drawn
    /// ahead of the rest of the frame. What is queued for the screen
    /// stays, in order.
    pub fn forget_canvas_passes(&mut self) {
        let mut calls = Vec::with_capacity(self.calls.len());
        self.passes.retain_mut(|pass| {
            if pass.target.is_some() {
                return false;
            }
            let first = pass.first_call as usize;
            pass.first_call = calls.len() as u32;
            calls.extend_from_slice(&self.calls[first..first + pass.call_count as usize]);
            true
        });
        // The vertices and uniform data stay where they are, since the
        // calls that remain point into them.
        self.calls = calls;
    }

    /// Draws what is queued from now on with `shader`, this uniform data
    /// and these textures, or with the built-in shader for `None`.
    pub fn set_shader(&mut self, shader: Option<ShaderState>) {
        self.shader_data.clear();
        self.shader_textures.clear();
        self.shader_offset = None;
        self.shader = shader.map(|(id, data, textures)| {
            self.shader_data.extend_from_slice(data);
            self.shader_textures.extend_from_slice(textures);
            id
        });
    }

    /// Sends what is queued from now on to `target`.
    pub fn set_target(&mut self, target: Option<TextureId>) {
        self.target = Targets::single(target);
    }

    /// Sends what is queued from now on to several targets at once.
    pub fn set_targets(&mut self, targets: Targets) {
        self.target = targets;
    }

    /// Fills the current targets with `color`, replacing their contents,
    /// and resets the stencil values to zero.
    pub fn clear_target(&mut self, color: Color) {
        self.clear_target_ex(Some(color), Some(0));
    }

    /// Fills the current targets with `color` and sets the stencil values
    /// to `stencil`, each only if given.
    pub fn clear_target_ex(&mut self, color: Option<Color>, stencil: Option<u8>) {
        self.clear_targets([color; MAX_TARGETS], stencil);
    }

    /// Fills each current target with its own color, and sets the
    /// stencil values to `stencil`, each only if given. `colors[0]` is
    /// for the first target.
    pub fn clear_targets(&mut self, colors: [Option<Color>; MAX_TARGETS], stencil: Option<u8>) {
        let in_use = 1 + self.target.extra.iter().flatten().count();
        let colors_used = &colors[..in_use];
        if colors_used.iter().all(Option::is_none) {
            if let Some(value) = stencil {
                self.clear_stencil_to(value);
            }
            return;
        }
        // Draws still waiting to reach these targets would be painted
        // over, so drop them rather than send them to the GPU.
        if colors_used.iter().all(Option::is_some)
            && let Some(last) = self.passes.last()
            && self.target.matches(last)
        {
            let first_call = last.first_call as usize;
            if let Some(call) = self.calls.get(first_call) {
                self.vertices.truncate(call.first_vertex as usize);
            }
            self.calls.truncate(first_call);
            self.passes.pop();
        }
        let mut extra_clears = [None; MAX_TARGETS - 1];
        extra_clears.copy_from_slice(&colors[1..]);
        self.passes.push(Pass {
            clear: colors[0],
            extra_clears,
            clear_stencil: stencil,
            ..self.target.pass(self.calls.len() as u32)
        });
    }

    /// Resets the current target's stencil values to zero, keeping its
    /// picture.
    pub fn clear_stencil(&mut self) {
        self.clear_stencil_to(0);
    }

    /// Sets the current target's stencil values to `value`, keeping its
    /// picture.
    pub fn clear_stencil_to(&mut self, value: u8) {
        self.passes.push(Pass {
            clear_stencil: Some(value),
            ..self.target.pass(self.calls.len() as u32)
        });
    }

    /// Queues triangles that ignore the scissor, stencil, shader, color
    /// mask and wireframe in force and use `blend`. For the framework's
    /// own drawing, such as showing the game's picture in the window.
    pub fn push_plain(
        &mut self,
        texture: Option<TextureId>,
        filter: Filter,
        blend: BlendMode,
        triangles: &[Vertex],
    ) {
        let saved = (
            std::mem::replace(&mut self.blend, blend),
            std::mem::replace(&mut self.blend_alpha, blend.default_alpha_mode()),
            self.scissor.take(),
            std::mem::replace(&mut self.stencil, StencilMode::Off),
            self.shader.take(),
            std::mem::take(&mut self.color_mask),
            std::mem::take(&mut self.wireframe),
        );
        self.push(texture, filter, triangles);
        (
            self.blend,
            self.blend_alpha,
            self.scissor,
            self.stencil,
            self.shader,
            self.color_mask,
            self.wireframe,
        ) = saved;
    }

    /// Queues triangles, three vertices each, to draw on top of everything
    /// queued so far.
    pub fn push(&mut self, texture: Option<TextureId>, filter: Filter, triangles: &[Vertex]) {
        self.push_layer(texture, 0, filter, triangles);
    }

    /// As `push`, drawing picture `layer` of an array texture.
    pub fn push_layer(
        &mut self,
        texture: Option<TextureId>,
        layer: u32,
        filter: Filter,
        triangles: &[Vertex],
    ) {
        debug_assert!(triangles.len().is_multiple_of(3));
        if triangles.is_empty() {
            return;
        }
        assert!(
            texture.is_none() || !self.target.contains(texture),
            "a canvas cannot be drawn onto itself"
        );
        // Untextured geometry ignores the filter, so normalize it to let
        // all shapes share a draw call.
        let filter = if texture.is_some() {
            filter
        } else {
            Filter::default()
        };

        self.ensure_pass();
        let (uniform_offset, texture_offset) = self.shader_offsets();
        let first_vertex = self.vertices.len() as u32;
        let vertex_count = triangles.len() as u32;
        self.vertices.extend_from_slice(triangles);

        let pass_has_calls = self.passes.last().is_some_and(|pass| pass.call_count > 0);
        match self.calls.last_mut() {
            Some(last)
                if pass_has_calls
                    && last.mesh.is_none()
                    && last.texture == texture
                    && last.layer == layer
                    && last.filter == filter
                    && last.blend == self.blend
                    && last.blend_alpha == self.blend_alpha
                    && last.scissor == self.scissor
                    && last.stencil == self.stencil
                    && last.stencil_value == self.stencil_value
                    && last.color_mask == self.color_mask
                    && last.wireframe == self.wireframe
                    && last.cull == self.cull
                    && last.shader == self.shader
                    && last.uniform_offset == uniform_offset
                    && last.texture_offset == texture_offset =>
            {
                last.vertex_count += vertex_count;
                self.counts.draw_calls_batched += 1;
            }
            _ => self.push_call(DrawCall {
                texture,
                layer,
                filter,
                blend: self.blend,
                blend_alpha: self.blend_alpha,
                scissor: self.scissor,
                stencil: self.stencil,
                stencil_value: self.stencil_value,
                color_mask: self.color_mask,
                wireframe: self.wireframe,
                cull: self.cull,
                shader: self.shader,
                uniform_offset,
                texture_offset,
                mesh: None,
                first_vertex,
                vertex_count,
            }),
        }
    }

    /// Queues geometry kept in GPU buffers, to draw on top of everything
    /// queued so far in a draw call of its own.
    pub fn push_mesh(&mut self, texture: Option<TextureId>, filter: Filter, draw: MeshDraw) {
        if draw.index_count == 0 {
            return;
        }
        assert!(
            texture.is_none() || !self.target.contains(texture),
            "a canvas cannot be drawn onto itself"
        );
        let filter = if texture.is_some() {
            filter
        } else {
            Filter::default()
        };
        self.ensure_pass();
        let (uniform_offset, texture_offset) = self.shader_offsets();
        let mesh = self.meshes.len() as u32;
        self.meshes.push(draw);
        self.push_call(DrawCall {
            texture,
            layer: 0,
            filter,
            blend: self.blend,
            blend_alpha: self.blend_alpha,
            scissor: self.scissor,
            stencil: self.stencil,
            stencil_value: self.stencil_value,
            color_mask: self.color_mask,
            wireframe: self.wireframe,
            cull: self.cull,
            shader: self.shader,
            uniform_offset,
            texture_offset,
            mesh: Some(mesh),
            // Where a later clear would cut the vertices back to.
            first_vertex: self.vertices.len() as u32,
            vertex_count: 0,
        });
    }

    /// Starts a pass for the current target unless the last one is for it.
    fn ensure_pass(&mut self) {
        if self
            .passes
            .last()
            .is_none_or(|pass| !self.target.matches(pass))
        {
            if !self.passes.is_empty() {
                self.counts.canvas_switches += 1;
            }
            self.passes.push(self.target.pass(self.calls.len() as u32));
        }
    }

    /// Where the active shader's uniform data and textures are in the
    /// frame, copying them in the first time they are drawn with.
    fn shader_offsets(&mut self) -> (u32, u32) {
        match (self.shader, self.shader_offset) {
            (None, _) => (0, 0),
            (Some(_), Some(offsets)) => offsets,
            (Some(_), None) => {
                let offsets = (self.uniforms.len() as u32, self.textures.len() as u32);
                self.uniforms.extend_from_slice(&self.shader_data);
                self.textures.extend_from_slice(&self.shader_textures);
                self.shader_offset = Some(offsets);
                offsets
            }
        }
    }

    /// Adds a call to the last pass, which must be for the current target.
    fn push_call(&mut self, call: DrawCall) {
        self.counts.draw_calls += 1;
        if self
            .counts
            .last_shader
            .is_some_and(|last| last != self.shader)
        {
            self.counts.shader_switches += 1;
        }
        self.counts.last_shader = Some(self.shader);
        self.calls.push(call);
        let pass = self.passes.last_mut().expect("a pass was just ensured");
        pass.call_count += 1;
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

    fn mesh_draw(index_count: u32) -> MeshDraw {
        MeshDraw {
            layout: smitten_core::backend::VertexLayoutId(1),
            buffers: [None; smitten_core::backend::MAX_VERTEX_BUFFERS],
            vertex_count: 3,
            indices: smitten_core::backend::BufferId(2),
            index_type: smitten_core::backend::IndexDataType::U32,
            first_index: 0,
            index_count,
            transform: [0.0; 16],
            tint: [1.0; 4],
        }
    }

    #[test]
    fn a_mesh_draw_gets_a_call_of_its_own_that_later_draws_do_not_join() {
        let mut batch = Batch::default();
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push_mesh(None, Filter::Linear, mesh_draw(6));
        batch.push_mesh(None, Filter::Linear, mesh_draw(3));
        batch.push(None, Filter::Linear, &TRIANGLE);

        assert_eq!(batch.calls.len(), 4);
        assert_eq!(batch.meshes.len(), 2);
        assert_eq!(batch.calls[1].mesh, Some(0));
        assert_eq!(batch.calls[2].mesh, Some(1));
        assert_eq!(batch.calls[1].vertex_count, 0);
        // It marks where the shared vertices had got to.
        assert_eq!(batch.calls[1].first_vertex, 3);
        assert_eq!(batch.calls[3].mesh, None);
        assert_eq!(batch.counts.draw_calls, 4);
        assert_eq!(batch.passes[0].call_count, 4);
    }

    #[test]
    fn an_empty_mesh_draw_queues_nothing() {
        let mut batch = Batch::default();
        batch.push_mesh(None, Filter::Linear, mesh_draw(0));
        assert!(batch.calls.is_empty());
        assert!(batch.passes.is_empty());
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
    fn a_color_mask_or_wireframe_starts_a_new_call_that_plain_draws_ignore() {
        let mut batch = Batch::default();
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.color_mask = ColorMask {
            a: false,
            ..ColorMask::ALL
        };
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.wireframe = true;
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push_plain(None, Filter::Linear, BlendMode::Replace, &TRIANGLE);

        let states: Vec<_> = batch
            .calls
            .iter()
            .map(|call| (call.color_mask.a, call.wireframe))
            .collect();
        assert_eq!(
            states,
            [(true, false), (false, false), (false, true), (true, false)]
        );
        // The game's settings are back once the plain draw is queued.
        assert!(batch.wireframe);
        assert!(!batch.color_mask.a);
    }

    #[test]
    fn counts_follow_what_was_queued_until_cleared() {
        let mut batch = Batch::default();
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_shader(Some((ShaderId(3), &[], &[])));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_target(Some(TextureId(9)));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_shader(None);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.forget_canvas_passes();

        let counts = batch.counts;
        assert_eq!(counts.draw_calls, 4);
        assert_eq!(counts.draw_calls_batched, 1);
        assert_eq!(counts.canvas_switches, 1);
        assert_eq!(counts.shader_switches, 2);
        batch.clear();
        assert_eq!(batch.counts.draw_calls, 0);
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
    fn forgetting_canvas_passes_keeps_the_screen_draws_in_order() {
        let mut batch = Batch::default();
        let canvas = Some(TextureId(7));
        batch.push(Some(TextureId(1)), Filter::Linear, &TRIANGLE);
        batch.set_target(canvas);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(Some(TextureId(2)), Filter::Linear, &TRIANGLE);
        batch.set_target(None);
        batch.push(canvas, Filter::Linear, &TRIANGLE);
        batch.forget_canvas_passes();

        let passes: Vec<_> = batch
            .passes
            .iter()
            .map(|pass| (pass.target, pass.first_call, pass.call_count))
            .collect();
        assert_eq!(passes, [(None, 0, 1), (None, 1, 1)]);
        let calls: Vec<_> = batch
            .calls
            .iter()
            .map(|call| (call.texture, call.first_vertex))
            .collect();
        assert_eq!(calls, [(Some(TextureId(1)), 0), (canvas, 9)]);

        // Drawing carries on in the last screen pass.
        batch.push(canvas, Filter::Linear, &TRIANGLE);
        assert_eq!(batch.passes.len(), 2);
        assert_eq!(batch.calls.len(), 2);
        assert_eq!(batch.calls[1].vertex_count, 6);

        // A canvas drawn to again starts a pass of its own.
        batch.set_target(canvas);
        batch.push(None, Filter::Linear, &TRIANGLE);
        assert_eq!(batch.passes.len(), 3);
        assert_eq!(batch.passes[2].first_call, 2);
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
        batch.set_shader(Some((shader, &[1, 1, 1, 1], &[])));
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);
        batch.set_shader(Some((shader, &[2, 2, 2, 2], &[])));
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
        batch.set_shader(Some((ShaderId(3), &[1, 1, 1, 1], &[])));
        batch.set_shader(Some((ShaderId(3), &[2, 2, 2, 2], &[])));
        batch.push(None, Filter::Linear, &TRIANGLE);

        assert_eq!(batch.uniforms, [2, 2, 2, 2]);
    }

    #[test]
    fn a_shader_stays_active_after_the_frame_is_cleared() {
        let mut batch = Batch::default();
        batch.set_shader(Some((ShaderId(3), &[1, 1, 1, 1], &[])));
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
            .map(|pass| {
                (
                    pass.clear.is_some(),
                    pass.clear_stencil.is_some(),
                    pass.call_count,
                )
            })
            .collect();
        // Clearing the stencil keeps the picture drawn so far.
        assert_eq!(passes, [(true, true, 1), (false, true, 3)]);
        let counts: Vec<_> = batch.calls.iter().map(|call| call.vertex_count).collect();
        assert_eq!(counts, [3, 3, 6, 3]);
    }

    #[test]
    fn plain_draws_ignore_the_state_and_leave_it_as_it_was() {
        let mut batch = Batch {
            blend: BlendMode::Add,
            scissor: Some(Scissor {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            }),
            ..Batch::default()
        };
        batch.set_shader(Some((ShaderId(3), &[1, 1, 1, 1], &[])));
        batch.push_plain(None, Filter::Linear, BlendMode::Replace, &TRIANGLE);
        batch.push(None, Filter::Linear, &TRIANGLE);

        assert_eq!(batch.calls[0].blend, BlendMode::Replace);
        assert_eq!(batch.calls[0].scissor, None);
        assert_eq!(batch.calls[0].shader, None);
        assert_eq!(batch.calls[1].blend, BlendMode::Add);
        assert!(batch.calls[1].scissor.is_some());
        assert_eq!(batch.calls[1].shader, Some(ShaderId(3)));
    }

    #[test]
    #[should_panic(expected = "cannot be drawn onto itself")]
    fn drawing_a_canvas_onto_itself_panics() {
        let mut batch = Batch::default();
        batch.set_target(Some(TextureId(7)));
        batch.push(Some(TextureId(7)), Filter::Linear, &TRIANGLE);
    }
}

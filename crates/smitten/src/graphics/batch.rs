//! Collects a frame's triangles and groups them into as few draw calls as
//! the draw order allows.

use smitten_core::Filter;
use smitten_core::backend::{DrawCall, TextureId, Vertex};

#[derive(Default)]
pub(crate) struct Batch {
    pub vertices: Vec<Vertex>,
    pub calls: Vec<DrawCall>,
}

impl Batch {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.calls.clear();
    }

    /// Queues triangles, three vertices each, to draw on top of everything
    /// queued so far.
    pub fn push(&mut self, texture: Option<TextureId>, filter: Filter, triangles: &[Vertex]) {
        debug_assert!(triangles.len().is_multiple_of(3));
        if triangles.is_empty() {
            return;
        }
        // Untextured geometry ignores the filter, so normalize it to let
        // all shapes share a draw call.
        let filter = if texture.is_some() {
            filter
        } else {
            Filter::default()
        };

        let first_vertex = self.vertices.len() as u32;
        let vertex_count = triangles.len() as u32;
        self.vertices.extend_from_slice(triangles);

        match self.calls.last_mut() {
            Some(last) if last.texture == texture && last.filter == filter => {
                last.vertex_count += vertex_count;
            }
            _ => self.calls.push(DrawCall {
                texture,
                filter,
                first_vertex,
                vertex_count,
            }),
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
    }
}

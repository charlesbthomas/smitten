//! Counts of what the game draws and holds, the equivalent of
//! `love.graphics.getStats`.

use std::cell::Cell;

use smitten_core::backend::ShaderId;

/// What the current frame has drawn so far and what the game holds, from
/// [`graphics::stats`](crate::graphics::stats).
///
/// The draw counts start from zero each frame, so read them at the end of
/// `draw` to see the whole frame, as in LÖVE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Draw calls sent to the GPU. Each change of image, blend mode,
    /// shader, scissor or other state starts a new one.
    pub draw_calls: u32,
    /// Draws that were merged into the draw call before them instead of
    /// starting their own.
    pub draw_calls_batched: u32,
    /// Times drawing moved to a different canvas or back to the screen.
    pub canvas_switches: u32,
    /// Times a draw call used a different shader from the one before it.
    pub shader_switches: u32,
    /// Roughly how much GPU memory images, canvases and fonts take, in
    /// bytes.
    pub texture_memory: u64,
    /// Images alive, not counting canvases.
    pub images: u32,
    /// Canvases alive.
    pub canvases: u32,
    /// Fonts alive.
    pub fonts: u32,
}

/// The draw counts for the frame being queued.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FrameCounts {
    pub draw_calls: u32,
    pub draw_calls_batched: u32,
    pub canvas_switches: u32,
    pub shader_switches: u32,
    /// The shader of the latest draw call, once there is one.
    pub last_shader: Option<Option<ShaderId>>,
}

/// The kinds of resource [`Stats`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Canvas,
    Font,
}

thread_local! {
    /// How many of each kind are alive, indexed by `Kind`.
    static ALIVE: Cell<[u32; 3]> = const { Cell::new([0; 3]) };
}

/// Counts one resource of its kind for as long as it lives.
#[derive(Debug)]
pub(crate) struct Counted(Kind);

impl Counted {
    pub fn new(kind: Kind) -> Counted {
        change(kind, 1);
        Counted(kind)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        change(self.0, -1);
    }
}

fn change(kind: Kind, by: i32) {
    ALIVE.with(|alive| {
        let mut counts = alive.get();
        let count = &mut counts[kind as usize];
        *count = count.saturating_add_signed(by);
        alive.set(counts);
    });
}

/// How many resources of `kind` are alive.
pub(crate) fn alive(kind: Kind) -> u32 {
    ALIVE.with(|alive| alive.get()[kind as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counted_resources_are_forgotten_when_dropped() {
        let before = alive(Kind::Font);
        let first = Counted::new(Kind::Font);
        let second = Counted::new(Kind::Font);
        assert_eq!(alive(Kind::Font), before + 2);
        drop(first);
        assert_eq!(alive(Kind::Font), before + 1);
        drop(second);
        assert_eq!(alive(Kind::Font), before);
    }
}

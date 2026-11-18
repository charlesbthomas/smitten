//! Text laid out once and drawn many times, the equivalent of LÖVE's
//! `Text`.

use glam::Vec2;
use std::cell::RefCell;

use smitten_core::backend::{Backend, TextureId, Vertex};

use super::text::{GlyphColors, joined};
use super::{Align, Font, Transform};
use crate::Color;

/// Text laid out ahead of time, so drawing it costs no more than drawing
/// an image: the equivalent of LÖVE's `Text`, made with
/// [`Text::new`](crate::graphics::Text::new) and drawn with
/// [`graphics::draw_text`](crate::graphics::draw_text).
///
/// [`graphics::print`](crate::graphics::print) works out where every glyph
/// goes each time it is called. A `Text` does that once, the first time it
/// is drawn after a change, which matters for a lot of text that rarely
/// changes, such as a score table or a page of dialogue.
///
/// It can hold several pieces of text, each added at its own place with
/// [`add`](Text::add) or wrapped with [`add_f`](Text::add_f). They are
/// all drawn together in the current color.
#[derive(Debug, Clone)]
pub struct Text {
    font: Font,
    entries: Vec<Entry>,
    /// The laid-out glyphs, made when first drawn and dropped on any
    /// change.
    cache: RefCell<Option<Cache>>,
}

#[derive(Debug, Clone)]
struct Entry {
    /// All the text, for measuring.
    text: String,
    /// The text in parts, each with its color.
    segments: Vec<(Color, String)>,
    wrap: Option<(f32, Align)>,
    transform: Transform,
}

#[derive(Debug, Clone)]
struct Cache {
    /// What the font looked like when the glyphs were laid out.
    key: (u32, u32),
    /// Runs of glyph quads that share an atlas page, in white, in the
    /// text's own coordinates.
    runs: Vec<(TextureId, Vec<Vertex>)>,
}

impl Text {
    /// Makes a `Text` in `font` holding `text` at its top-left corner. An
    /// empty string makes it start out empty.
    pub fn new(font: &Font, text: &str) -> Text {
        let mut made = Text {
            font: font.clone(),
            entries: Vec::new(),
            cache: RefCell::new(None),
        };
        made.set(text);
        made
    }

    /// Replaces everything with `text` at the top-left corner.
    pub fn set(&mut self, text: &str) {
        self.clear();
        if !text.is_empty() {
            self.add(text, (0.0, 0.0));
        }
    }

    /// Replaces everything with `text` wrapped to `limit` units wide and
    /// aligned within that width, as
    /// [`graphics::printf`](crate::graphics::printf) draws it.
    pub fn set_f(&mut self, text: &str, limit: f32, align: Align) {
        self.clear();
        if !text.is_empty() {
            self.add_f(text, limit, align, (0.0, 0.0));
        }
    }

    /// Adds `text` with its top-left corner at `(x, y)`, keeping what is
    /// already there. Returns its index, counting from 0.
    pub fn add(&mut self, text: &str, position: impl Into<Vec2>) -> usize {
        let Vec2 { x, y } = position.into();
        self.add_with(
            text,
            &Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0),
        )
    }

    /// Adds `text` with the full set of LÖVE's `add` arguments, which mean
    /// what they do for [`graphics::draw_ex`](crate::graphics::draw_ex).
    #[allow(clippy::too_many_arguments)] // Mirrors Text:add.
    pub fn add_ex(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) -> usize {
        self.add_with(text, &Transform::from_draw_args(x, y, r, sx, sy, ox, oy))
    }

    /// Adds `text` placed by `transform`.
    pub fn add_with(&mut self, text: &str, transform: &Transform) -> usize {
        self.push(text, None, transform)
    }

    /// Adds `text` wrapped to `limit` units and aligned within that width,
    /// with its top-left corner at `(x, y)`.
    pub fn add_f(
        &mut self,
        text: &str,
        limit: f32,
        align: Align,
        position: impl Into<Vec2>,
    ) -> usize {
        let Vec2 { x, y } = position.into();
        let transform = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
        self.add_f_with(text, limit, align, &transform)
    }

    /// Adds wrapped text placed by `transform`.
    pub fn add_f_with(
        &mut self,
        text: &str,
        limit: f32,
        align: Align,
        transform: &Transform,
    ) -> usize {
        self.push(text, Some((limit, align)), transform)
    }

    /// Replaces everything with text made of colored parts at the top-left
    /// corner, the equivalent of `text:set(coloredtext)`. Each part is
    /// tinted by its color as well as the one the text is drawn in.
    pub fn set_colored(&mut self, segments: &[(Color, &str)]) {
        self.clear();
        self.add_colored(segments, (0.0, 0.0));
    }

    /// Replaces everything with colored text wrapped to `limit` units.
    pub fn set_f_colored(&mut self, segments: &[(Color, &str)], limit: f32, align: Align) {
        self.clear();
        self.add_f_colored(segments, limit, align, (0.0, 0.0));
    }

    /// Adds text made of colored parts with its top-left corner at
    /// `(x, y)`. Returns its index.
    pub fn add_colored(&mut self, segments: &[(Color, &str)], position: impl Into<Vec2>) -> usize {
        let Vec2 { x, y } = position.into();
        let transform = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
        self.add_colored_with(segments, &transform)
    }

    /// Adds colored text placed by `transform`.
    pub fn add_colored_with(&mut self, segments: &[(Color, &str)], transform: &Transform) -> usize {
        self.push_segments(segments, None, transform)
    }

    /// Adds colored text wrapped to `limit` units with its top-left corner
    /// at `(x, y)`.
    pub fn add_f_colored(
        &mut self,
        segments: &[(Color, &str)],
        limit: f32,
        align: Align,
        position: impl Into<Vec2>,
    ) -> usize {
        let Vec2 { x, y } = position.into();
        let transform = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
        self.push_segments(segments, Some((limit, align)), &transform)
    }

    fn push(&mut self, text: &str, wrap: Option<(f32, Align)>, transform: &Transform) -> usize {
        self.push_segments(&[(Color::WHITE, text)], wrap, transform)
    }

    fn push_segments(
        &mut self,
        segments: &[(Color, &str)],
        wrap: Option<(f32, Align)>,
        transform: &Transform,
    ) -> usize {
        self.entries.push(Entry {
            text: joined(segments),
            segments: segments
                .iter()
                .map(|&(color, text)| (color, text.to_string()))
                .collect(),
            wrap,
            transform: *transform,
        });
        self.cache.get_mut().take();
        self.entries.len() - 1
    }

    /// Removes all the text.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cache.get_mut().take();
    }

    /// How many pieces of text have been added since it was last cleared
    /// or set.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there is no text at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The font the text is drawn in.
    pub fn font(&self) -> Font {
        self.font.clone()
    }

    /// Lays all the text out again in `font`.
    pub fn set_font(&mut self, font: &Font) {
        if !self.font.is(font) {
            self.font = font.clone();
            self.cache.get_mut().take();
        }
    }

    /// The size in units of the box from `(0, 0)` that holds all the text.
    pub fn dimensions(&self) -> (f32, f32) {
        (0..self.entries.len())
            .filter_map(|index| self.bounds(index))
            .fold(
                (0.0f32, 0.0f32),
                |(width, height), [_, _, right, bottom]| (width.max(right), height.max(bottom)),
            )
    }

    /// The width in units of the box from `(0, 0)` that holds all the text.
    pub fn width(&self) -> f32 {
        self.dimensions().0
    }

    /// The height in units of the box from `(0, 0)` that holds all the
    /// text.
    pub fn height(&self) -> f32 {
        self.dimensions().1
    }

    /// The box around the piece of text at `index` as `[left, top, right,
    /// bottom]` in units, or `None` if there is no such piece.
    pub fn bounds(&self, index: usize) -> Option<[f32; 4]> {
        let entry = self.entries.get(index)?;
        let font = &self.font;
        let lines = match entry.wrap {
            Some((limit, _)) => font.wrap(&entry.text, limit).1.len(),
            None => entry.text.split('\n').count(),
        }
        .max(1);
        let width = match entry.wrap {
            None => font.width(&entry.text),
            Some((limit, align)) => font.aligned_width(&entry.text, limit, align),
        };
        let height = font.height() * (1.0 + (lines - 1) as f32 * font.line_height());
        let corners = [
            entry.transform.apply((0.0, 0.0)),
            entry.transform.apply((width, 0.0)),
            entry.transform.apply((width, height)),
            entry.transform.apply((0.0, height)),
        ];
        Some(corners.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |[left, top, right, bottom], &Vec2 { x, y }| {
                [left.min(x), top.min(y), right.max(x), bottom.max(y)]
            },
        ))
    }

    /// Calls `emit` with each run of glyphs in their own colors, in the
    /// text's own coordinates, laying the text out first if it changed.
    pub(crate) fn with_runs(
        &self,
        backend: &mut dyn Backend,
        mut emit: impl FnMut(TextureId, &[Vertex]),
    ) {
        let key = self.font.layout_key();
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|cache| cache.key != key) {
            *cache = Some(Cache {
                key,
                runs: self.layout(backend),
            });
        }
        for (texture, vertices) in &cache.as_ref().expect("the cache was just filled").runs {
            emit(*texture, vertices);
        }
    }

    fn layout(&self, backend: &mut dyn Backend) -> Vec<(TextureId, Vec<Vertex>)> {
        let mut runs: Vec<(TextureId, Vec<Vertex>)> = Vec::new();
        for entry in &self.entries {
            let segments: Vec<(Color, &str)> = entry
                .segments
                .iter()
                .map(|(color, text)| (*color, text.as_str()))
                .collect();
            let mut colors = GlyphColors::new(&segments);
            self.font.layout(
                backend,
                &entry.text,
                entry.wrap,
                |character, texture, rect, uv| {
                    let color = colors.color(character);
                    let color = [color.r, color.g, color.b, color.a];
                    if runs.last().is_none_or(|(last, _)| *last != texture) {
                        runs.push((texture, Vec::new()));
                    }
                    let corner = |x: f32, y: f32, u: f32, v: f32| {
                        let Vec2 { x, y } = entry.transform.apply((x, y));
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
                    let run = &mut runs.last_mut().expect("a run was just ensured").1;
                    run.extend_from_slice(&[
                        top_left,
                        top_right,
                        bottom_right,
                        top_left,
                        bottom_right,
                        bottom_left,
                    ]);
                },
            );
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    fn font() -> Font {
        Font::fallback_at(12.0, Filter::Linear, 1.0)
    }

    #[test]
    fn add_returns_indexes_and_set_starts_over() {
        let mut text = Text::new(&font(), "score");
        assert_eq!(text.len(), 1);
        assert_eq!(text.add("lives", (0.0, 20.0)), 1);
        assert_eq!(text.add_f("wrapped", 50.0, Align::Center, (0.0, 40.0)), 2);
        text.set("only this");
        assert_eq!(text.len(), 1);
        text.set("");
        assert!(text.is_empty());
    }

    #[test]
    fn dimensions_cover_every_piece_where_it_was_placed() {
        let font = font();
        let mut text = Text::new(&font, "");
        assert_eq!(text.dimensions(), (0.0, 0.0));
        text.add("hello", (10.0, 0.0));
        let (width, height) = text.dimensions();
        assert!((width - (10.0 + font.width("hello"))).abs() < 1e-3);
        assert!((height - font.height()).abs() < 1e-3);
        text.add("x", (0.0, 100.0));
        assert!((text.height() - (100.0 + font.height())).abs() < 1e-3);
    }

    #[test]
    fn scaled_pieces_measure_larger() {
        let font = font();
        let mut text = Text::new(&font, "");
        text.add_ex("hello", 0.0, 0.0, 0.0, 2.0, 3.0, 0.0, 0.0);
        let (width, height) = text.dimensions();
        assert!((width - 2.0 * font.width("hello")).abs() < 1e-3);
        assert!((height - 3.0 * font.height()).abs() < 1e-3);
    }

    #[test]
    fn right_aligned_text_reaches_the_limit() {
        let mut text = Text::new(&font(), "");
        text.add_f("hi", 200.0, Align::Right, (0.0, 0.0));
        assert!((text.width() - 200.0).abs() < 1e-3);
        let [left, ..] = text.bounds(0).unwrap();
        assert_eq!(left, 0.0);
        assert!(text.bounds(1).is_none());
    }

    #[test]
    fn lines_add_height() {
        let font = font();
        let text = Text::new(&font, "a\nb\nc");
        let expected = font.height() * (1.0 + 2.0 * font.line_height());
        assert!((text.height() - expected).abs() < 1e-3);
    }
}

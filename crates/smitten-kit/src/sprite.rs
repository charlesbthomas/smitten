//! Sprite sheets: an image cut into numbered frames, drawn by number.
//!
//! Pair a [`SpriteSheet`] with an [`Animator`](crate::animation::Animator)
//! (which picks the frame number) to animate a character:
//!
//! ```no_run
//! use smitten::graphics::Image;
//! use smitten_kit::animation::{Animation, Animator};
//! use smitten_kit::sprite::{Flip, SpriteSheet};
//!
//! # fn main() -> smitten::Result<()> {
//! // A 128x64 sheet of 32x32 frames: 4 across, 2 down, numbered row by row.
//! let sheet = SpriteSheet::grid(Image::load("hero.png")?, 32, 32);
//! let mut hero = Animator::new("run", Animation::range(0..4, 10.0));
//!
//! // each frame:
//! hero.update(1.0 / 60.0);
//! sheet.draw_at(hero.frame(), (100.0, 80.0), Flip::X);
//! # Ok(())
//! # }
//! ```

use smitten::Vec2;
use smitten::graphics::{self, DrawParams, Image, Quad};

/// Which way to mirror a sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Flip {
    /// Mirrored left to right, as a character facing the other way.
    pub x: bool,
    /// Mirrored top to bottom.
    pub y: bool,
}

impl Flip {
    /// As drawn.
    pub const NONE: Flip = Flip { x: false, y: false };
    /// Mirrored left to right.
    pub const X: Flip = Flip { x: true, y: false };
    /// Mirrored top to bottom.
    pub const Y: Flip = Flip { x: false, y: true };

    /// Mirrored left to right when `facing_left`, the usual case.
    pub fn facing_left(facing_left: bool) -> Flip {
        Flip {
            x: facing_left,
            y: false,
        }
    }
}

/// The numbered frames of one image. See the [module docs](self).
#[derive(Clone)]
pub struct SpriteSheet {
    image: Image,
    quads: Vec<Quad>,
}

impl std::fmt::Debug for SpriteSheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpriteSheet")
            .field("size", &self.image.dimensions())
            .field("frames", &self.quads.len())
            .finish()
    }
}

/// Where each frame of a grid sits in a sheet: `(x, y)` pixel corners,
/// row by row. Frames that would run off the sheet are left out.
pub fn grid_frames(
    sheet: (u32, u32),
    frame: (u32, u32),
    margin: u32,
    spacing: u32,
) -> Vec<(u32, u32)> {
    let (sheet_w, sheet_h) = sheet;
    let (w, h) = frame;
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut y = margin;
    while y + h <= sheet_h {
        let mut x = margin;
        while x + w <= sheet_w {
            out.push((x, y));
            x += w + spacing;
        }
        y += h + spacing;
    }
    out
}

impl SpriteSheet {
    /// Cuts `image` into `frame_width` by `frame_height` frames, numbered
    /// row by row from the top left.
    pub fn grid(image: Image, frame_width: u32, frame_height: u32) -> SpriteSheet {
        SpriteSheet::grid_spaced(image, frame_width, frame_height, 0, 0)
    }

    /// Like [`grid`](SpriteSheet::grid), for sheets with `margin` pixels
    /// around the edge and `spacing` pixels between frames.
    pub fn grid_spaced(
        image: Image,
        frame_width: u32,
        frame_height: u32,
        margin: u32,
        spacing: u32,
    ) -> SpriteSheet {
        let size = image.dimensions();
        let quads = grid_frames(size, (frame_width, frame_height), margin, spacing)
            .into_iter()
            .map(|(x, y)| {
                Quad::new(
                    (x as f32, y as f32),
                    (frame_width as f32, frame_height as f32),
                    image.size(),
                )
            })
            .collect();
        SpriteSheet { image, quads }
    }

    /// A sheet with frames cut wherever `frames` says, as a packed atlas
    /// lists them: `(position, size)` in pixels.
    pub fn from_regions(
        image: Image,
        frames: impl IntoIterator<Item = (Vec2, Vec2)>,
    ) -> SpriteSheet {
        let sheet = image.size();
        let quads = frames
            .into_iter()
            .map(|(position, size)| Quad::new(position, size, sheet))
            .collect();
        SpriteSheet { image, quads }
    }

    /// The image.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// How many frames it has.
    pub fn len(&self) -> usize {
        self.quads.len()
    }

    /// Whether it has no frames.
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// The part of the image frame `index` covers, or `None` past the
    /// end.
    pub fn quad(&self, index: usize) -> Option<&Quad> {
        self.quads.get(index)
    }

    /// The size of frame `index`, or zero past the end.
    pub fn frame_size(&self, index: usize) -> Vec2 {
        self.quad(index)
            .map_or(Vec2::ZERO, |quad| Vec2::new(quad.width(), quad.height()))
    }

    /// Draws frame `index` placed by `params`. Past the end it draws
    /// nothing.
    pub fn draw(&self, index: usize, params: impl Into<DrawParams>) {
        if let Some(quad) = self.quads.get(index) {
            graphics::draw(&self.image.region(quad), params);
        }
    }

    /// Draws frame `index` centered on `center`, mirrored by `flip`.
    pub fn draw_at(&self, index: usize, center: impl Into<Vec2>, flip: Flip) {
        let size = self.frame_size(index);
        let scale = Vec2::new(
            if flip.x { -1.0 } else { 1.0 },
            if flip.y { -1.0 } else { 1.0 },
        );
        self.draw(
            index,
            DrawParams::at(center).origin(size / 2.0).scale_xy(scale),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_numbers_frames_row_by_row() {
        let frames = grid_frames((64, 40), (16, 16), 0, 0);
        assert_eq!(frames.len(), 8); // 4 across, 2 full rows down
        assert_eq!(frames[0], (0, 0));
        assert_eq!(frames[4], (0, 16));
        let spaced = grid_frames((37, 20), (16, 16), 1, 2);
        assert_eq!(spaced, [(1, 1), (19, 1)]);
        assert!(grid_frames((10, 10), (0, 4), 0, 0).is_empty());
    }
}

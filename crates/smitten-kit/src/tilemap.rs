//! Drawing maps made in the [Tiled] editor.
//!
//! [`TiledMap::load`] reads a `.tmj` map, its external tilesets and their
//! images through smitten's filesystem. [`TiledMap::draw_layer`] draws one
//! tile layer, only the tiles inside the visible rectangle, so a large
//! map costs what the screen shows:
//!
//! ```no_run
//! use smitten_kit::camera::Camera;
//! use smitten_kit::tilemap::TiledMap;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let level = TiledMap::load("maps/cave.tmj")?;
//! let camera = Camera::fullscreen();
//!
//! // each frame:
//! camera.draw(|| {
//!     level.draw_layer("ground", camera.visible_rect());
//!     // ... the player and enemies ...
//!     level.draw_layer("overhang", camera.visible_rect());
//! });
//!
//! // in the rules:
//! let solid = level.map().solid_grid();
//! # Ok(())
//! # }
//! ```
//!
//! The map itself is a [`TileMap`] from
//! [`smitten_kit_core::tilemap`](crate::tilemap::data), with its objects,
//! properties and solid cells.
//!
//! [Tiled]: https://www.mapeditor.org

use std::path::{Path, PathBuf};

use smitten::Vec2;
use smitten::graphics::{self, DrawParams, Image, Quad};
use smitten_kit_core::glam::IVec2;

pub use smitten_kit_core::tilemap as data;
pub use smitten_kit_core::tilemap::{
    Layer, LayerKind, Object, Properties, Property, Shape, Tile, TileMap, TiledError, Tileset,
};

use crate::math::Rect;

/// Why [`TiledMap::load`] failed.
#[derive(Debug)]
pub enum LoadError {
    /// A file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// Why.
        error: std::io::Error,
    },
    /// The map is not one this reader handles.
    Map(TiledError),
    /// A tileset image could not be loaded.
    Image {
        /// The image.
        path: PathBuf,
        /// Why.
        error: String,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io { path, error } => write!(f, "reading {}: {error}", path.display()),
            LoadError::Map(error) => error.fmt(f),
            LoadError::Image { path, error } => {
                write!(f, "loading {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// Where a tile of `size` drawn into a `cell` sized grid cell at `corner`
/// (its top left) goes, and how it is turned: the center, rotation and
/// scale to draw it with around its own center. Tiles taller than a cell
/// stand on the cell's bottom edge, as Tiled draws them.
pub fn tile_placement(tile: Tile, corner: Vec2, cell: Vec2, size: Vec2) -> (Vec2, f32, Vec2) {
    let top_left = Vec2::new(corner.x, corner.y + cell.y - size.y);
    let center = top_left + size / 2.0;
    let x = if tile.flip_x { -1.0 } else { 1.0 };
    let y = if tile.flip_y { -1.0 } else { 1.0 };
    if tile.flip_diagonal {
        // Mirroring across the diagonal is a quarter turn and a flip.
        (center, std::f32::consts::FRAC_PI_2, Vec2::new(y, -x))
    } else {
        (center, 0.0, Vec2::new(x, y))
    }
}

/// The cells of a `size` map (in cells of `cell` pixels) that `visible`
/// can show, with one row more below for tiles taller than a cell.
/// `(first, last)`, both included; `None` when nothing shows.
pub fn visible_cells(
    visible: Rect,
    cell: Vec2,
    size: (u32, u32),
    tall: bool,
) -> Option<(IVec2, IVec2)> {
    let lo = (visible.min / cell).floor().as_ivec2().max(IVec2::ZERO);
    let extra = if tall { 1 } else { 0 };
    let hi = ((visible.max / cell).floor().as_ivec2() + IVec2::new(0, extra))
        .min(IVec2::new(size.0 as i32 - 1, size.1 as i32 - 1));
    (lo.x <= hi.x && lo.y <= hi.y).then_some((lo, hi))
}

struct LoadedSet {
    image: Option<Image>,
    quads: Vec<Quad>,
}

/// A [`TileMap`] with its tileset images, ready to draw. See the
/// [module docs](self).
pub struct TiledMap {
    map: TileMap,
    sets: Vec<LoadedSet>,
}

impl std::fmt::Debug for TiledMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TiledMap")
            .field("size", &self.map.size)
            .field("layers", &self.map.layers.len())
            .finish()
    }
}

fn read(path: &Path) -> Result<String, LoadError> {
    smitten::filesystem::read_string(path).map_err(|error| LoadError::Io {
        path: path.to_owned(),
        error,
    })
}

impl TiledMap {
    /// Loads a map and everything it needs through smitten's filesystem.
    /// Paths inside the map are taken from the map's folder.
    pub fn load(path: impl AsRef<Path>) -> Result<TiledMap, LoadError> {
        let path = path.as_ref();
        let folder = path.parent().unwrap_or(Path::new(""));
        let text = read(path)?;
        let map = TileMap::from_json_with(&text, |source| {
            read(&folder.join(source)).map_err(|error| error.to_string())
        })
        .map_err(LoadError::Map)?;
        TiledMap::with_images(map, |image| {
            let path = folder.join(image);
            Image::load(&path)
                .map(Some)
                .map_err(|error| LoadError::Image {
                    path,
                    error: error.to_string(),
                })
        })
    }

    /// A map whose tileset images come from `image`, given each
    /// tileset's image path. Returning `Ok(None)` leaves that tileset
    /// undrawn.
    pub fn with_images<E>(
        map: TileMap,
        mut image: impl FnMut(&str) -> Result<Option<Image>, E>,
    ) -> Result<TiledMap, E> {
        let mut sets = Vec::new();
        for set in &map.tilesets {
            let loaded = if set.image.is_empty() {
                None
            } else {
                image(&set.image)?
            };
            let sheet = loaded
                .as_ref()
                .map_or(set.image_size.as_vec2(), |image| image.size());
            let quads = (0..set.tile_count)
                .map(|id| {
                    Quad::new(
                        set.tile_position(id).as_vec2(),
                        set.tile_size.as_vec2(),
                        sheet,
                    )
                })
                .collect();
            sets.push(LoadedSet {
                image: loaded,
                quads,
            });
        }
        Ok(TiledMap { map, sets })
    }

    /// The map: layers, objects, properties.
    pub fn map(&self) -> &TileMap {
        &self.map
    }

    /// Draws the tile layer called `name`, only the tiles `visible` (in
    /// map pixels, as [`Camera::visible_rect`](crate::camera::Camera::visible_rect)
    /// gives) can show. Hidden layers and other kinds of layer draw
    /// nothing.
    pub fn draw_layer(&self, name: &str, visible: Rect) {
        if let Some(index) = self.map.layers.iter().position(|l| l.name == name) {
            self.draw_layer_at(index, visible);
        }
    }

    /// Draws every visible tile layer, bottom first.
    pub fn draw(&self, visible: Rect) {
        for index in 0..self.map.layers.len() {
            self.draw_layer_at(index, visible);
        }
    }

    fn draw_layer_at(&self, index: usize, visible: Rect) {
        let layer = &self.map.layers[index];
        let Some(tiles) = layer.tiles() else {
            return;
        };
        if !layer.visible || layer.opacity <= 0.0 {
            return;
        }
        let cell = self.map.tile_size.as_vec2();
        let tall = self
            .map
            .tilesets
            .iter()
            .any(|set| set.tile_size.y > self.map.tile_size.y);
        let Some((lo, hi)) = visible_cells(
            visible.translate(-layer.offset),
            cell,
            (tiles.width(), tiles.height()),
            tall,
        ) else {
            return;
        };
        let color = graphics::color();
        graphics::set_color(color.with_alpha(color.a * layer.opacity));
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let tile = tiles[IVec2::new(x, y)];
                if tile.is_empty() {
                    continue;
                }
                let Some(set_index) = self
                    .map
                    .tilesets
                    .iter()
                    .rposition(|set| set.first_gid <= tile.gid)
                else {
                    continue;
                };
                let set = &self.map.tilesets[set_index];
                let loaded = &self.sets[set_index];
                let (Some(image), Some(quad)) = (
                    &loaded.image,
                    loaded.quads.get((tile.gid - set.first_gid) as usize),
                ) else {
                    continue;
                };
                let size = set.tile_size.as_vec2();
                let corner = Vec2::new(x as f32, y as f32) * cell + layer.offset;
                let (center, rotation, scale) = tile_placement(tile, corner, cell, size);
                graphics::draw(
                    &image.region(quad),
                    DrawParams::at(center)
                        .rotation(rotation)
                        .scale_xy(scale)
                        .origin(size / 2.0),
                );
            }
        }
        graphics::set_color(color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn culling_covers_what_shows() {
        let cell = Vec2::splat(16.0);
        let view = Rect::from_position_size((20.0, 0.0), (40.0, 20.0));
        let (lo, hi) = visible_cells(view, cell, (100, 100), false).unwrap();
        assert_eq!((lo.x, lo.y, hi.x, hi.y), (1, 0, 3, 1));
        let (_, hi) = visible_cells(view, cell, (100, 100), true).unwrap();
        assert_eq!(hi.y, 2);
        // Clamped to the map, and nothing for a view off it.
        let (lo, hi) = visible_cells(
            Rect::new((-50.0, -50.0), (5000.0, 5.0)),
            cell,
            (10, 10),
            false,
        )
        .unwrap();
        assert_eq!((lo.x, lo.y, hi.x, hi.y), (0, 0, 9, 0));
        assert!(
            visible_cells(
                Rect::new((-50.0, -50.0), (-1.0, -1.0)),
                cell,
                (10, 10),
                false
            )
            .is_none()
        );
    }

    #[test]
    fn placement_turns_and_stands_tall_tiles() {
        let plain = Tile {
            gid: 1,
            ..Tile::EMPTY
        };
        let (center, rotation, scale) = tile_placement(
            plain,
            Vec2::new(16.0, 0.0),
            Vec2::splat(16.0),
            Vec2::new(16.0, 32.0),
        );
        assert_eq!(center, Vec2::new(24.0, 0.0)); // stands on the cell's bottom
        assert_eq!((rotation, scale), (0.0, Vec2::ONE));
        let turned = Tile {
            flip_diagonal: true,
            flip_x: true,
            ..plain
        };
        let (_, rotation, scale) =
            tile_placement(turned, Vec2::ZERO, Vec2::splat(16.0), Vec2::splat(16.0));
        // Diagonal and left-right together is Tiled's quarter turn.
        assert_eq!(rotation, std::f32::consts::FRAC_PI_2);
        assert_eq!(scale, Vec2::ONE);
        let (_, _, scale) = tile_placement(
            Tile {
                flip_diagonal: true,
                ..plain
            },
            Vec2::ZERO,
            Vec2::splat(16.0),
            Vec2::splat(16.0),
        );
        assert_eq!(scale, Vec2::new(1.0, -1.0));
    }
}

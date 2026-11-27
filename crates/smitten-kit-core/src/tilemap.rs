//! Maps made in the [Tiled] editor, read from its JSON format (`.tmj` or
//! `.json`): tile layers, object layers, tilesets and custom properties.
//!
//! This module only reads the map. `smitten-kit`'s `tilemap` module loads
//! the tileset images and draws the layers; rules code and tests use the
//! [`TileMap`] alone, to find walls, spawn points and triggers.
//!
//! Orthogonal maps of a fixed size are supported, with tile data stored
//! as an array or as uncompressed base64, and tilesets embedded in the
//! map or saved beside it as JSON (`.tsj`). Infinite maps and compressed
//! layers give an error saying so.
//!
//! ```
//! use smitten_kit_core::glam::IVec2;
//! use smitten_kit_core::tilemap::TileMap;
//!
//! let json = r#"{
//!   "width": 3, "height": 2, "tilewidth": 16, "tileheight": 16,
//!   "orientation": "orthogonal",
//!   "layers": [
//!     { "type": "tilelayer", "name": "ground", "width": 3, "height": 2,
//!       "data": [0, 0, 2,  1, 1, 1] },
//!     { "type": "objectgroup", "name": "spawns",
//!       "objects": [{ "id": 1, "name": "player", "type": "spawn",
//!                     "x": 8, "y": 8, "width": 0, "height": 0, "point": true }] }
//!   ],
//!   "tilesets": [{
//!     "firstgid": 1, "name": "tiles", "image": "tiles.png",
//!     "imagewidth": 32, "imageheight": 16, "tilewidth": 16, "tileheight": 16,
//!     "columns": 2, "tilecount": 2,
//!     "tiles": [{ "id": 0, "properties": [{ "name": "solid", "type": "bool", "value": true }] }]
//!   }]
//! }"#;
//!
//! let map = TileMap::from_json(json)?;
//! assert!(map.is_solid(IVec2::new(0, 1)));   // tile 1 is marked solid
//! assert!(!map.is_solid(IVec2::new(2, 0)));  // tile 2 is not
//! let player = map.objects().find(|o| o.name == "player").unwrap();
//! assert_eq!(player.position.x, 8.0);
//! # Ok::<(), smitten_kit_core::tilemap::TiledError>(())
//! ```
//!
//! [Tiled]: https://www.mapeditor.org

use std::collections::HashMap;
use std::fmt;

use glam::{IVec2, UVec2, Vec2};
use serde::Deserialize;

use crate::grid::Grid;

/// Why a map could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TiledError {
    /// The text is not JSON of the shape Tiled writes.
    Json(String),
    /// The map uses something this reader does not handle.
    Unsupported(String),
    /// An external tileset could not be found or read.
    Tileset {
        /// The tileset's path, as the map gives it.
        source: String,
        /// What went wrong.
        reason: String,
    },
}

impl fmt::Display for TiledError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TiledError::Json(error) => write!(f, "not a Tiled JSON map: {error}"),
            TiledError::Unsupported(what) => write!(f, "{what} is not supported"),
            TiledError::Tileset { source, reason } => {
                write!(f, "tileset `{source}`: {reason}")
            }
        }
    }
}

impl std::error::Error for TiledError {}

/// One custom property's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    /// A `bool` property.
    Bool(bool),
    /// An `int` property, or an `object` property (the object's id).
    Int(i64),
    /// A `float` property.
    Float(f64),
    /// A `string`, `file` or `color` property (colors as `#AARRGGBB`).
    String(String),
}

/// The custom properties set on a map, layer, tile or object.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Properties(Vec<(String, Property)>);

impl Properties {
    /// The property called `name`.
    pub fn get(&self, name: &str) -> Option<&Property> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// A `bool` property, `false` when missing or not a bool.
    pub fn flag(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Property::Bool(true)))
    }

    /// A number property (int or float).
    pub fn number(&self, name: &str) -> Option<f64> {
        match self.get(name)? {
            Property::Int(n) => Some(*n as f64),
            Property::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// A string, file or color property.
    pub fn string(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Property::String(s) => Some(s),
            _ => None,
        }
    }

    /// Every property, in the order Tiled saved them.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Property)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v))
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One cell of a tile layer: which tile, and how it is turned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Tile {
    /// The tile's global id across every tileset, 0 for an empty cell.
    pub gid: u32,
    /// Mirrored left to right.
    pub flip_x: bool,
    /// Mirrored top to bottom.
    pub flip_y: bool,
    /// Mirrored across the top-left to bottom-right diagonal (applied
    /// before the other two). With `flip_x` it turns the tile 90°.
    pub flip_diagonal: bool,
}

const FLIP_X: u32 = 0x8000_0000;
const FLIP_Y: u32 = 0x4000_0000;
const FLIP_DIAGONAL: u32 = 0x2000_0000;
const FLIP_HEX: u32 = 0x1000_0000;

impl Tile {
    /// An empty cell.
    pub const EMPTY: Tile = Tile {
        gid: 0,
        flip_x: false,
        flip_y: false,
        flip_diagonal: false,
    };

    /// A tile from the number Tiled stores: the gid with flip flags in the
    /// top bits.
    pub fn from_raw(raw: u32) -> Tile {
        Tile {
            gid: raw & !(FLIP_X | FLIP_Y | FLIP_DIAGONAL | FLIP_HEX),
            flip_x: raw & FLIP_X != 0,
            flip_y: raw & FLIP_Y != 0,
            flip_diagonal: raw & FLIP_DIAGONAL != 0,
        }
    }

    /// Whether the cell is empty.
    pub fn is_empty(&self) -> bool {
        self.gid == 0
    }
}

/// A frame of an animated tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileFrame {
    /// The tile shown, as an id within the same tileset.
    pub tile: u32,
    /// For how long, in seconds.
    pub seconds: f32,
}

/// What a tileset says about one of its tiles.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TileInfo {
    /// The tile's class (its "type" before Tiled 1.9).
    pub class: String,
    /// Its custom properties.
    pub properties: Properties,
    /// Its animation, empty for a still tile.
    pub animation: Vec<TileFrame>,
}

/// A tileset: one image cut into tiles.
#[derive(Clone, Debug, PartialEq)]
pub struct Tileset {
    /// The global id of its first tile.
    pub first_gid: u32,
    /// Its name.
    pub name: String,
    /// The image's path, relative to the map's folder.
    pub image: String,
    /// The image's size in pixels.
    pub image_size: UVec2,
    /// The size of each tile in pixels.
    pub tile_size: UVec2,
    /// Tiles across the image.
    pub columns: u32,
    /// How many tiles it has.
    pub tile_count: u32,
    /// Pixels around the edge of the image.
    pub margin: u32,
    /// Pixels between tiles.
    pub spacing: u32,
    /// What it says about particular tiles, by id within the tileset.
    pub tiles: HashMap<u32, TileInfo>,
    /// Its custom properties.
    pub properties: Properties,
}

impl Tileset {
    /// Whether global id `gid` is one of this tileset's tiles.
    pub fn contains(&self, gid: u32) -> bool {
        gid >= self.first_gid && gid < self.first_gid + self.tile_count
    }

    /// Where tile `id` (within the tileset) sits in the image: its
    /// top-left corner in pixels.
    pub fn tile_position(&self, id: u32) -> UVec2 {
        let columns = self.columns.max(1);
        UVec2::new(
            self.margin + (id % columns) * (self.tile_size.x + self.spacing),
            self.margin + (id / columns) * (self.tile_size.y + self.spacing),
        )
    }
}

/// A shape on an object layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// A rectangle from the object's position, `size` big.
    Rect,
    /// An ellipse filling the object's rectangle.
    Ellipse,
    /// A single point at the object's position.
    Point,
    /// A closed shape, points relative to the object's position.
    Polygon(Vec<Vec2>),
    /// An open line, points relative to the object's position.
    Polyline(Vec<Vec2>),
    /// A tile placed as an object; its rectangle's bottom left is at the
    /// position, as Tiled draws it.
    Tile(Tile),
    /// A text box.
    Text(String),
}

/// One object on an object layer: a spawn point, a trigger, a door.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    /// Its id, unique in the map.
    pub id: u32,
    /// Its name.
    pub name: String,
    /// Its class (its "type" before Tiled 1.9).
    pub class: String,
    /// Its position in pixels.
    pub position: Vec2,
    /// Its size in pixels (zero for points).
    pub size: Vec2,
    /// Its rotation in degrees, clockwise.
    pub rotation: f32,
    /// Whether it is shown.
    pub visible: bool,
    /// Its shape.
    pub shape: Shape,
    /// Its custom properties.
    pub properties: Properties,
}

/// What a layer holds.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    /// A grid of tiles, the size of the map.
    Tiles(Grid<Tile>),
    /// Objects.
    Objects(Vec<Object>),
    /// One image.
    Image(String),
}

/// One layer of a map. Groups are flattened: their layers appear in
/// order, with names like `group/layer`, offsets added up, opacity
/// multiplied, and hidden if the group is.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Its name.
    pub name: String,
    /// What it holds.
    pub kind: LayerKind,
    /// Whether it is shown.
    pub visible: bool,
    /// From 0.0 (clear) to 1.0.
    pub opacity: f32,
    /// How far it is drawn from where it would be, in pixels.
    pub offset: Vec2,
    /// Its custom properties.
    pub properties: Properties,
}

impl Layer {
    /// Its tiles, if it is a tile layer.
    pub fn tiles(&self) -> Option<&Grid<Tile>> {
        match &self.kind {
            LayerKind::Tiles(grid) => Some(grid),
            _ => None,
        }
    }

    /// Its objects, if it is an object layer.
    pub fn objects(&self) -> &[Object] {
        match &self.kind {
            LayerKind::Objects(objects) => objects,
            _ => &[],
        }
    }
}

/// A map from Tiled. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct TileMap {
    /// Size in tiles.
    pub size: UVec2,
    /// Size of a grid cell in pixels.
    pub tile_size: UVec2,
    /// The layers, bottom first.
    pub layers: Vec<Layer>,
    /// The tilesets, by first gid.
    pub tilesets: Vec<Tileset>,
    /// The map's own custom properties.
    pub properties: Properties,
}

impl TileMap {
    /// Reads a map whose tilesets are all embedded in it.
    pub fn from_json(json: &str) -> Result<TileMap, TiledError> {
        TileMap::from_json_with(json, |source| {
            Err(format!("external tileset `{source}` needs a loader"))
        })
    }

    /// Reads a map, asking `tileset` for the text of each external
    /// tileset by the path the map gives (relative to the map's folder).
    pub fn from_json_with(
        json: &str,
        mut tileset: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<TileMap, TiledError> {
        let raw: RawMap =
            serde_json::from_str(json).map_err(|e| TiledError::Json(e.to_string()))?;
        if raw.infinite {
            return Err(TiledError::Unsupported("an infinite map".into()));
        }
        if !matches!(raw.orientation.as_str(), "orthogonal" | "") {
            return Err(TiledError::Unsupported(format!(
                "a {} map",
                raw.orientation
            )));
        }
        let mut tilesets = Vec::new();
        for entry in raw.tilesets {
            let set = match &entry.source {
                Some(source) => {
                    let text = tileset(source).map_err(|reason| TiledError::Tileset {
                        source: source.clone(),
                        reason,
                    })?;
                    let mut set: RawTileset =
                        serde_json::from_str(&text).map_err(|e| TiledError::Tileset {
                            source: source.clone(),
                            reason: e.to_string(),
                        })?;
                    // The image is relative to the tileset file.
                    if let Some((folder, _)) = source.rsplit_once('/') {
                        set.image = format!("{folder}/{}", set.image);
                    }
                    set.firstgid = entry.firstgid;
                    set
                }
                None => entry,
            };
            tilesets.push(set.into_tileset());
        }
        tilesets.sort_by_key(|set| set.first_gid);

        let size = UVec2::new(raw.width, raw.height);
        let mut layers = Vec::new();
        flatten(raw.layers, size, "", true, 1.0, Vec2::ZERO, &mut layers)?;
        Ok(TileMap {
            size,
            tile_size: UVec2::new(raw.tilewidth, raw.tileheight),
            layers,
            tilesets,
            properties: properties(raw.properties),
        })
    }

    /// Size in pixels.
    pub fn pixel_size(&self) -> Vec2 {
        (self.size * self.tile_size).as_vec2()
    }

    /// The layer called `name`.
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.name == name)
    }

    /// Every object on every object layer.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.layers.iter().flat_map(Layer::objects)
    }

    /// The tileset holding global id `gid`, and the tile's id within it.
    pub fn tileset_of(&self, gid: u32) -> Option<(&Tileset, u32)> {
        let set = self
            .tilesets
            .iter()
            .rev()
            .find(|set| set.first_gid <= gid)?;
        set.contains(gid).then(|| (set, gid - set.first_gid))
    }

    /// What its tileset says about tile `gid`, if anything.
    pub fn tile_info(&self, gid: u32) -> Option<&TileInfo> {
        let (set, id) = self.tileset_of(gid)?;
        set.tiles.get(&id)
    }

    /// The grid cell holding pixel `point`.
    pub fn cell_at(&self, point: Vec2) -> IVec2 {
        (point / self.tile_size.as_vec2()).floor().as_ivec2()
    }

    /// Whether something solid fills `cell`. By convention a cell is
    /// solid when a tile layer has a tile there and either the tile has a
    /// `solid` property set true in its tileset, or the layer has one.
    /// Cells off the map are solid, so nothing walks off the edge.
    pub fn is_solid(&self, cell: IVec2) -> bool {
        if cell.x < 0 || cell.y < 0 || cell.x >= self.size.x as i32 || cell.y >= self.size.y as i32
        {
            return true;
        }
        self.layers.iter().any(|layer| {
            let Some(tiles) = layer.tiles() else {
                return false;
            };
            let tile = tiles[cell];
            !tile.is_empty()
                && (layer.properties.flag("solid")
                    || self
                        .tile_info(tile.gid)
                        .is_some_and(|info| info.properties.flag("solid")))
        })
    }

    /// [`is_solid`](TileMap::is_solid) for every cell, worked out once.
    pub fn solid_grid(&self) -> Grid<bool> {
        Grid::from_fn(self.size.x, self.size.y, |cell| self.is_solid(cell))
    }
}

fn flatten(
    raw: Vec<RawLayer>,
    size: UVec2,
    prefix: &str,
    visible: bool,
    opacity: f32,
    offset: Vec2,
    out: &mut Vec<Layer>,
) -> Result<(), TiledError> {
    for layer in raw {
        let name = format!("{prefix}{}", layer.name);
        let shown = visible && layer.visible;
        let alpha = opacity * layer.opacity;
        let shift = offset + Vec2::new(layer.offsetx, layer.offsety);
        let kind = match layer.kind.as_str() {
            "group" => {
                flatten(
                    layer.layers,
                    size,
                    &format!("{name}/"),
                    shown,
                    alpha,
                    shift,
                    out,
                )?;
                continue;
            }
            "tilelayer" => {
                if layer.chunks.is_some() {
                    return Err(TiledError::Unsupported("an infinite map".into()));
                }
                let raw = tile_data(&layer)?;
                let (w, h) = (
                    layer.width.unwrap_or(size.x),
                    layer.height.unwrap_or(size.y),
                );
                let tiles = raw.into_iter().map(Tile::from_raw).collect();
                let grid = Grid::from_vec(w, h, tiles).ok_or_else(|| {
                    TiledError::Json(format!("layer `{name}` does not hold {w}x{h} tiles"))
                })?;
                LayerKind::Tiles(grid)
            }
            "objectgroup" => LayerKind::Objects(layer.objects.into_iter().map(object).collect()),
            "imagelayer" => LayerKind::Image(layer.image.unwrap_or_default()),
            other => return Err(TiledError::Unsupported(format!("a `{other}` layer"))),
        };
        out.push(Layer {
            name,
            kind,
            visible: shown,
            opacity: alpha,
            offset: shift,
            properties: properties(layer.properties),
        });
    }
    Ok(())
}

fn tile_data(layer: &RawLayer) -> Result<Vec<u32>, TiledError> {
    match &layer.data {
        None => Ok(Vec::new()),
        Some(Data::Array(values)) => Ok(values.clone()),
        Some(Data::Encoded(text)) => {
            if layer.compression.as_deref().is_some_and(|c| !c.is_empty()) {
                return Err(TiledError::Unsupported(format!(
                    "a {}-compressed layer (save it as CSV or uncompressed base64)",
                    layer.compression.as_deref().unwrap_or("")
                )));
            }
            let bytes = base64(text).ok_or_else(|| {
                TiledError::Json(format!("layer `{}` has broken base64 data", layer.name))
            })?;
            Ok(bytes
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect())
        }
    }
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn properties(raw: Vec<RawProperty>) -> Properties {
    Properties(
        raw.into_iter()
            .map(|p| {
                let value = match p.value {
                    serde_json::Value::Bool(b) => Property::Bool(b),
                    serde_json::Value::Number(n) if p.kind == "float" => {
                        Property::Float(n.as_f64().unwrap_or(0.0))
                    }
                    serde_json::Value::Number(n) => match n.as_i64() {
                        Some(i) => Property::Int(i),
                        None => Property::Float(n.as_f64().unwrap_or(0.0)),
                    },
                    serde_json::Value::String(s) => Property::String(s),
                    other => Property::String(other.to_string()),
                };
                (p.name, value)
            })
            .collect(),
    )
}

fn points(raw: Vec<RawPoint>) -> Vec<Vec2> {
    raw.into_iter().map(|p| Vec2::new(p.x, p.y)).collect()
}

fn object(raw: RawObject) -> Object {
    let shape = if let Some(gid) = raw.gid {
        Shape::Tile(Tile::from_raw(gid))
    } else if raw.point {
        Shape::Point
    } else if raw.ellipse {
        Shape::Ellipse
    } else if let Some(polygon) = raw.polygon {
        Shape::Polygon(points(polygon))
    } else if let Some(polyline) = raw.polyline {
        Shape::Polyline(points(polyline))
    } else if let Some(text) = raw.text {
        Shape::Text(text.text)
    } else {
        Shape::Rect
    };
    Object {
        id: raw.id,
        name: raw.name,
        class: if raw.class.is_empty() {
            raw.kind
        } else {
            raw.class
        },
        position: Vec2::new(raw.x, raw.y),
        size: Vec2::new(raw.width, raw.height),
        rotation: raw.rotation,
        visible: raw.visible,
        shape,
        properties: properties(raw.properties),
    }
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
struct RawMap {
    width: u32,
    height: u32,
    tilewidth: u32,
    tileheight: u32,
    #[serde(default)]
    orientation: String,
    #[serde(default)]
    infinite: bool,
    #[serde(default)]
    layers: Vec<RawLayer>,
    #[serde(default)]
    tilesets: Vec<RawTileset>,
    #[serde(default)]
    properties: Vec<RawProperty>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Data {
    Array(Vec<u32>),
    Encoded(String),
}

#[derive(Deserialize)]
struct RawLayer {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    name: String,
    width: Option<u32>,
    height: Option<u32>,
    data: Option<Data>,
    compression: Option<String>,
    chunks: Option<serde_json::Value>,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    offsetx: f32,
    #[serde(default)]
    offsety: f32,
    #[serde(default)]
    objects: Vec<RawObject>,
    #[serde(default)]
    layers: Vec<RawLayer>,
    image: Option<String>,
    #[serde(default)]
    properties: Vec<RawProperty>,
}

#[derive(Deserialize)]
struct RawProperty {
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    value: serde_json::Value,
}

#[derive(Deserialize)]
struct RawPoint {
    x: f32,
    y: f32,
}

#[derive(Deserialize)]
struct RawText {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct RawObject {
    #[serde(default)]
    id: u32,
    #[serde(default)]
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    width: f32,
    #[serde(default)]
    height: f32,
    #[serde(default)]
    rotation: f32,
    #[serde(default = "yes")]
    visible: bool,
    gid: Option<u32>,
    #[serde(default)]
    point: bool,
    #[serde(default)]
    ellipse: bool,
    polygon: Option<Vec<RawPoint>>,
    polyline: Option<Vec<RawPoint>>,
    text: Option<RawText>,
    #[serde(default)]
    properties: Vec<RawProperty>,
}

#[derive(Deserialize)]
struct RawFrame {
    tileid: u32,
    duration: u32,
}

#[derive(Deserialize)]
struct RawTile {
    id: u32,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    properties: Vec<RawProperty>,
    #[serde(default)]
    animation: Vec<RawFrame>,
}

#[derive(Deserialize)]
struct RawTileset {
    #[serde(default)]
    firstgid: u32,
    source: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    image: String,
    #[serde(default)]
    imagewidth: u32,
    #[serde(default)]
    imageheight: u32,
    #[serde(default)]
    tilewidth: u32,
    #[serde(default)]
    tileheight: u32,
    #[serde(default)]
    columns: u32,
    #[serde(default)]
    tilecount: u32,
    #[serde(default)]
    margin: u32,
    #[serde(default)]
    spacing: u32,
    #[serde(default)]
    tiles: Vec<RawTile>,
    #[serde(default)]
    properties: Vec<RawProperty>,
}

impl RawTileset {
    fn into_tileset(self) -> Tileset {
        Tileset {
            first_gid: self.firstgid,
            name: self.name,
            image: self.image,
            image_size: UVec2::new(self.imagewidth, self.imageheight),
            tile_size: UVec2::new(self.tilewidth, self.tileheight),
            columns: self.columns,
            tile_count: self.tilecount,
            margin: self.margin,
            spacing: self.spacing,
            tiles: self
                .tiles
                .into_iter()
                .map(|tile| {
                    (
                        tile.id,
                        TileInfo {
                            class: if tile.class.is_empty() {
                                tile.kind
                            } else {
                                tile.class
                            },
                            properties: properties(tile.properties),
                            animation: tile
                                .animation
                                .into_iter()
                                .map(|frame| TileFrame {
                                    tile: frame.tileid,
                                    seconds: frame.duration as f32 / 1000.0,
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
            properties: properties(self.properties),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = r#"{
      "width": 2, "height": 2, "tilewidth": 8, "tileheight": 8,
      "orientation": "orthogonal", "infinite": false,
      "properties": [{ "name": "music", "type": "file", "value": "cave.ogg" }],
      "layers": [
        { "type": "group", "name": "world", "offsetx": 4, "opacity": 0.5, "layers": [
          { "type": "tilelayer", "name": "walls", "width": 2, "height": 2,
            "encoding": "base64", "data": "AQAAAAAAAAAAAACAAgAAAA==",
            "properties": [{ "name": "solid", "type": "bool", "value": true }] }
        ]},
        { "type": "objectgroup", "name": "things", "visible": false, "objects": [
          { "id": 3, "name": "door", "class": "exit", "x": 1, "y": 2, "width": 8, "height": 16,
            "properties": [{ "name": "to", "type": "string", "value": "level2" },
                           { "name": "speed", "type": "float", "value": 2 }] },
          { "id": 4, "name": "path", "x": 0, "y": 0, "polyline": [{"x":0,"y":0},{"x":5,"y":5}] }
        ]}
      ],
      "tilesets": [{ "firstgid": 1, "source": "art/tiles.tsj" }]
    }"#;

    const TILESET: &str = r#"{
      "name": "tiles", "image": "tiles.png", "imagewidth": 20, "imageheight": 10,
      "tilewidth": 8, "tileheight": 8, "columns": 2, "tilecount": 2, "margin": 1, "spacing": 2,
      "tiles": [{ "id": 1, "animation": [{ "tileid": 0, "duration": 100 }, { "tileid": 1, "duration": 250 }] }]
    }"#;

    fn load() -> TileMap {
        TileMap::from_json_with(MAP, |source| {
            assert_eq!(source, "art/tiles.tsj");
            Ok(TILESET.to_owned())
        })
        .unwrap()
    }

    #[test]
    fn groups_flatten_and_base64_decodes() {
        let map = load();
        let walls = map.layer("world/walls").unwrap();
        assert_eq!(walls.offset, Vec2::new(4.0, 0.0));
        assert_eq!(walls.opacity, 0.5);
        let tiles = walls.tiles().unwrap();
        assert_eq!(tiles[IVec2::new(0, 0)].gid, 1);
        assert!(tiles[IVec2::new(1, 0)].is_empty());
        // 0x80000000: flipped left to right, no tile.
        assert!(tiles[IVec2::new(0, 1)].flip_x);
        assert_eq!(tiles[IVec2::new(1, 1)].gid, 2);
        assert_eq!(map.properties.string("music"), Some("cave.ogg"));
    }

    #[test]
    fn external_tilesets_and_animation() {
        let map = load();
        let (set, id) = map.tileset_of(2).unwrap();
        assert_eq!(id, 1);
        assert_eq!(set.image, "art/tiles.png");
        assert_eq!(set.tile_position(1), UVec2::new(11, 1));
        let info = map.tile_info(2).unwrap();
        assert_eq!(info.animation[1].seconds, 0.25);
        assert!(map.tileset_of(3).is_none());
        assert!(TileMap::from_json(MAP).is_err());
    }

    #[test]
    fn objects_and_solids() {
        let map = load();
        let door = map.objects().find(|o| o.name == "door").unwrap();
        assert_eq!(door.class, "exit");
        assert_eq!(door.shape, Shape::Rect);
        assert_eq!(door.properties.string("to"), Some("level2"));
        assert_eq!(door.properties.number("speed"), Some(2.0));
        assert!(!map.layer("things").unwrap().visible);
        let path = map.objects().find(|o| o.id == 4).unwrap();
        assert!(matches!(&path.shape, Shape::Polyline(p) if p.len() == 2));

        assert!(map.is_solid(IVec2::new(0, 0)));
        assert!(!map.is_solid(IVec2::new(1, 0)));
        assert!(map.is_solid(IVec2::new(-1, 0)));
        assert_eq!(map.solid_grid().iter().filter(|(_, s)| **s).count(), 2);
        assert_eq!(map.cell_at(Vec2::new(9.0, 3.0)), IVec2::new(1, 0));
    }

    #[test]
    fn unsupported_maps_say_why() {
        let infinite = r#"{"width":1,"height":1,"tilewidth":1,"tileheight":1,"infinite":true}"#;
        assert_eq!(
            TileMap::from_json(infinite),
            Err(TiledError::Unsupported("an infinite map".into()))
        );
        let iso =
            r#"{"width":1,"height":1,"tilewidth":1,"tileheight":1,"orientation":"isometric"}"#;
        assert!(matches!(
            TileMap::from_json(iso),
            Err(TiledError::Unsupported(_))
        ));
        assert!(matches!(TileMap::from_json("{"), Err(TiledError::Json(_))));
    }
}

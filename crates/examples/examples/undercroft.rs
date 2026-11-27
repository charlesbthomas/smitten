//! Undercroft, a small turn-based dungeon crawl.
//!
//! Each floor of the undercroft is built from a seed: rooms are scattered,
//! joined by corridors, and stocked with gold, tonics and things that bite.
//! Find the stairs and go deeper. Monsters that see you follow you around
//! corners, and the deeper you go the more of them there are.
//!
//! - W, A, S, D, the arrow keys or the number pad walk. Walking into a
//!   monster attacks it.
//! - Click a tile you have seen to walk there.
//! - Space waits a turn, Q drinks a tonic, and Return takes the stairs.
//! - F5 saves, F9 loads, and Escape saves and quits. The game also saves
//!   on every new floor, and picks up where you left off next time.
//! - P shows the paths the monsters that are after you plan to take.
//! - R starts a new run after you fall. `--seed 1234` on the command
//!   line starts a new run on the floors that seed builds.
//!
//! Besides being a game, this shows several parts of Smitten together:
//!
//! - The tiles and creatures are pixel art painted into an `ImageData` in
//!   code, and each floor's tiles go into one `SpriteBatch`, so the whole
//!   map is a single draw.
//! - The fog of war is a light map on a canvas. Your torch and the
//!   braziers in some rooms add their glow, walls cut shadows out of it
//!   with the stencil, places you have seen before stay dimly lit, and
//!   places you have never seen stay black.
//! - Monsters and click-to-walk find their way with A* search.
//! - The run is saved as a small text file in the save directory, with
//!   `filesystem::write`, and each floor is rebuilt from its seed.

use smitten::graphics::{
    Align, Canvas, DrawMode, DrawParams, Font, Image, ImageData, Quad, Shader, SpriteBatch,
    StencilWrite,
};
use smitten::math::RandomGenerator;
use smitten::{
    BlendMode, Color, CompareMode, Config, Filter, Game, Key, MouseButton, MouseClick, Vec2, event,
    filesystem, graphics, keyboard, math, timer,
};
use smitten_kit::grid::astar;

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 640.0;

/// Tiles are painted 16 pixels square and drawn twice that size.
const ART: u32 = 16;
const SCALE: f32 = 2.0;
const TILE: f32 = ART as f32 * SCALE;

const MAP_WIDTH: i32 = 56;
const MAP_HEIGHT: i32 = 38;

/// How many tiles your torch lets you see, and how far its glow reaches.
const SIGHT: i32 = 7;
const TORCH_REACH: f32 = 270.0;
const BRAZIER_REACH: f32 = 230.0;
/// How far you can see into a room a brazier lights.
const FAR_SIGHT: i32 = 18;
const SHADOW_LENGTH: f32 = 3000.0;

/// Seconds between steps while a key is held, and before the first repeat.
const STEP_REPEAT: f32 = 0.12;
const STEP_DELAY: f32 = 0.22;
/// Seconds between steps while walking to a clicked tile.
const TRAVEL_STEP: f32 = 0.07;
/// How many tiles A* may look at before giving up.
const SEARCH_LIMIT: usize = 1500;

const SAVE_FILE: &str = "undercroft-save.txt";
const SAVE_HEADER: &str = "undercroft-save 1";

/// The same round glow as the lighting example: strongest in the middle
/// and gone at the edge of the circle that fits in the rectangle drawn.
const GLOW: &str = "
struct Params {
    falloff: f32,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let distance_from_center = distance(uv, vec2<f32>(0.5, 0.5)) * 2.0;
    let strength = pow(clamp(1.0 - distance_from_center, 0.0, 1.0), params.falloff);
    return vec4<f32>(color.rgb, color.a * strength);
}
";

// ---------------------------------------------------------------------------
// The art. Every picture is a function from a pixel to a color, painted
// once into a sheet of 8 by 2 pictures.

/// Where each picture is on the sheet, counted left to right, top to bottom.
mod art {
    pub const FLOOR: [u32; 3] = [0, 1, 2];
    pub const WALL_TOP: u32 = 3;
    pub const WALL_FACE: u32 = 4;
    pub const STAIRS: u32 = 5;
    pub const BRAZIER: u32 = 6;
    pub const BONES: u32 = 7;
    pub const HERO: u32 = 8;
    pub const GNAWER: u32 = 9;
    pub const HUSK: u32 = 10;
    pub const WARDEN: u32 = 11;
    pub const TONIC: u32 = 12;
    pub const GOLD: u32 = 13;
}

const SHEET_COLUMNS: u32 = 8;
const SHEET_ROWS: u32 = 2;

fn rgb(r: f32, g: f32, b: f32) -> Option<Color> {
    Some(Color::rgb(r, g, b))
}

/// A pixel's distance from a point, measured from the pixel's middle.
fn dist(x: u32, y: u32, cx: f32, cy: f32) -> f32 {
    ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt()
}

fn floor_pixel(variant: u32, x: u32, y: u32) -> Option<Color> {
    // Flagstones in a 2 by 2 pattern, shifted on alternate rows, with
    // darker mortar between them.
    let (sx, sy) = if y < 8 { (x, y) } else { ((x + 4) % 16, y - 8) };
    if sx % 8 == 0 || sy == 0 {
        return rgb(0.16, 0.14, 0.15);
    }
    let stone = (sx / 8) + (y / 8) * 2 + variant * 4;
    let grain = math::noise2(x as f32 * 0.9 + variant as f32 * 7.0, y as f32 * 0.9) * 0.05;
    let shade = 0.44 + (stone % 3) as f32 * 0.03 + grain;
    // A crack across some of the stones.
    if variant == 1 && x + 3 == y + 2 && (4..12).contains(&x) {
        return rgb(0.2, 0.18, 0.18);
    }
    if variant == 2 && (x * 7 + y * 3).is_multiple_of(23) {
        return rgb(0.28, 0.36, 0.22); // moss
    }
    rgb(shade, shade * 0.95, shade * 0.9)
}

fn wall_top_pixel(x: u32, y: u32) -> Option<Color> {
    // Rough dark rock with a raised rim, so it reads as the top of a wall
    // and not as more floor.
    let grain = math::noise2(x as f32 * 0.7 + 40.0, y as f32 * 0.7) * 0.05;
    let rim = if x == 0 || y == 0 {
        0.08
    } else if x == 15 || y == 15 {
        -0.05
    } else {
        0.0
    };
    let shade = 0.12 + grain + rim;
    rgb(shade * 0.95, shade * 0.85, shade * 1.25)
}

fn wall_face_pixel(x: u32, y: u32) -> Option<Color> {
    // Bricks four pixels tall, staggered on alternate courses.
    if y < 2 {
        return rgb(0.32, 0.29, 0.36); // the lip at the top
    }
    let course = (y - 2) / 4;
    let shift = if course.is_multiple_of(2) { 0 } else { 4 };
    if (y - 2) % 4 == 3 || (x + shift).is_multiple_of(8) {
        return rgb(0.12, 0.1, 0.13);
    }
    let brick = (x + shift) / 8 + course * 3;
    let shade = 0.25 + (brick % 3) as f32 * 0.03 + math::noise2(x as f32, y as f32 * 1.3) * 0.03;
    // The courses get darker toward the floor.
    let shade = shade - y as f32 * 0.006;
    rgb(shade * 1.05, shade * 0.95, shade * 1.15)
}

fn stairs_pixel(x: u32, y: u32) -> Option<Color> {
    if !(2..=13).contains(&x) || y < 2 {
        return floor_pixel(0, x, y);
    }
    // Steps going down and away, each one darker.
    let step = (y - 2) / 3;
    if (y - 2).is_multiple_of(3) {
        return rgb(0.5, 0.48, 0.5);
    }
    let shade = 0.3 - step as f32 * 0.05;
    rgb(shade, shade, shade * 1.1)
}

fn brazier_pixel(x: u32, y: u32) -> Option<Color> {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    // The flame: a teardrop above the bowl, hottest at its root.
    let flame_width = (fy - 2.0).max(0.0) * 0.45;
    if fy < 9.0 && (fx - 8.0).abs() < flame_width.min(4.0) {
        let heat = (fy - 2.0) / 7.0;
        return rgb(1.0, 0.45 + heat * 0.5, 0.1 + heat * 0.5);
    }
    // The bowl and its stand.
    if (9..12).contains(&y) && (3..13).contains(&x) {
        return rgb(0.45, 0.32, 0.18);
    }
    if y >= 12 && (7..9).contains(&x) {
        return rgb(0.3, 0.22, 0.14);
    }
    if y == 15 && (5..11).contains(&x) {
        return rgb(0.3, 0.22, 0.14);
    }
    floor_pixel(2, x, y)
}

fn bones_pixel(x: u32, y: u32) -> Option<Color> {
    let bone = rgb(0.82, 0.78, 0.68);
    if dist(x, y, 6.0, 7.0) < 2.6 && !(dist(x, y, 5.2, 6.8) < 0.8 || dist(x, y, 7.0, 6.8) < 0.8) {
        return bone; // a skull with two eyes
    }
    if y == 11 && (3..13).contains(&x) || x + y == 22 && (8..14).contains(&x) {
        return bone;
    }
    floor_pixel(1, x, y)
}

fn hero_pixel(x: u32, y: u32) -> Option<Color> {
    // A hooded figure with a sword in the right hand.
    if x == 13 && (3..11).contains(&y) {
        return rgb(0.85, 0.88, 0.95);
    }
    if y == 10 && (12..15).contains(&x) {
        return rgb(0.7, 0.55, 0.2);
    }
    if dist(x, y, 8.0, 5.0) < 3.2 {
        if y >= 5 && (6..10).contains(&x) {
            return rgb(0.95, 0.8, 0.65); // the face
        }
        return rgb(0.25, 0.45, 0.75); // the hood
    }
    if (8..15).contains(&y) {
        let half = (y as f32 - 6.0) * 0.55;
        if (x as f32 + 0.5 - 8.0).abs() < half {
            let shade = if x < 8 { 0.85 } else { 1.0 };
            return rgb(0.2 * shade, 0.38 * shade, 0.7 * shade);
        }
    }
    None
}

fn gnawer_pixel(x: u32, y: u32) -> Option<Color> {
    // A fat rat facing left.
    if dist(x, y, 9.0, 10.0) < 4.5 && y <= 13 {
        return rgb(0.5, 0.36, 0.26);
    }
    if dist(x, y, 4.0, 9.0) < 2.6 {
        if x == 3 && y == 8 {
            return rgb(1.0, 0.2, 0.15);
        }
        return rgb(0.56, 0.42, 0.32);
    }
    if dist(x, y, 6.0, 5.8) < 1.4 {
        return rgb(0.8, 0.55, 0.55); // an ear
    }
    if y == 12 + x / 8 && x > 12 {
        return rgb(0.8, 0.6, 0.6); // the tail
    }
    None
}

fn husk_pixel(x: u32, y: u32) -> Option<Color> {
    // A pale, hunched thing with its arms hanging forward.
    let skin = (0.6, 0.7, 0.5);
    if dist(x, y, 6.0, 5.0) < 2.8 {
        if (x == 5 || x == 7) && y == 5 {
            return rgb(0.1, 0.05, 0.05);
        }
        return rgb(skin.0, skin.1, skin.2);
    }
    if dist(x, y, 9.0, 10.0) < 4.0 && y < 14 {
        return rgb(0.32, 0.3, 0.26); // rags
    }
    if (x == 4 || x == 5) && (8..13).contains(&y) {
        return rgb(skin.0 * 0.9, skin.1 * 0.9, skin.2 * 0.9);
    }
    if (y == 14 || y == 15) && (x == 7 || x == 11) {
        return rgb(skin.0 * 0.8, skin.1 * 0.8, skin.2 * 0.8);
    }
    None
}

fn warden_pixel(x: u32, y: u32) -> Option<Color> {
    // A horned knight in dark plate with a glowing visor.
    if (y == 1 || y == 2) && (x == 3 || x == 12) || y == 3 && (x == 4 || x == 11) {
        return rgb(0.85, 0.8, 0.7);
    }
    if (3..8).contains(&y) && (5..11).contains(&x) {
        if y == 5 && (6..10).contains(&x) {
            return rgb(1.0, 0.3, 0.6);
        }
        return rgb(0.32, 0.25, 0.42);
    }
    if (8..14).contains(&y) && (3..13).contains(&x) {
        let shade = if (x + y).is_multiple_of(5) { 0.5 } else { 0.38 };
        return rgb(shade * 0.85, shade * 0.7, shade * 1.1);
    }
    if (14..16).contains(&y) && (x == 5 || x == 6 || x == 9 || x == 10) {
        return rgb(0.25, 0.2, 0.3);
    }
    None
}

fn tonic_pixel(x: u32, y: u32) -> Option<Color> {
    if (3..6).contains(&y) && (7..9).contains(&x) {
        return rgb(0.65, 0.5, 0.3); // the cork
    }
    if (6..8).contains(&y) && (7..9).contains(&x) {
        return rgb(0.75, 0.8, 0.85);
    }
    if dist(x, y, 8.0, 11.0) < 4.0 {
        if dist(x, y, 6.6, 9.8) < 1.0 {
            return rgb(1.0, 0.8, 0.8); // a shine on the glass
        }
        return rgb(0.85, 0.15, 0.25);
    }
    None
}

fn gold_pixel(x: u32, y: u32) -> Option<Color> {
    for (cx, cy) in [(5.0, 12.0), (10.5, 12.5), (8.0, 9.5)] {
        let d = dist(x, y, cx, cy);
        if d < 2.8 {
            let shine = if d < 1.2 { 0.15 } else { 0.0 };
            return rgb(0.95 + shine * 0.3, 0.75 + shine, 0.2 + shine);
        }
        if d < 3.4 {
            return rgb(0.6, 0.42, 0.1);
        }
    }
    None
}

/// Paints every picture into one sheet.
fn paint_sheet() -> ImageData {
    let mut sheet = ImageData::new(ART * SHEET_COLUMNS, ART * SHEET_ROWS);
    sheet.map_pixels(|x, y, _| {
        let picture = (y / ART) * SHEET_COLUMNS + x / ART;
        let (x, y) = (x % ART, y % ART);
        let painted = match picture {
            art::WALL_TOP => wall_top_pixel(x, y),
            art::WALL_FACE => wall_face_pixel(x, y),
            art::STAIRS => stairs_pixel(x, y),
            art::BRAZIER => brazier_pixel(x, y),
            art::BONES => bones_pixel(x, y),
            art::HERO => hero_pixel(x, y),
            art::GNAWER => gnawer_pixel(x, y),
            art::HUSK => husk_pixel(x, y),
            art::WARDEN => warden_pixel(x, y),
            art::TONIC => tonic_pixel(x, y),
            art::GOLD => gold_pixel(x, y),
            floor => floor_pixel(floor, x, y),
        };
        painted.unwrap_or(Color::rgba(0.0, 0.0, 0.0, 0.0))
    });
    sheet
}

fn sprite_quad(index: u32) -> Quad {
    Quad::new(
        (
            ((index % SHEET_COLUMNS) * ART) as f32,
            ((index / SHEET_COLUMNS) * ART) as f32,
        ),
        (ART as f32, ART as f32),
        ((ART * SHEET_COLUMNS) as f32, (ART * SHEET_ROWS) as f32),
    )
}

// ---------------------------------------------------------------------------
// The map.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tile {
    Wall,
    Floor(u8),
    Bones,
    Stairs,
    Brazier,
}

impl Tile {
    fn blocks_sight(self) -> bool {
        self == Tile::Wall
    }

    fn blocks_walking(self) -> bool {
        matches!(self, Tile::Wall | Tile::Brazier)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    fn offset(self, dx: i32, dy: i32) -> Point {
        Point::new(self.x + dx, self.y + dy)
    }

    /// Steps when only moving along the grid, never diagonally.
    fn steps_to(self, other: Point) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }

    /// The middle of the tile, in pixels.
    fn center(self) -> Vec2 {
        Vec2::new((self.x as f32 + 0.5) * TILE, (self.y as f32 + 0.5) * TILE)
    }

    fn corner(self) -> Vec2 {
        Vec2::new(self.x as f32 * TILE, self.y as f32 * TILE)
    }
}

#[derive(Clone, Copy, Debug)]
struct Room {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Room {
    fn center(&self) -> Point {
        Point::new(self.x + self.width / 2, self.y + self.height / 2)
    }

    /// Whether this room, with a wall's width around it, overlaps `other`.
    fn crowds(&self, other: &Room) -> bool {
        self.x - 2 < other.x + other.width
            && other.x - 2 < self.x + self.width
            && self.y - 2 < other.y + other.height
            && other.y - 2 < self.y + self.height
    }
}

struct Map {
    tiles: Vec<Tile>,
}

impl Map {
    fn index(point: Point) -> Option<usize> {
        (point.x >= 0 && point.y >= 0 && point.x < MAP_WIDTH && point.y < MAP_HEIGHT)
            .then(|| (point.y * MAP_WIDTH + point.x) as usize)
    }

    fn point(index: usize) -> Point {
        Point::new(index as i32 % MAP_WIDTH, index as i32 / MAP_WIDTH)
    }

    fn get(&self, point: Point) -> Tile {
        Map::index(point).map_or(Tile::Wall, |index| self.tiles[index])
    }

    fn set(&mut self, point: Point, tile: Tile) {
        if let Some(index) = Map::index(point) {
            self.tiles[index] = tile;
        }
    }

    /// Whether a straight line from `from` reaches `to` without passing
    /// through anything that blocks sight. The tiles at either end may
    /// block sight themselves, so walls can be seen.
    fn line_of_sight(&self, from: Point, to: Point) -> bool {
        // Bresenham's line.
        let (dx, dy) = ((to.x - from.x).abs(), -(to.y - from.y).abs());
        let (sx, sy) = ((to.x - from.x).signum(), (to.y - from.y).signum());
        let (mut x, mut y, mut error) = (from.x, from.y, dx + dy);
        loop {
            if x == to.x && y == to.y {
                return true;
            }
            if (x != from.x || y != from.y) && self.get(Point::new(x, y)).blocks_sight() {
                return false;
            }
            let doubled = error * 2;
            if doubled >= dy {
                error += dy;
                x += sx;
            }
            if doubled <= dx {
                error += dx;
                y += sy;
            }
        }
    }

    /// The tiles within `reach` steps of `from`, as the crow flies, that a
    /// line of sight reaches, and the walls around them.
    fn seen_from(&self, from: Point, reach: i32, mut mark: impl FnMut(usize)) {
        let mut seen = vec![false; self.tiles.len()];
        for y in from.y - reach..=from.y + reach {
            for x in from.x - reach..=from.x + reach {
                let to = Point::new(x, y);
                let (dx, dy) = (x - from.x, y - from.y);
                if dx * dx + dy * dy > reach * reach + reach {
                    continue;
                }
                // Bresenham's line from one end is not quite the line from
                // the other, so either one getting through will do.
                if let Some(index) = Map::index(to)
                    && (self.line_of_sight(from, to) || self.line_of_sight(to, from))
                {
                    seen[index] = true;
                }
            }
        }
        // A straight line to a wall along a corridor grazes the walls
        // before it, so it seldom gets through. Any wall beside an open
        // tile in sight counts as seen instead.
        for (index, &seen) in seen.iter().enumerate() {
            if !seen {
                continue;
            }
            mark(index);
            if self.tiles[index].blocks_sight() {
                continue;
            }
            let point = Map::point(index);
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
                (-1, -1),
            ] {
                if let Some(wall) = Map::index(point.offset(dx, dy))
                    && self.tiles[wall].blocks_sight()
                {
                    mark(wall);
                }
            }
        }
    }

    /// Finds the shortest walk from `from` to `to` with A* search (smitten-kit's `astar`), moving
    /// along the grid. `open` says which tiles may be walked through; the
    /// goal itself is always allowed, so a monster can path to the tile
    /// you stand on. Returns the steps after `from`, ending with `to`, or
    /// `None` when there is no way within [`SEARCH_LIMIT`] tiles.
    fn find_path(
        &self,
        from: Point,
        to: Point,
        open: impl Fn(Point) -> bool,
    ) -> Option<Vec<Point>> {
        Map::index(from)?;
        Map::index(to)?;
        astar(
            from,
            to,
            |here, steps| {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let next = here.offset(dx, dy);
                    let Some(index) = Map::index(next) else {
                        continue;
                    };
                    if next == to || (!self.tiles[index].blocks_walking() && open(next)) {
                        steps.push((next, 1));
                    }
                }
            },
            |point| point.steps_to(to).unsigned_abs(),
            SEARCH_LIMIT,
        )
    }

    /// Wall tiles next to anything open, as boxes for casting shadows,
    /// with neighbors in a row joined into one box.
    fn shadow_boxes_near(&self, center: Vec2, reach: f32) -> Vec<[Vec2; 4]> {
        let mut boxes = Vec::new();
        let low = |value: f32| ((value - reach) / TILE).floor() as i32;
        let high = |value: f32| ((value + reach) / TILE).ceil() as i32;
        for y in low(center.y).max(0)..high(center.y).min(MAP_HEIGHT) {
            let mut run_start = None;
            for x in low(center.x).max(-1)..=high(center.x).min(MAP_WIDTH) {
                let point = Point::new(x, y);
                let casts = self.get(point) == Tile::Wall && self.borders_open(point);
                match (casts, run_start) {
                    (true, None) => run_start = Some(x),
                    (false, Some(start)) => {
                        boxes.push(tile_box(start, x, y));
                        run_start = None;
                    }
                    _ => {}
                }
            }
            if let Some(start) = run_start {
                boxes.push(tile_box(start, high(center.x).min(MAP_WIDTH) + 1, y));
            }
        }
        boxes
    }

    fn borders_open(&self, point: Point) -> bool {
        (-1..=1).any(|dy| (-1..=1).any(|dx| self.get(point.offset(dx, dy)) != Tile::Wall))
    }
}

/// The corners of the tiles from `start` up to but not including `end`
/// along row `y`.
fn tile_box(start: i32, end: i32, y: i32) -> [Vec2; 4] {
    let (left, right) = (start as f32 * TILE, end as f32 * TILE);
    let (top, bottom) = (y as f32 * TILE, (y + 1) as f32 * TILE);
    [
        Vec2::new(left, top),
        Vec2::new(right, top),
        Vec2::new(right, bottom),
        Vec2::new(left, bottom),
    ]
}

// ---------------------------------------------------------------------------
// What lives in the map.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Gnawer,
    Husk,
    Warden,
}

struct Stats {
    name: &'static str,
    health: i32,
    attack: i32,
    /// Turns between moves: 1 acts every turn, 2 every other turn.
    pace: u32,
    /// How far away it notices you, in steps.
    senses: i32,
    worth: i32,
    picture: u32,
}

impl Kind {
    fn stats(self) -> Stats {
        match self {
            Kind::Gnawer => Stats {
                name: "gnawer",
                health: 4,
                attack: 2,
                pace: 1,
                senses: 8,
                worth: 1,
                picture: art::GNAWER,
            },
            Kind::Husk => Stats {
                name: "husk",
                health: 10,
                attack: 4,
                pace: 2,
                senses: 10,
                worth: 2,
                picture: art::HUSK,
            },
            Kind::Warden => Stats {
                name: "warden",
                health: 16,
                attack: 5,
                pace: 1,
                senses: 12,
                worth: 4,
                picture: art::WARDEN,
            },
        }
    }

    fn save_name(self) -> &'static str {
        self.stats().name
    }

    fn from_save_name(name: &str) -> Option<Kind> {
        [Kind::Gnawer, Kind::Husk, Kind::Warden]
            .into_iter()
            .find(|kind| kind.save_name() == name)
    }
}

struct Monster {
    kind: Kind,
    at: Point,
    health: i32,
    /// Whether it has noticed you and is giving chase.
    hunting: bool,
    /// Where it last saw you.
    last_seen: Option<Point>,
    /// The way it plans to go, kept to show with P.
    plan: Vec<Point>,
    /// Where it is drawn, which slides after `at`.
    shown: Vec2,
    /// A short push toward whatever it just attacked.
    lunge: Vec2,
    /// A flash of white when it is hit.
    flash: f32,
}

impl Monster {
    fn new(kind: Kind, at: Point) -> Monster {
        Monster {
            kind,
            at,
            health: kind.stats().health,
            hunting: false,
            last_seen: None,
            plan: Vec::new(),
            shown: at.corner(),
            lunge: Vec2::ZERO,
            flash: 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Loot {
    Gold(i32),
    Tonic,
}

struct Item {
    loot: Loot,
    at: Point,
}

struct Hero {
    at: Point,
    health: i32,
    max_health: i32,
    attack: i32,
    level: i32,
    experience: i32,
    gold: i32,
    tonics: i32,
    shown: Vec2,
    lunge: Vec2,
    flash: f32,
}

impl Hero {
    fn new(at: Point) -> Hero {
        Hero {
            at,
            health: 20,
            max_health: 20,
            attack: 4,
            level: 1,
            experience: 0,
            gold: 0,
            tonics: 1,
            shown: at.corner(),
            lunge: Vec2::ZERO,
            flash: 0.0,
        }
    }

    fn experience_to_level(&self) -> i32 {
        self.level * 4
    }
}

/// A floor as it comes out of the generator.
struct Floor {
    map: Map,
    start: Point,
    braziers: Vec<Point>,
    monsters: Vec<Monster>,
    items: Vec<Item>,
}

/// The seed for one floor of a run, so each floor is the same every time
/// it is built, and a saved run only needs to keep the run's seed.
fn floor_seed(run_seed: u64, depth: i32) -> u64 {
    run_seed ^ (depth as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Builds a floor: rooms scattered at random, each joined to the one
/// before it by a bent corridor, with a few more corridors so there are
/// loops to run around.
fn generate(run_seed: u64, depth: i32) -> Floor {
    let mut rng = RandomGenerator::new(floor_seed(run_seed, depth));
    let mut map = Map {
        tiles: vec![Tile::Wall; (MAP_WIDTH * MAP_HEIGHT) as usize],
    };

    let mut rooms: Vec<Room> = Vec::new();
    for _ in 0..200 {
        if rooms.len() >= 13 {
            break;
        }
        let width = rng.random_int(4, 10) as i32;
        let height = rng.random_int(3, 7) as i32;
        let room = Room {
            x: rng.random_int(1, (MAP_WIDTH - width - 2) as i64) as i32,
            y: rng.random_int(2, (MAP_HEIGHT - height - 2) as i64) as i32,
            width,
            height,
        };
        if rooms.iter().all(|other| !room.crowds(other)) {
            rooms.push(room);
        }
    }

    let floor_tile = |rng: &mut RandomGenerator| {
        let roll = rng.random();
        Tile::Floor(if roll < 0.7 {
            0
        } else if roll < 0.9 {
            1
        } else {
            2
        })
    };
    for room in &rooms {
        for y in room.y..room.y + room.height {
            for x in room.x..room.x + room.width {
                map.set(Point::new(x, y), floor_tile(&mut rng));
            }
        }
    }

    let dig = |map: &mut Map, from: Point, to: Point, rng: &mut RandomGenerator| {
        let corner = if rng.chance(0.5) {
            Point::new(to.x, from.y)
        } else {
            Point::new(from.x, to.y)
        };
        for (a, b) in [(from, corner), (corner, to)] {
            let mut at = a;
            loop {
                if map.get(at) == Tile::Wall {
                    map.set(at, floor_tile(rng));
                }
                if at == b {
                    break;
                }
                at = at.offset((b.x - at.x).signum(), (b.y - at.y).signum());
            }
        }
    };
    for pair in rooms.windows(2) {
        dig(&mut map, pair[0].center(), pair[1].center(), &mut rng);
    }
    for _ in 0..3 {
        let a = rng.random_int(0, rooms.len() as i64 - 1) as usize;
        let b = rng.random_int(0, rooms.len() as i64 - 1) as usize;
        if a != b {
            dig(&mut map, rooms[a].center(), rooms[b].center(), &mut rng);
        }
    }

    // The stairs go in the room that takes the longest walk to reach.
    let start = rooms[0].center();
    let walk_to = |map: &Map, to: Point| {
        map.find_path(start, to, |_| true)
            .map_or(0, |path| path.len())
    };
    let far_room = (1..rooms.len())
        .max_by_key(|&index| walk_to(&map, rooms[index].center()))
        .unwrap_or(0);
    let stairs = rooms[far_room].center();
    map.set(stairs, Tile::Stairs);

    let mut braziers = Vec::new();
    let mut monsters: Vec<Monster> = Vec::new();
    let mut items = Vec::new();
    for (index, room) in rooms.iter().enumerate() {
        let free_spot = |map: &Map, rng: &mut RandomGenerator| {
            for _ in 0..20 {
                let spot = Point::new(
                    rng.random_int(room.x as i64, (room.x + room.width - 1) as i64) as i32,
                    rng.random_int(room.y as i64, (room.y + room.height - 1) as i64) as i32,
                );
                if matches!(map.get(spot), Tile::Floor(_)) && spot != start {
                    return Some(spot);
                }
            }
            None
        };

        // A brazier stands against a room's top wall, where it does not
        // block the corridors that come in from the sides.
        if index != 0 && room.width >= 5 && rng.chance(0.45) {
            let spot = Point::new(
                rng.random_int((room.x + 1) as i64, (room.x + room.width - 2) as i64) as i32,
                room.y,
            );
            // It must not cut off a corridor: it needs the wall behind it
            // and room to walk around it in front.
            let around = [(-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
            if map.get(spot.offset(0, -1)) == Tile::Wall
                && around
                    .iter()
                    .all(|&(dx, dy)| matches!(map.get(spot.offset(dx, dy)), Tile::Floor(_)))
            {
                map.set(spot, Tile::Brazier);
                braziers.push(spot);
            }
        }
        if rng.chance(0.3)
            && let Some(spot) = free_spot(&map, &mut rng)
        {
            map.set(spot, Tile::Bones);
        }

        if index != 0 {
            let crowd = rng.random_int(0, 1 + (depth as i64 + 1) / 2).min(4);
            for _ in 0..crowd {
                let roll = rng.random() + depth as f32 * 0.08;
                let kind = if roll > 1.05 && depth >= 3 {
                    Kind::Warden
                } else if roll > 0.65 {
                    Kind::Husk
                } else {
                    Kind::Gnawer
                };
                if let Some(spot) = free_spot(&map, &mut rng)
                    && monsters.iter().all(|monster| monster.at != spot)
                {
                    monsters.push(Monster::new(kind, spot));
                }
            }
        }
        if rng.chance(0.55)
            && let Some(spot) = free_spot(&map, &mut rng)
        {
            let amount = rng.random_int(3, 8 + depth as i64 * 3) as i32;
            items.push(Item {
                loot: Loot::Gold(amount),
                at: spot,
            });
        }
        if rng.chance(0.22)
            && let Some(spot) = free_spot(&map, &mut rng)
        {
            items.push(Item {
                loot: Loot::Tonic,
                at: spot,
            });
        }
    }

    Floor {
        map,
        start,
        braziers,
        monsters,
        items,
    }
}

// ---------------------------------------------------------------------------
// The game.

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Playing,
    Fallen,
}

/// A number or word that floats up from where something happened.
struct Popup {
    at: Vec2,
    text: String,
    color: Color,
    age: f32,
}

struct Message {
    text: String,
    age: f32,
}

struct Undercroft {
    sheet: Image,
    tiles: SpriteBatch,
    light_map: Canvas,
    glow: Shader,
    font: Font,
    big_font: Font,

    run_seed: u64,
    depth: i32,
    turn: u32,
    /// Rolls for fights and wandering, kept in the save so a loaded game
    /// carries on the same way.
    rng: RandomGenerator,
    map: Map,
    braziers: Vec<Point>,
    /// Shadow boxes around each brazier, worked out once per floor.
    brazier_shadows: Vec<Vec<[Vec2; 4]>>,
    /// Shadow boxes around you, worked out again after each step.
    torch_shadows: Vec<[Vec2; 4]>,
    /// Tiles a brazier shines on, which can be seen from further away.
    lit_by_brazier: Vec<bool>,
    hero: Hero,
    monsters: Vec<Monster>,
    items: Vec<Item>,
    explored: Vec<bool>,
    visible: Vec<bool>,

    state: State,
    camera: Vec2,
    step_wait: f32,
    travel: Vec<Point>,
    show_plans: bool,
    popups: Vec<Popup>,
    messages: Vec<Message>,
}

impl Undercroft {
    fn new_run(&mut self, run_seed: u64) {
        self.run_seed = run_seed;
        self.rng = RandomGenerator::new(run_seed.rotate_left(17) ^ 0x5EED);
        self.depth = 1;
        self.turn = 0;
        self.state = State::Playing;
        self.enter_floor(None);
        self.messages.clear();
        self.say("You climb down into the undercroft.");
    }

    /// Builds the floor for the current depth. A fresh hero is made for a
    /// new run; otherwise `hero` carries over, moved to the start.
    fn enter_floor(&mut self, hero: Option<Hero>) {
        let floor = generate(self.run_seed, self.depth);
        self.map = floor.map;
        self.braziers = floor.braziers;
        self.monsters = floor.monsters;
        self.items = floor.items;
        self.hero = match hero {
            Some(mut hero) => {
                hero.at = floor.start;
                hero.shown = floor.start.corner();
                hero
            }
            None => Hero::new(floor.start),
        };
        self.explored = vec![false; self.map.tiles.len()];
        self.travel.clear();
        self.popups.clear();
        self.prepare_floor();
        self.camera = self.camera_target();
    }

    /// Works out what depends only on the map: the tile batch, the
    /// braziers' shadows and what they light.
    fn prepare_floor(&mut self) {
        self.tiles.clear();
        for (index, &tile) in self.map.tiles.iter().enumerate() {
            let point = Map::point(index);
            let picture = match tile {
                Tile::Wall if self.map.get(point.offset(0, 1)) != Tile::Wall => art::WALL_FACE,
                Tile::Wall => art::WALL_TOP,
                Tile::Floor(variant) => art::FLOOR[variant as usize],
                Tile::Bones => art::BONES,
                Tile::Stairs => art::STAIRS,
                Tile::Brazier => art::BRAZIER,
            };
            // A slight change of shade from tile to tile keeps the floor
            // from looking stamped out.
            let shade = 0.9 + math::noise2(point.x as f32 * 0.37, point.y as f32 * 0.37) * 0.2;
            self.tiles.set_color(Color::rgb(shade, shade, shade));
            self.tiles.add_quad(
                &sprite_quad(picture),
                DrawParams::at(point.corner()).scale(SCALE),
            );
        }

        self.brazier_shadows = self
            .braziers
            .iter()
            .map(|brazier| self.map.shadow_boxes_near(brazier.center(), BRAZIER_REACH))
            .collect();
        self.lit_by_brazier = vec![false; self.map.tiles.len()];
        let reach = (BRAZIER_REACH / TILE) as i32 - 1;
        for &brazier in &self.braziers {
            let lit = &mut self.lit_by_brazier;
            self.map
                .seen_from(brazier, reach, |index| lit[index] = true);
        }
        self.look_around();
    }

    /// Works out what you can see from where you stand, and remembers it.
    fn look_around(&mut self) {
        self.visible = vec![false; self.map.tiles.len()];
        let visible = &mut self.visible;
        self.map
            .seen_from(self.hero.at, SIGHT, |index| visible[index] = true);
        // Further away, only what a brazier lights.
        let lit = &self.lit_by_brazier;
        self.map.seen_from(self.hero.at, FAR_SIGHT, |index| {
            if lit[index] {
                visible[index] = true;
            }
        });
        for (seen, &visible) in self.explored.iter_mut().zip(&self.visible) {
            *seen |= visible;
        }
        self.torch_shadows = self
            .map
            .shadow_boxes_near(self.hero.at.center(), TORCH_REACH);
    }

    fn is_visible(&self, point: Point) -> bool {
        Map::index(point).is_some_and(|index| self.visible[index])
    }

    fn is_explored(&self, point: Point) -> bool {
        Map::index(point).is_some_and(|index| self.explored[index])
    }

    fn monster_at(&self, point: Point) -> Option<usize> {
        self.monsters.iter().position(|monster| monster.at == point)
    }

    fn say(&mut self, text: impl Into<String>) {
        self.messages.push(Message {
            text: text.into(),
            age: 0.0,
        });
        if self.messages.len() > 6 {
            self.messages.remove(0);
        }
    }

    fn popup(&mut self, at: Point, text: impl Into<String>, color: Color) {
        self.popups.push(Popup {
            at: at.center(),
            text: text.into(),
            color,
            age: 0.0,
        });
    }

    // -- Turns ------------------------------------------------------------

    /// Steps one tile, or attacks whatever is there. Returns whether a
    /// turn was spent.
    fn step(&mut self, dx: i32, dy: i32) -> bool {
        if self.state != State::Playing {
            return false;
        }
        let target = self.hero.at.offset(dx, dy);
        if let Some(index) = self.monster_at(target) {
            self.hero_attacks(index);
            self.end_turn();
            return true;
        }
        if self.map.get(target).blocks_walking() {
            return false;
        }
        self.hero.at = target;
        self.pick_up();
        if self.map.get(target) == Tile::Stairs {
            self.say("Stairs lead down. Press Return to descend.");
        }
        self.end_turn();
        true
    }

    fn wait(&mut self) {
        if self.state == State::Playing {
            self.end_turn();
        }
    }

    fn hero_attacks(&mut self, index: usize) {
        let damage = self.rng.random_int(1, self.hero.attack as i64) as i32;
        let monster = &mut self.monsters[index];
        let toward = monster.at.center() - self.hero.at.center();
        self.hero.lunge = toward * 0.3;
        monster.health -= damage;
        monster.flash = 1.0;
        monster.hunting = true;
        let (at, name, dead) = (monster.at, monster.kind.stats().name, monster.health <= 0);
        self.popup(at, format!("{damage}"), Color::rgb(1.0, 0.95, 0.6));
        if dead {
            let worth = self.monsters.remove(index).kind.stats().worth;
            self.say(format!("You slay the {name}."));
            self.gain_experience(worth);
        }
    }

    fn gain_experience(&mut self, amount: i32) {
        self.hero.experience += amount;
        while self.hero.experience >= self.hero.experience_to_level() {
            self.hero.experience -= self.hero.experience_to_level();
            self.hero.level += 1;
            self.hero.max_health += 4;
            self.hero.health = (self.hero.health + 6).min(self.hero.max_health);
            self.hero.attack += 1;
            let level = self.hero.level;
            self.say(format!("You feel stronger. You are now level {level}."));
            self.popup(self.hero.at, "level up", Color::rgb(0.6, 0.9, 1.0));
        }
    }

    fn pick_up(&mut self) {
        while let Some(index) = self.items.iter().position(|item| item.at == self.hero.at) {
            let item = self.items.remove(index);
            match item.loot {
                Loot::Gold(amount) => {
                    self.hero.gold += amount;
                    self.popup(
                        item.at,
                        format!("+{amount} gold"),
                        Color::rgb(1.0, 0.85, 0.3),
                    );
                }
                Loot::Tonic => {
                    self.hero.tonics += 1;
                    self.say("You pick up a tonic. Press Q to drink it.");
                }
            }
        }
    }

    fn drink(&mut self) {
        if self.state != State::Playing {
            return;
        }
        if self.hero.tonics == 0 {
            self.say("You have no tonics.");
            return;
        }
        if self.hero.health == self.hero.max_health {
            self.say("You are already unhurt.");
            return;
        }
        self.hero.tonics -= 1;
        let healed = (self.hero.max_health / 2).min(self.hero.max_health - self.hero.health);
        self.hero.health += healed;
        self.popup(
            self.hero.at,
            format!("+{healed}"),
            Color::rgb(0.4, 1.0, 0.5),
        );
        self.say("The tonic warms you.");
        self.end_turn();
    }

    fn descend(&mut self) {
        if self.state != State::Playing {
            return;
        }
        if self.map.get(self.hero.at) != Tile::Stairs {
            self.say("There are no stairs here.");
            return;
        }
        self.depth += 1;
        let hero = std::mem::replace(&mut self.hero, Hero::new(Point::new(0, 0)));
        self.enter_floor(Some(hero));
        let depth = self.depth;
        self.say(format!("You descend to depth {depth}."));
        self.save();
    }

    /// Lets every monster act, then looks again.
    fn end_turn(&mut self) {
        self.turn += 1;
        for index in 0..self.monsters.len() {
            if self.state != State::Playing {
                break;
            }
            self.monster_turn(index);
        }
        self.look_around();
        // Walking to a clicked tile stops when something comes for you.
        if self
            .monsters
            .iter()
            .any(|monster| monster.hunting && self.is_visible(monster.at))
        {
            self.travel.clear();
        }
    }

    fn monster_turn(&mut self, index: usize) {
        let stats = self.monsters[index].kind.stats();
        let here = self.monsters[index].at;
        let hero = self.hero.at;

        // It notices you when it can see you and you are close enough.
        let near = here.steps_to(hero) <= stats.senses;
        if near && self.map.line_of_sight(here, hero) {
            if !self.monsters[index].hunting && self.is_visible(here) {
                self.popups.push(Popup {
                    at: here.center(),
                    text: "!".into(),
                    color: Color::rgb(1.0, 0.4, 0.3),
                    age: 0.0,
                });
            }
            let monster = &mut self.monsters[index];
            monster.hunting = true;
            monster.last_seen = Some(hero);
        }

        if stats.pace > 1 && !(self.turn + index as u32).is_multiple_of(stats.pace) {
            return;
        }

        if here.steps_to(hero) == 1 && self.monsters[index].hunting {
            self.monster_attacks(index);
            return;
        }

        let goal = if self.monsters[index].hunting {
            self.monsters[index].last_seen
        } else {
            None
        };
        let next = match goal {
            Some(goal) => {
                // Other monsters are in the way, except right next to you,
                // so a crowd still spreads out around a corridor.
                let path = self.map.find_path(here, goal, |point| {
                    self.monster_at(point).is_none() || point.steps_to(here) > 3
                });
                match path {
                    Some(path) => {
                        let next = path.first().copied();
                        self.monsters[index].plan = path;
                        next
                    }
                    None => None,
                }
            }
            None => {
                // Asleep, or lost track of you: shuffle about now and then.
                self.monsters[index].plan.clear();
                let roll = self.rng.random_int(0, 7);
                let (dx, dy) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .get(roll as usize)
                    .copied()
                    .unwrap_or((0, 0));
                Some(here.offset(dx, dy))
            }
        };

        if let Some(next) = next
            && next != hero
            && !self.map.get(next).blocks_walking()
            && self.monster_at(next).is_none()
        {
            self.monsters[index].at = next;
        }
        let monster = &mut self.monsters[index];
        if monster.last_seen == Some(monster.at) {
            // It got to where it last saw you, and you are gone.
            monster.last_seen = None;
            monster.hunting = false;
        }
    }

    fn monster_attacks(&mut self, index: usize) {
        let stats = self.monsters[index].kind.stats();
        let monster = &mut self.monsters[index];
        monster.lunge = (self.hero.at.center() - monster.at.center()) * 0.3;
        let damage = self.rng.random_int(1, stats.attack as i64) as i32;
        self.hero.health -= damage;
        self.hero.flash = 1.0;
        self.popup(
            self.hero.at,
            format!("-{damage}"),
            Color::rgb(1.0, 0.35, 0.3),
        );
        if self.hero.health <= 0 {
            self.hero.health = 0;
            self.state = State::Fallen;
            self.travel.clear();
            let name = stats.name;
            self.say(format!("The {name} brings you down."));
            // A fallen run is over, so there is nothing to continue.
            let _ = filesystem::remove(SAVE_FILE);
        }
    }

    // -- Saving -----------------------------------------------------------

    /// Writes the run as lines of words. The map itself is not saved: it
    /// is built again from the run's seed and the depth.
    fn save_text(&self) -> String {
        let hero = &self.hero;
        let mut text = format!(
            "{SAVE_HEADER}\nseed {}\ndepth {}\nturn {}\nrng {}\n",
            self.run_seed,
            self.depth,
            self.turn,
            self.rng.state()
        );
        text += &format!(
            "hero {} {} {} {} {} {} {} {} {}\n",
            hero.at.x,
            hero.at.y,
            hero.health,
            hero.max_health,
            hero.attack,
            hero.level,
            hero.experience,
            hero.gold,
            hero.tonics
        );
        let explored: String = self
            .explored
            .iter()
            .map(|&seen| if seen { '1' } else { '0' })
            .collect();
        text += &format!("explored {explored}\n");
        for monster in &self.monsters {
            text += &format!(
                "monster {} {} {} {} {}\n",
                monster.kind.save_name(),
                monster.at.x,
                monster.at.y,
                monster.health,
                u8::from(monster.hunting)
            );
        }
        for item in &self.items {
            let (name, amount) = match item.loot {
                Loot::Gold(amount) => ("gold", amount),
                Loot::Tonic => ("tonic", 0),
            };
            text += &format!("item {name} {} {} {amount}\n", item.at.x, item.at.y);
        }
        text
    }

    fn save(&mut self) {
        if self.state != State::Playing {
            return;
        }
        match filesystem::write(SAVE_FILE, self.save_text()) {
            Ok(()) => self.say("Saved."),
            Err(error) => self.say(format!("Could not save: {error}")),
        }
    }

    /// Loads the saved run, if there is one. Leaves the game as it was
    /// when the file is missing or cannot be read.
    fn load_save(&mut self) -> Result<(), String> {
        let text = filesystem::read_string(SAVE_FILE).map_err(|error| error.to_string())?;
        let saved = SavedRun::parse(&text).ok_or("the save file is damaged")?;

        self.run_seed = saved.seed;
        self.depth = saved.depth;
        self.turn = saved.turn;
        self.rng
            .set_state(&saved.rng)
            .map_err(|_| "the save file is damaged")?;
        self.state = State::Playing;
        self.enter_floor(None);
        let mut hero = saved.hero;
        hero.shown = hero.at.corner();
        self.hero = hero;
        self.monsters = saved.monsters;
        self.items = saved.items;
        if saved.explored.len() == self.explored.len() {
            self.explored = saved.explored;
        }
        self.look_around();
        self.camera = self.camera_target();
        Ok(())
    }

    // -- Drawing ----------------------------------------------------------

    fn camera_target(&self) -> Vec2 {
        let center = self.hero.shown + Vec2::new(TILE / 2.0, TILE / 2.0);
        let world = Vec2::new(MAP_WIDTH as f32 * TILE, MAP_HEIGHT as f32 * TILE);
        Vec2::new(
            (center.x - WIDTH / 2.0).clamp(0.0, world.x - WIDTH),
            (center.y - HEIGHT / 2.0).clamp(0.0, world.y - HEIGHT),
        )
    }

    fn with_camera(&self, draw: impl FnOnce()) {
        graphics::scoped(|| {
            graphics::translate((-self.camera.x.round(), -self.camera.y.round()));
            draw();
        });
    }

    /// The first and last tile columns and rows on screen.
    fn tiles_on_screen(&self) -> (i32, i32, i32, i32) {
        let left = (self.camera.x / TILE).floor() as i32;
        let top = (self.camera.y / TILE).floor() as i32;
        let right = ((self.camera.x + WIDTH) / TILE).ceil() as i32;
        let bottom = ((self.camera.y + HEIGHT) / TILE).ceil() as i32;
        (
            left.max(0),
            top.max(0),
            right.min(MAP_WIDTH - 1),
            bottom.min(MAP_HEIGHT - 1),
        )
    }

    /// Draws a creature or item. They are drawn over the light map, since
    /// they only show where you can see them, so they are shaded here by
    /// how far they are from your torch instead.
    fn draw_sprite(&self, picture: u32, at: Vec2, flash: f32) {
        let distance = (at - self.hero.shown).length();
        let shade = (1.1 - distance / (SIGHT as f32 * TILE * 1.4)).clamp(0.5, 1.0);
        graphics::set_color(Color::rgb(shade, shade * 0.95, shade * 0.88));
        graphics::draw(
            &self.sheet.region(&sprite_quad(picture)),
            DrawParams::at(at).scale(SCALE),
        );
        if flash > 0.0 {
            // Drawing the sprite again, added on top, lights it up.
            graphics::set_blend_mode(BlendMode::Add);
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, flash));
            graphics::draw(
                &self.sheet.region(&sprite_quad(picture)),
                DrawParams::at(at).scale(SCALE),
            );
            graphics::set_blend_mode(BlendMode::Alpha);
        }
        graphics::set_color(Color::WHITE);
    }

    /// The map, and the paths drawn on it, all of which the light map
    /// darkens.
    fn draw_map(&self) {
        graphics::set_color(Color::WHITE);
        graphics::draw(&self.tiles, (0.0, 0.0));

        if self.show_plans {
            graphics::set_color(Color::rgba(1.0, 0.35, 0.3, 0.8));
            for monster in &self.monsters {
                if monster.hunting {
                    for step in &monster.plan {
                        graphics::circle(DrawMode::Fill, step.center(), 3.5);
                    }
                }
            }
        }
        graphics::set_color(Color::rgba(0.5, 0.8, 1.0, 0.7));
        for step in &self.travel {
            graphics::circle(DrawMode::Fill, step.center(), 3.0);
        }
        graphics::set_color(Color::WHITE);
    }

    /// Whatever you can see in the map.
    fn draw_things(&self) {
        for item in &self.items {
            if self.is_visible(item.at) {
                let picture = match item.loot {
                    Loot::Gold(_) => art::GOLD,
                    Loot::Tonic => art::TONIC,
                };
                let bob = (timer::time() as f32 * 3.0 + item.at.x as f32).sin() * 1.5;
                self.draw_sprite(picture, item.at.corner() + Vec2::new(0.0, bob), 0.0);
            }
        }

        for monster in &self.monsters {
            if self.is_visible(monster.at) {
                let picture = monster.kind.stats().picture;
                self.draw_sprite(picture, monster.shown + monster.lunge, monster.flash);
                self.draw_health_bar(monster);
            }
        }

        if self.state == State::Fallen {
            self.draw_sprite(art::BONES, self.hero.shown, 0.0);
        } else {
            self.draw_sprite(
                art::HERO,
                self.hero.shown + self.hero.lunge,
                self.hero.flash,
            );
        }
    }

    fn draw_health_bar(&self, monster: &Monster) {
        let max = monster.kind.stats().health;
        if monster.health >= max {
            return;
        }
        let at = monster.shown + Vec2::new(4.0, -4.0);
        let width = TILE - 8.0;
        graphics::set_color(Color::rgb(0.2, 0.05, 0.05));
        graphics::rectangle(DrawMode::Fill, at, (width, 3.0));
        graphics::set_color(Color::rgb(0.9, 0.25, 0.2));
        graphics::rectangle(
            DrawMode::Fill,
            at,
            (width * monster.health as f32 / max as f32, 3.0),
        );
        graphics::set_color(Color::WHITE);
    }

    /// Fills the tiles on screen that `pick` chooses, joining neighbors in
    /// a row into one rectangle, so a few rectangles cover what would be
    /// many squares.
    fn fill_tiles(&self, pick: impl Fn(usize) -> bool) {
        let (left, top, right, bottom) = self.tiles_on_screen();
        for y in top..=bottom {
            let mut start = None;
            for x in left..=right + 1 {
                let chosen = x <= right && pick((y * MAP_WIDTH + x) as usize);
                match (chosen, start) {
                    (true, None) => start = Some(x),
                    (false, Some(first)) => {
                        graphics::rectangle(
                            DrawMode::Fill,
                            Point::new(first, y).corner(),
                            ((x - first) as f32 * TILE, TILE),
                        );
                        start = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Fills the light map. It starts black. Places you remember get a dim
    /// blue, then the torch and braziers add their glow, kept out of the
    /// shadows the walls cast. Last, places never seen go black again, so
    /// a brazier does not give away a room before you find it.
    fn draw_light_map(&self) {
        self.light_map.render_to(|| {
            graphics::clear(Color::BLACK);
            self.with_camera(|| {
                graphics::set_color(Color::rgb(0.13, 0.14, 0.22));
                self.fill_tiles(|index| self.explored[index] && !self.visible[index]);

                let time = timer::time() as f32;
                let flicker = math::noise(time * 5.0);
                let is_wall = |index: usize| self.map.tiles[index] == Tile::Wall;
                let torch = (
                    self.hero.shown + Vec2::new(TILE / 2.0, TILE / 2.0),
                    TORCH_REACH + flicker * 18.0,
                    Color::rgb(1.0, 0.8 + flicker * 0.08, 0.55),
                    self.torch_shadows.as_slice(),
                    &self.visible,
                );
                let braziers = self
                    .braziers
                    .iter()
                    .zip(&self.brazier_shadows)
                    .enumerate()
                    .map(|(index, (brazier, shadows))| {
                        let flicker = math::noise(time * 7.0 + index as f32 * 13.0);
                        (
                            brazier.center(),
                            BRAZIER_REACH + flicker * 20.0,
                            Color::rgb(1.0, 0.55 + flicker * 0.1, 0.25),
                            shadows.as_slice(),
                            &self.lit_by_brazier,
                        )
                    });

                graphics::set_shader(Some(&self.glow));
                for (center, reach, color, shadows, lit) in std::iter::once(torch).chain(braziers) {
                    graphics::stencil(|| {
                        for corners in shadows {
                            graphics::shadow(center, corners, SHADOW_LENGTH);
                        }
                    });
                    // A wall's shadow falls on the walls behind it too, so
                    // a long wall seen from the side would be cut into
                    // dark wedges. The walls the light reaches by line of
                    // sight are taken back out of the shadow.
                    graphics::stencil_with(
                        StencilWrite {
                            value: 0,
                            keep_values: true,
                            ..StencilWrite::default()
                        },
                        || self.fill_tiles(|index| lit[index] && is_wall(index)),
                    );
                    graphics::set_stencil_test(CompareMode::Equal, 0);
                    graphics::set_blend_mode(BlendMode::Add);
                    graphics::set_color(color);
                    graphics::rectangle(
                        DrawMode::Fill,
                        center - Vec2::new(reach, reach),
                        (reach * 2.0, reach * 2.0),
                    );
                    graphics::set_blend_mode(BlendMode::Alpha);
                    graphics::clear_stencil_test();
                }
                graphics::set_shader(None);

                graphics::set_color(Color::BLACK);
                self.fill_tiles(|index| !self.explored[index]);
            });
        });
    }

    fn draw_hud(&self) {
        graphics::set_font(&self.font);
        let hero = &self.hero;

        // A dark panel along the top.
        graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.6));
        graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, 34.0));
        let bar = 160.0;
        graphics::set_color(Color::rgb(0.25, 0.06, 0.06));
        graphics::rectangle(DrawMode::Fill, (12.0, 10.0), (bar, 14.0));
        graphics::set_color(Color::rgb(0.85, 0.2, 0.2));
        graphics::rectangle(
            DrawMode::Fill,
            (12.0, 10.0),
            (bar * hero.health as f32 / hero.max_health as f32, 14.0),
        );
        graphics::set_color(Color::WHITE);
        graphics::printf(
            &format!("{} / {}", hero.health, hero.max_health),
            (12.0, 8.0),
            bar,
            Align::Center,
        );
        graphics::print(
            &format!(
                "Depth {}    Level {} ({}/{})    Gold {}    Tonics {}",
                self.depth,
                hero.level,
                hero.experience,
                hero.experience_to_level(),
                hero.gold,
                hero.tonics
            ),
            (188.0, 8.0),
        );

        // What happened lately, fading out as it gets older.
        let count = self.messages.len();
        for (row, message) in self.messages.iter().enumerate() {
            let fade = (1.0 - (message.age - 5.0) / 2.0).clamp(0.0, 1.0);
            let newest = if row + 1 == count { 1.0 } else { 0.75 };
            graphics::set_color(Color::rgba(1.0, 0.95, 0.85, fade * newest));
            let y = HEIGHT - 28.0 - (count - 1 - row) as f32 * 20.0;
            graphics::print(&message.text, (12.0, y));
        }

        self.draw_minimap();

        if self.state == State::Fallen {
            graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.55));
            graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, HEIGHT));
            graphics::set_font(&self.big_font);
            graphics::set_color(Color::rgb(0.9, 0.3, 0.25));
            graphics::printf("You have fallen", (0.0, 230.0), WIDTH, Align::Center);
            graphics::set_font(&self.font);
            graphics::set_color(Color::WHITE);
            graphics::printf(
                &format!(
                    "Depth {} with {} gold, at level {}. This run's seed was {}.\n\nPress R to start a new run.",
                    self.depth, hero.gold, hero.level, self.run_seed
                ),
                (0.0, 300.0),
                WIDTH,
                Align::Center,
            );
        } else {
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.45));
            graphics::printf(
                "Space wait  Q tonic  Return stairs  P paths  F5 save  F9 load",
                (0.0, HEIGHT - 28.0),
                WIDTH - 12.0,
                Align::Right,
            );
        }
    }

    /// A small map of what you have explored, in the top right corner.
    fn draw_minimap(&self) {
        let cell = 3.0;
        let origin = Vec2::new(WIDTH - MAP_WIDTH as f32 * cell - 12.0, 44.0);
        graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.5));
        graphics::rectangle(
            DrawMode::Fill,
            origin - Vec2::new(4.0, 4.0),
            (
                MAP_WIDTH as f32 * cell + 8.0,
                MAP_HEIGHT as f32 * cell + 8.0,
            ),
        );
        for (index, &tile) in self.map.tiles.iter().enumerate() {
            if !self.explored[index] || tile == Tile::Wall {
                continue;
            }
            let point = Map::point(index);
            let color = match tile {
                Tile::Stairs => Color::rgb(0.4, 0.8, 1.0),
                Tile::Brazier => Color::rgb(1.0, 0.6, 0.2),
                _ if self.visible[index] => Color::rgba(0.75, 0.7, 0.6, 0.9),
                _ => Color::rgba(0.45, 0.45, 0.55, 0.8),
            };
            graphics::set_color(color);
            graphics::rectangle(
                DrawMode::Fill,
                origin + Vec2::new(point.x as f32 * cell, point.y as f32 * cell),
                (cell, cell),
            );
        }
        graphics::set_color(Color::rgb(0.4, 0.6, 1.0));
        graphics::rectangle(
            DrawMode::Fill,
            origin
                + Vec2::new(
                    self.hero.at.x as f32 * cell - 1.0,
                    self.hero.at.y as f32 * cell - 1.0,
                ),
            (cell + 2.0, cell + 2.0),
        );
    }
}

/// A run read back from the save file.
struct SavedRun {
    seed: u64,
    depth: i32,
    turn: u32,
    rng: String,
    hero: Hero,
    explored: Vec<bool>,
    monsters: Vec<Monster>,
    items: Vec<Item>,
}

impl SavedRun {
    fn parse(text: &str) -> Option<SavedRun> {
        let mut lines = text.lines();
        if lines.next()? != SAVE_HEADER {
            return None;
        }
        let mut run = SavedRun {
            seed: 0,
            depth: 1,
            turn: 0,
            rng: String::new(),
            hero: Hero::new(Point::new(0, 0)),
            explored: Vec::new(),
            monsters: Vec::new(),
            items: Vec::new(),
        };
        for line in lines {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            let numbers = || {
                rest.split_whitespace()
                    .map(|word| word.parse::<i32>().ok())
                    .collect::<Option<Vec<i32>>>()
            };
            match key {
                "seed" => run.seed = rest.parse().ok()?,
                "depth" => run.depth = rest.parse().ok()?,
                "turn" => run.turn = rest.parse().ok()?,
                "rng" => run.rng = rest.to_string(),
                "hero" => {
                    let n = numbers()?;
                    let [
                        x,
                        y,
                        health,
                        max_health,
                        attack,
                        level,
                        experience,
                        gold,
                        tonics,
                    ] = n.as_slice().try_into().ok()?;
                    run.hero = Hero {
                        health,
                        max_health,
                        attack,
                        level,
                        experience,
                        gold,
                        tonics,
                        ..Hero::new(Point::new(x, y))
                    };
                }
                "explored" => run.explored = rest.chars().map(|c| c == '1').collect(),
                "monster" => {
                    let (name, rest) = rest.split_once(' ')?;
                    let n: Vec<i32> = rest
                        .split_whitespace()
                        .map(|word| word.parse().ok())
                        .collect::<Option<_>>()?;
                    let [x, y, health, hunting] = n.as_slice().try_into().ok()?;
                    let mut monster = Monster::new(Kind::from_save_name(name)?, Point::new(x, y));
                    monster.health = health;
                    monster.hunting = hunting != 0;
                    // A hunter picks up the trail from where you stand.
                    monster.last_seen = monster.hunting.then_some(run.hero.at);
                    run.monsters.push(monster);
                }
                "item" => {
                    let (name, rest) = rest.split_once(' ')?;
                    let n: Vec<i32> = rest
                        .split_whitespace()
                        .map(|word| word.parse().ok())
                        .collect::<Option<_>>()?;
                    let [x, y, amount] = n.as_slice().try_into().ok()?;
                    let loot = match name {
                        "gold" => Loot::Gold(amount),
                        "tonic" => Loot::Tonic,
                        _ => return None,
                    };
                    run.items.push(Item {
                        loot,
                        at: Point::new(x, y),
                    });
                }
                _ => {}
            }
        }
        Some(run)
    }
}

/// Moves `value` toward `target`, covering most of the way in a fraction
/// of a second however fast the frames come.
fn approach(value: Vec2, target: Vec2, rate: f32, dt: f32) -> Vec2 {
    value + (target - value) * (1.0 - (-rate * dt).exp())
}

/// A seed for a new run. Mixing in the time gives every run its own floors.
fn fresh_seed() -> u64 {
    math::random_seed() ^ (timer::time() * 1e9) as u64
}

/// The step a movement key asks for.
fn key_step(key: Key) -> Option<(i32, i32)> {
    match key {
        Key::W | Key::Up | Key::Kp8 => Some((0, -1)),
        Key::S | Key::Down | Key::Kp2 => Some((0, 1)),
        Key::A | Key::Left | Key::Kp4 => Some((-1, 0)),
        Key::D | Key::Right | Key::Kp6 => Some((1, 0)),
        _ => None,
    }
}

const STEP_KEYS: [Key; 12] = [
    Key::W,
    Key::Up,
    Key::Kp8,
    Key::S,
    Key::Down,
    Key::Kp2,
    Key::A,
    Key::Left,
    Key::Kp4,
    Key::D,
    Key::Right,
    Key::Kp6,
];

impl Game for Undercroft {
    fn load() -> smitten::Result<Self> {
        graphics::set_default_filter(Filter::Nearest);
        graphics::set_background_color(Color::BLACK);
        let sheet = Image::from_image_data(&paint_sheet())?;
        let glow = Shader::new(GLOW)?;
        glow.send("falloff", 0.9_f32);

        let mut game = Undercroft {
            tiles: SpriteBatch::new(&sheet, (MAP_WIDTH * MAP_HEIGHT) as usize),
            sheet,
            light_map: Canvas::new(WIDTH as u32, HEIGHT as u32)?,
            glow,
            font: Font::default_at(15.0),
            big_font: Font::default_at(40.0),
            run_seed: 0,
            depth: 1,
            turn: 0,
            rng: RandomGenerator::new(0),
            map: Map { tiles: Vec::new() },
            braziers: Vec::new(),
            brazier_shadows: Vec::new(),
            torch_shadows: Vec::new(),
            lit_by_brazier: Vec::new(),
            hero: Hero::new(Point::new(0, 0)),
            monsters: Vec::new(),
            items: Vec::new(),
            explored: Vec::new(),
            visible: Vec::new(),
            state: State::Playing,
            camera: Vec2::ZERO,
            step_wait: 0.0,
            travel: Vec::new(),
            show_plans: false,
            popups: Vec::new(),
            messages: Vec::new(),
        };

        // `--seed 1234` starts a new run with that seed, to play the same
        // floors again or share them.
        let mut args = std::env::args().skip_while(|arg| arg != "--seed").skip(1);
        let chosen_seed = args.next().and_then(|seed| seed.parse().ok());
        match chosen_seed {
            Some(seed) => game.new_run(seed),
            None => match game.load_save() {
                Ok(()) => {
                    let depth = game.depth;
                    game.say(format!("Welcome back. You are on depth {depth}."));
                }
                Err(_) => game.new_run(fresh_seed()),
            },
        }
        Ok(game)
    }

    fn update(&mut self, dt: f32) {
        // Holding a direction keeps walking, after a short pause.
        self.step_wait -= dt;
        if self.step_wait <= 0.0 && self.state == State::Playing {
            let held = STEP_KEYS
                .iter()
                .find(|&&key| keyboard::is_down(key))
                .and_then(|&key| key_step(key));
            if let Some((dx, dy)) = held {
                self.travel.clear();
                self.step(dx, dy);
                self.step_wait = STEP_REPEAT;
            } else if !self.travel.is_empty() {
                let next = self.travel.remove(0);
                let (dx, dy) = (next.x - self.hero.at.x, next.y - self.hero.at.y);
                if dx.abs() + dy.abs() != 1 || !self.step(dx, dy) {
                    self.travel.clear();
                }
                self.step_wait = TRAVEL_STEP;
            }
        }

        // Everything slides toward where it now stands.
        self.hero.shown = approach(self.hero.shown, self.hero.at.corner(), 20.0, dt);
        self.hero.lunge = approach(self.hero.lunge, Vec2::ZERO, 14.0, dt);
        self.hero.flash = (self.hero.flash - dt * 5.0).max(0.0);
        for monster in &mut self.monsters {
            monster.shown = approach(monster.shown, monster.at.corner(), 16.0, dt);
            monster.lunge = approach(monster.lunge, Vec2::ZERO, 14.0, dt);
            monster.flash = (monster.flash - dt * 5.0).max(0.0);
        }
        self.camera = approach(self.camera, self.camera_target(), 8.0, dt);

        for popup in &mut self.popups {
            popup.age += dt;
        }
        self.popups.retain(|popup| popup.age < 1.0);
        for message in &mut self.messages {
            message.age += dt;
        }
    }

    fn draw(&mut self) {
        self.with_camera(|| self.draw_map());

        // The map is darkened by the light map. White leaves it as it is,
        // black hides it.
        self.draw_light_map();
        graphics::set_color(Color::WHITE);
        graphics::set_blend_mode(BlendMode::Multiply);
        graphics::draw(&self.light_map, (0.0, 0.0));
        graphics::set_blend_mode(BlendMode::Alpha);

        self.with_camera(|| self.draw_things());

        // The popups float above the darkness.
        graphics::set_font(&self.font);
        self.with_camera(|| {
            for popup in &self.popups {
                let fade = 1.0 - popup.age;
                graphics::set_color(Color::rgba(0.0, 0.0, 0.0, fade));
                let at = popup.at - Vec2::new(50.0, 22.0 + popup.age * 26.0);
                graphics::printf(&popup.text, at + Vec2::new(1.0, 1.0), 100.0, Align::Center);
                let Color { r, g, b, .. } = popup.color;
                graphics::set_color(Color::rgba(r, g, b, fade));
                graphics::printf(&popup.text, at, 100.0, Align::Center);
            }
        });

        self.draw_hud();
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        if let Some((dx, dy)) = key_step(key) {
            self.travel.clear();
            self.step(dx, dy);
            self.step_wait = STEP_DELAY;
            return;
        }
        match key {
            Key::Space | Key::Period | Key::Kp5 => self.wait(),
            Key::Q => self.drink(),
            Key::Return | Key::KpEnter => self.descend(),
            Key::P => self.show_plans = !self.show_plans,
            Key::F5 => self.save(),
            Key::F9 => match self.load_save() {
                Ok(()) => self.say("Loaded."),
                Err(error) => self.say(format!("Could not load: {error}")),
            },
            Key::R if self.state == State::Fallen => self.new_run(fresh_seed()),
            Key::Escape => event::quit(),
            _ => {}
        }
    }

    fn mouse_pressed(&mut self, click: MouseClick) {
        if click.button != MouseButton::Left || self.state != State::Playing {
            return;
        }
        let world = click.position + self.camera;
        let target = Point::new(
            (world.x / TILE).floor() as i32,
            (world.y / TILE).floor() as i32,
        );
        if !self.is_explored(target) || self.map.get(target).blocks_walking() {
            return;
        }
        // Only through places you know, and around monsters you can see.
        let path = self.map.find_path(self.hero.at, target, |point| {
            self.is_explored(point) && !(self.is_visible(point) && self.monster_at(point).is_some())
        });
        self.travel = path.unwrap_or_default();
    }

    fn quit(&mut self) -> bool {
        self.save();
        false
    }
}

fn main() {
    smitten::run::<Undercroft>(Config {
        title: "Undercroft".into(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        identity: Some("smitten-undercroft".into()),
        ..Config::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stairs_of(map: &Map) -> Point {
        Map::point(
            map.tiles
                .iter()
                .position(|&tile| tile == Tile::Stairs)
                .expect("every floor has stairs"),
        )
    }

    #[test]
    fn every_floor_can_be_crossed() {
        for seed in 0..40 {
            for depth in 1..=6 {
                let floor = generate(seed, depth);
                let stairs = stairs_of(&floor.map);
                assert_ne!(stairs, floor.start, "seed {seed} depth {depth}");
                let path = floor.map.find_path(floor.start, stairs, |_| true);
                assert!(path.is_some(), "seed {seed} depth {depth}");
                // Every open tile can be reached from the start.
                let mut reached = vec![false; floor.map.tiles.len()];
                let mut queue = vec![floor.start];
                while let Some(at) = queue.pop() {
                    let Some(index) = Map::index(at) else {
                        continue;
                    };
                    if reached[index] || floor.map.get(at).blocks_walking() {
                        continue;
                    }
                    reached[index] = true;
                    queue.extend([
                        at.offset(1, 0),
                        at.offset(-1, 0),
                        at.offset(0, 1),
                        at.offset(0, -1),
                    ]);
                }
                for (index, tile) in floor.map.tiles.iter().enumerate() {
                    assert!(
                        tile.blocks_walking() || reached[index],
                        "seed {seed} depth {depth}: {:?} is cut off",
                        Map::point(index)
                    );
                }
                for monster in &floor.monsters {
                    assert!(!floor.map.get(monster.at).blocks_walking());
                    assert!(
                        floor
                            .map
                            .find_path(floor.start, monster.at, |_| true)
                            .is_some()
                    );
                }
            }
        }
    }

    #[test]
    fn floors_are_built_the_same_from_the_same_seed() {
        let (a, b) = (generate(7, 3), generate(7, 3));
        assert_eq!(a.map.tiles, b.map.tiles);
        assert_eq!(a.items.len(), b.items.len());
        assert_ne!(generate(7, 4).map.tiles, a.map.tiles);
    }

    #[test]
    fn a_star_finds_the_shortest_way_around_a_wall() {
        let mut map = Map {
            tiles: vec![Tile::Floor(0); (MAP_WIDTH * MAP_HEIGHT) as usize],
        };
        // A wall down column 5 from row 0 to row 9, open below.
        for y in 0..10 {
            map.set(Point::new(5, y), Tile::Wall);
        }
        let path = map
            .find_path(Point::new(3, 2), Point::new(7, 2), |_| true)
            .unwrap();
        assert_eq!(path.last(), Some(&Point::new(7, 2)));
        // Down 8 to row 10, across 4, up 8.
        assert_eq!(path.len(), 20);
        let mut at = Point::new(3, 2);
        for &step in &path {
            assert_eq!(at.steps_to(step), 1);
            assert!(!map.get(step).blocks_walking());
            at = step;
        }
        // Sealing the gap leaves no way through.
        for x in 0..MAP_WIDTH {
            map.set(Point::new(x, 10), Tile::Wall);
        }
        for y in 0..MAP_HEIGHT {
            map.set(Point::new(5, y), Tile::Wall);
        }
        assert!(
            map.find_path(Point::new(3, 2), Point::new(7, 2), |_| true)
                .is_none()
        );
    }

    #[test]
    fn a_save_reads_back() {
        let text = format!(
            "{SAVE_HEADER}\nseed 99\ndepth 3\nturn 41\nrng 0x1:2:3:4:5\n\
             hero 4 5 12 28 6 3 2 77 2\nexplored 0110\n\
             monster husk 9 9 3 1\nmonster gnawer 1 2 4 0\n\
             item gold 3 3 12\nitem tonic 6 7 0\n"
        );
        let run = SavedRun::parse(&text).unwrap();
        assert_eq!((run.seed, run.depth, run.turn), (99, 3, 41));
        assert_eq!(run.rng, "0x1:2:3:4:5");
        assert_eq!(run.hero.at, Point::new(4, 5));
        assert_eq!(
            (run.hero.health, run.hero.gold, run.hero.tonics),
            (12, 77, 2)
        );
        assert_eq!(run.explored, [false, true, true, false]);
        assert_eq!(run.monsters.len(), 2);
        assert_eq!(run.monsters[0].kind, Kind::Husk);
        assert!(run.monsters[0].hunting);
        assert_eq!(run.items[0].loot, Loot::Gold(12));
        assert_eq!(run.items[1].loot, Loot::Tonic);

        assert!(SavedRun::parse("something else").is_none());
        assert!(SavedRun::parse(&format!("{SAVE_HEADER}\nhero 1 2\n")).is_none());
    }
}

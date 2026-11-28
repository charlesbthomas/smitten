//! Collision tests that say how to push things apart, rays, a spatial
//! hash for finding what is near, and moving a box through a tile map.
//!
//! The overlap tests return the *push-out* vector: the shortest move that
//! takes the first shape out of the second. Add it to the first shape's
//! position to resolve the hit, or split it between the two. Shapes that
//! only touch do not overlap.
//!
//! ```
//! use smitten_kit_core::collision::aabb_overlap;
//! use smitten_kit_core::glam::Vec2;
//! use smitten_kit_core::math::Rect;
//!
//! let player = Rect::from_position_size((0.0, 0.0), (10.0, 10.0));
//! let wall = Rect::from_position_size((8.0, -20.0), (10.0, 50.0));
//! assert_eq!(aabb_overlap(&player, &wall), Some(Vec2::new(-2.0, 0.0)));
//! ```
//!
//! For a platformer on a tile map, [`move_and_slide`] moves a box by a
//! frame's motion, stopping at solid tiles on each axis, and says which
//! sides touched:
//!
//! ```
//! use smitten_kit_core::collision::move_and_slide;
//! use smitten_kit_core::glam::{IVec2, Vec2};
//! use smitten_kit_core::math::Rect;
//!
//! const TILE: f32 = 16.0;
//! let floor = |cell: IVec2| cell.y >= 4; // everything from y = 64 down
//!
//! let player = Rect::from_position_size((20.0, 40.0), (12.0, 20.0));
//! let slide = move_and_slide(player, Vec2::new(3.0, 10.0), TILE, floor);
//! assert_eq!(slide.bounds.max.y, 64.0); // landed on the floor
//! assert_eq!(slide.bounds.min.x, 23.0); // still moved sideways
//! assert!(slide.contacts.bottom);
//! ```

use std::collections::HashMap;

use glam::{IVec2, Vec2};

use crate::math::{Circle, Rect};

/// The shortest move that takes rectangle `a` out of rectangle `b`, or
/// `None` when they do not overlap.
pub fn aabb_overlap(a: &Rect, b: &Rect) -> Option<Vec2> {
    let x = (a.max.x - b.min.x).min(b.max.x - a.min.x);
    let y = (a.max.y - b.min.y).min(b.max.y - a.min.y);
    if x <= 0.0 || y <= 0.0 {
        return None;
    }
    let away = a.center() - b.center();
    Some(if x < y {
        Vec2::new(if away.x < 0.0 { -x } else { x }, 0.0)
    } else {
        Vec2::new(0.0, if away.y < 0.0 { -y } else { y })
    })
}

/// The shortest move that takes circle `a` out of circle `b`, or `None`
/// when they do not overlap. Circles with the same center are pushed
/// along +x.
pub fn circle_overlap(a: &Circle, b: &Circle) -> Option<Vec2> {
    let between = a.center - b.center;
    let reach = a.radius + b.radius;
    let distance_squared = between.length_squared();
    if distance_squared >= reach * reach {
        return None;
    }
    let distance = distance_squared.sqrt();
    let direction = if distance > 0.0 {
        between / distance
    } else {
        Vec2::X
    };
    Some(direction * (reach - distance))
}

/// The shortest move that takes circle `circle` out of rectangle `rect`,
/// or `None` when they do not overlap.
pub fn circle_rect_overlap(circle: &Circle, rect: &Rect) -> Option<Vec2> {
    let nearest = rect.clamp(circle.center);
    let between = circle.center - nearest;
    let distance_squared = between.length_squared();
    if distance_squared > 0.0 {
        if distance_squared >= circle.radius * circle.radius {
            return None;
        }
        let distance = distance_squared.sqrt();
        return Some(between / distance * (circle.radius - distance));
    }
    // The center is inside: leave by the nearest side.
    let to_left = circle.center.x - rect.min.x;
    let to_right = rect.max.x - circle.center.x;
    let to_top = circle.center.y - rect.min.y;
    let to_bottom = rect.max.y - circle.center.y;
    let nearest_side = to_left.min(to_right).min(to_top).min(to_bottom);
    let r = circle.radius;
    Some(if nearest_side == to_left {
        Vec2::new(-(to_left + r), 0.0)
    } else if nearest_side == to_right {
        Vec2::new(to_right + r, 0.0)
    } else if nearest_side == to_top {
        Vec2::new(0.0, -(to_top + r))
    } else {
        Vec2::new(0.0, to_bottom + r)
    })
}

fn project(points: &[Vec2], axis: Vec2) -> (f32, f32) {
    points
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
            let d = p.dot(axis);
            (lo.min(d), hi.max(d))
        })
}

fn centroid(points: &[Vec2]) -> Vec2 {
    points.iter().copied().sum::<Vec2>() / points.len() as f32
}

/// The shortest move that takes convex polygon `a` out of convex polygon
/// `b` (separating axis test), or `None` when they do not overlap.
///
/// The points go around each polygon in either direction. The test can
/// report overlaps that are not there for a polygon that is not convex,
/// so split such a shape into convex parts.
pub fn polygon_overlap(a: &[Vec2], b: &[Vec2]) -> Option<Vec2> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let mut best = (f32::INFINITY, Vec2::ZERO);
    for points in [a, b] {
        for (i, &start) in points.iter().enumerate() {
            let end = points[(i + 1) % points.len()];
            let axis = (end - start).perp().normalize_or_zero();
            if axis == Vec2::ZERO {
                continue;
            }
            let (a_lo, a_hi) = project(a, axis);
            let (b_lo, b_hi) = project(b, axis);
            let overlap = (a_hi - b_lo).min(b_hi - a_lo);
            if overlap <= 0.0 {
                return None;
            }
            if overlap < best.0 {
                best = (overlap, axis);
            }
        }
    }
    let (depth, mut axis) = best;
    if (centroid(a) - centroid(b)).dot(axis) < 0.0 {
        axis = -axis;
    }
    Some(axis * depth)
}

/// Where a ray met a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// How far along the ray, in lengths of its direction: the hit is at
    /// `origin + direction * t`.
    pub t: f32,
    /// The point hit.
    pub point: Vec2,
    /// The outward normal of the surface hit, or zero for a ray that
    /// starts inside.
    pub normal: Vec2,
}

/// Where the ray from `origin` along `direction` first meets `rect`.
/// `None` if it never does (going forward). A ray starting inside hits
/// at `t` 0 with no normal.
///
/// `direction` need not be a unit vector: with a frame's motion as the
/// direction, a hit with `t <= 1.0` happens this frame.
pub fn ray_rect(origin: Vec2, direction: Vec2, rect: &Rect) -> Option<RayHit> {
    if rect.contains(origin) {
        return Some(RayHit {
            t: 0.0,
            point: origin,
            normal: Vec2::ZERO,
        });
    }
    let mut enter = f32::NEG_INFINITY;
    let mut leave = f32::INFINITY;
    let mut normal = Vec2::ZERO;
    for axis in 0..2 {
        let (o, d, lo, hi) = (
            origin[axis],
            direction[axis],
            rect.min[axis],
            rect.max[axis],
        );
        if d == 0.0 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((lo - o) / d, (hi - o) / d);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        let side = if d > 0.0 { -1.0 } else { 1.0 };
        if near > enter {
            enter = near;
            normal = Vec2::ZERO;
            normal[axis] = side;
        }
        leave = leave.min(far);
    }
    if enter > leave || enter < 0.0 {
        return None;
    }
    Some(RayHit {
        t: enter,
        point: origin + direction * enter,
        normal,
    })
}

/// Where the ray from `origin` along `direction` first meets `circle`.
/// `None` if it never does (going forward). A ray starting inside hits
/// at `t` 0 with no normal.
pub fn ray_circle(origin: Vec2, direction: Vec2, circle: &Circle) -> Option<RayHit> {
    if circle.contains(origin) {
        return Some(RayHit {
            t: 0.0,
            point: origin,
            normal: Vec2::ZERO,
        });
    }
    let to_origin = origin - circle.center;
    let a = direction.length_squared();
    if a == 0.0 {
        return None;
    }
    let b = to_origin.dot(direction);
    let c = to_origin.length_squared() - circle.radius * circle.radius;
    let discriminant = b * b - a * c;
    if discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / a;
    if t < 0.0 {
        return None;
    }
    let point = origin + direction * t;
    Some(RayHit {
        t,
        point,
        normal: (point - circle.center).normalize_or_zero(),
    })
}

/// Which sides of a moving box ran into something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Contacts {
    /// Stopped moving left.
    pub left: bool,
    /// Stopped moving right.
    pub right: bool,
    /// Stopped moving up (a ceiling, with y down).
    pub top: bool,
    /// Stopped moving down: standing on the ground.
    pub bottom: bool,
}

impl Contacts {
    /// Whether any side touched.
    pub fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

/// The result of [`move_and_slide`] or [`slide_against`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    /// The box where it ended up.
    pub bounds: Rect,
    /// How far it actually moved.
    pub moved: Vec2,
    /// What it ran into.
    pub contacts: Contacts,
}

/// Moves `bounds` by `motion` through a grid of `tile`-sized cells,
/// stopping flush against cells where `solid` is true. It moves along x
/// first, then y, so a box pressed into a wall still falls and one on
/// the floor still walks. Every cell crossed is checked, so a fast box
/// does not pass through thin walls.
///
/// A box that starts overlapping solid cells is not pushed out; it only
/// stops entering new ones.
pub fn move_and_slide(
    bounds: Rect,
    motion: Vec2,
    tile: f32,
    mut solid: impl FnMut(IVec2) -> bool,
) -> Slide {
    // Edges exactly on a cell line belong to the cell they face away
    // from, so a box standing on the floor is not inside it.
    let edge = tile * 1e-4;
    let mut rect = bounds;
    let mut contacts = Contacts::default();

    for axis in 0..2 {
        let d = motion[axis];
        if d == 0.0 {
            continue;
        }
        let across = 1 - axis;
        let lo = (rect.min[across] / tile).floor() as i32;
        let hi = ((rect.max[across] - edge) / tile).floor() as i32;
        let blocked = |line: i32, solid: &mut dyn FnMut(IVec2) -> bool| {
            (lo..=hi).any(|other| {
                let mut cell = IVec2::ZERO;
                cell[axis] = line;
                cell[across] = other;
                solid(cell)
            })
        };
        let mut moved = d;
        if d > 0.0 {
            let from = ((rect.max[axis] - edge) / tile).floor() as i32 + 1;
            let to = ((rect.max[axis] + d - edge) / tile).floor() as i32;
            for line in from..=to {
                if blocked(line, &mut solid) {
                    moved = line as f32 * tile - rect.max[axis];
                    if axis == 0 {
                        contacts.right = true;
                    } else {
                        contacts.bottom = true;
                    }
                    break;
                }
            }
        } else {
            let from = (rect.min[axis] / tile).floor() as i32 - 1;
            let to = ((rect.min[axis] + d) / tile).floor() as i32;
            for line in (to..=from).rev() {
                if blocked(line, &mut solid) {
                    moved = (line + 1) as f32 * tile - rect.min[axis];
                    if axis == 0 {
                        contacts.left = true;
                    } else {
                        contacts.top = true;
                    }
                    break;
                }
            }
        }
        let mut offset = Vec2::ZERO;
        offset[axis] = moved;
        rect = rect.translate(offset);
    }
    Slide {
        bounds: rect,
        moved: rect.min - bounds.min,
        contacts,
    }
}

/// Moves `bounds` by `motion` among solid rectangles, stopping flush
/// against them, x first and then y, as [`move_and_slide`] does on a
/// grid.
pub fn slide_against(bounds: Rect, motion: Vec2, obstacles: &[Rect]) -> Slide {
    let mut rect = bounds;
    let mut contacts = Contacts::default();
    for axis in 0..2 {
        let d = motion[axis];
        if d == 0.0 {
            continue;
        }
        let across = 1 - axis;
        let mut moved = d;
        let mut hit = false;
        for obstacle in obstacles {
            let beside =
                rect.max[across] > obstacle.min[across] && rect.min[across] < obstacle.max[across];
            if !beside {
                continue;
            }
            if d > 0.0 && obstacle.min[axis] >= rect.max[axis] {
                let gap = obstacle.min[axis] - rect.max[axis];
                if gap < moved {
                    moved = gap;
                    hit = true;
                }
            } else if d < 0.0 && obstacle.max[axis] <= rect.min[axis] {
                let gap = obstacle.max[axis] - rect.min[axis];
                if gap > moved {
                    moved = gap;
                    hit = true;
                }
            }
        }
        if hit {
            match (axis, d > 0.0) {
                (0, true) => contacts.right = true,
                (0, false) => contacts.left = true,
                (_, true) => contacts.bottom = true,
                (_, false) => contacts.top = true,
            }
        }
        let mut offset = Vec2::ZERO;
        offset[axis] = moved;
        rect = rect.translate(offset);
    }
    Slide {
        bounds: rect,
        moved: rect.min - bounds.min,
        contacts,
    }
}

/// Things filed by the grid cells their bounds cover, to find what is
/// near a point or a box without testing everything against everything.
///
/// Rebuild it each frame ([`clear`](SpatialHash::clear), then
/// [`insert`](SpatialHash::insert) everything that moves) and query it
/// for each bullet or each enemy.
///
/// ```
/// use smitten_kit_core::collision::SpatialHash;
/// use smitten_kit_core::math::Rect;
///
/// let mut near = SpatialHash::new(64.0);
/// near.insert(Rect::from_center_size((10.0, 10.0), (8.0, 8.0)), "slime");
/// near.insert(Rect::from_center_size((500.0, 10.0), (8.0, 8.0)), "bat");
///
/// let around_player = Rect::from_center_size((0.0, 0.0), (64.0, 64.0));
/// let found: Vec<_> = near.query(around_player).collect();
/// assert_eq!(found, [&"slime"]);
/// ```
#[derive(Clone, Debug)]
pub struct SpatialHash<T> {
    cell: f32,
    entries: Vec<(Rect, T)>,
    cells: HashMap<IVec2, Vec<usize>>,
}

impl<T> SpatialHash<T> {
    /// An empty hash with square cells `cell` units wide. A cell about
    /// the size of the things in it, or of the areas queried, works well.
    pub fn new(cell: f32) -> Self {
        SpatialHash {
            cell: if cell > 0.0 { cell } else { 1.0 },
            entries: Vec::new(),
            cells: HashMap::new(),
        }
    }

    fn cells_of(&self, rect: Rect) -> impl Iterator<Item = IVec2> + use<T> {
        let lo = (rect.min / self.cell).floor().as_ivec2();
        let hi = (rect.max / self.cell).floor().as_ivec2();
        (lo.y..=hi.y).flat_map(move |y| (lo.x..=hi.x).map(move |x| IVec2::new(x, y)))
    }

    /// Files `value` under every cell `bounds` covers.
    pub fn insert(&mut self, bounds: Rect, value: T) {
        let index = self.entries.len();
        for cell in self.cells_of(bounds) {
            self.cells.entry(cell).or_default().push(index);
        }
        self.entries.push((bounds, value));
    }

    /// Removes everything, keeping the memory for the next frame.
    pub fn clear(&mut self) {
        self.entries.clear();
        for list in self.cells.values_mut() {
            list.clear();
        }
    }

    /// How many things it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn candidates(&self, area: Rect) -> Vec<usize> {
        let mut found: Vec<usize> = self
            .cells_of(area)
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .copied()
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Everything whose bounds overlap `area` (touching counts), each
    /// once, in the order inserted.
    pub fn query(&self, area: Rect) -> impl Iterator<Item = &T> {
        self.query_bounds(area).map(|(_, value)| value)
    }

    /// Like [`query`](SpatialHash::query), with each thing's bounds.
    pub fn query_bounds(&self, area: Rect) -> impl Iterator<Item = (&Rect, &T)> {
        self.candidates(area).into_iter().filter_map(move |index| {
            let (bounds, value) = &self.entries[index];
            bounds.intersects(&area).then_some((bounds, value))
        })
    }

    /// Everything whose bounds hold `point`.
    pub fn query_point(&self, point: Vec2) -> impl Iterator<Item = &T> {
        self.query(Rect::new(point, point))
    }

    /// Every pair of things whose bounds overlap, each pair once, earlier
    /// inserted first.
    pub fn pairs(&self) -> Vec<(&T, &T)> {
        let mut pairs = Vec::new();
        for list in self.cells.values() {
            for (i, &a) in list.iter().enumerate() {
                for &b in &list[i + 1..] {
                    pairs.push((a.min(b), a.max(b)));
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        pairs
            .into_iter()
            .filter(|&(a, b)| self.entries[a].0.intersects(&self.entries[b].0))
            .map(|(a, b)| (&self.entries[a].1, &self.entries[b].1))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, size: f32) -> Rect {
        Rect::from_position_size((x, y), (size, size))
    }

    #[test]
    fn aabb_pushes_along_the_shallow_axis() {
        let a = square(0.0, 0.0, 10.0);
        assert_eq!(
            aabb_overlap(&a, &square(0.0, 9.0, 10.0)),
            Some(Vec2::new(0.0, -1.0))
        );
        assert_eq!(
            aabb_overlap(&a, &square(-7.0, 1.0, 10.0)),
            Some(Vec2::new(3.0, 0.0))
        );
        assert_eq!(aabb_overlap(&a, &square(10.0, 0.0, 10.0)), None);
    }

    #[test]
    fn circles_push_apart() {
        let a = Circle::new((0.0, 0.0), 2.0);
        let b = Circle::new((3.0, 0.0), 2.0);
        assert_eq!(circle_overlap(&a, &b), Some(Vec2::new(-1.0, 0.0)));
        assert_eq!(circle_overlap(&a, &Circle::new((4.0, 0.0), 2.0)), None);
        let r = square(1.0, -5.0, 10.0);
        assert_eq!(circle_rect_overlap(&a, &r), Some(Vec2::new(-1.0, 0.0)));
        // The center inside: out the nearest side.
        let inside = Circle::new((2.0, 0.0), 1.0);
        assert_eq!(circle_rect_overlap(&inside, &r), Some(Vec2::new(-2.0, 0.0)));
        assert_eq!(
            circle_rect_overlap(&Circle::new((-3.0, 0.0), 2.0), &r),
            None
        );
    }

    #[test]
    fn polygons() {
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(0.0, 4.0),
        ];
        let diamond = [
            Vec2::new(5.0, 2.0),
            Vec2::new(3.0, 4.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(3.0, 0.0),
        ]
        .map(|p| p + Vec2::new(2.5, 0.0));
        let push = polygon_overlap(&square, &diamond).unwrap();
        assert!((push - Vec2::new(-0.5, 0.0)).length() < 1e-5, "{push}");
        let far = diamond.map(|p| p + Vec2::new(10.0, 0.0));
        assert_eq!(polygon_overlap(&square, &far), None);
        // Same answer as the box test for boxes.
        let shifted = square.map(|p| p + Vec2::new(1.0, 3.0));
        let push = polygon_overlap(&square, &shifted).unwrap();
        assert!((push - Vec2::new(0.0, -1.0)).length() < 1e-5, "{push}");
    }

    #[test]
    fn rays() {
        let wall = square(10.0, -5.0, 10.0);
        let hit = ray_rect(Vec2::ZERO, Vec2::new(20.0, 0.0), &wall).unwrap();
        assert_eq!(hit.t, 0.5);
        assert_eq!(hit.point, Vec2::new(10.0, 0.0));
        assert_eq!(hit.normal, Vec2::NEG_X);
        let from_right = ray_rect(Vec2::new(30.0, 0.0), Vec2::new(-1.0, 0.0), &wall).unwrap();
        assert_eq!(from_right.normal, Vec2::X);
        assert_eq!(from_right.t, 10.0);
        assert!(ray_rect(Vec2::ZERO, Vec2::new(-1.0, 0.0), &wall).is_none());
        assert!(ray_rect(Vec2::new(0.0, 6.0), Vec2::X, &wall).is_none());
        let down = ray_rect(Vec2::new(15.0, -20.0), Vec2::Y, &wall).unwrap();
        assert_eq!(down.normal, Vec2::NEG_Y);

        let ball = Circle::new((10.0, 0.0), 2.0);
        let hit = ray_circle(Vec2::ZERO, Vec2::X, &ball).unwrap();
        assert_eq!(hit.t, 8.0);
        assert_eq!(hit.normal, Vec2::NEG_X);
        assert!(ray_circle(Vec2::ZERO, Vec2::NEG_X, &ball).is_none());
        assert!(ray_circle(Vec2::new(0.0, 3.0), Vec2::X, &ball).is_none());
    }

    #[test]
    fn slide_stops_at_walls_and_floors() {
        let level = [
            "........", //
            "......#.", //
            "........", //
            "########",
        ];
        let solid = |cell: IVec2| {
            level
                .get(cell.y as usize)
                .and_then(|row| row.as_bytes().get(cell.x as usize))
                == Some(&b'#')
        };
        // Standing on the floor: walking does not catch on it.
        let player = Rect::from_position_size((0.0, 32.0), (16.0, 16.0));
        let walk = move_and_slide(player, Vec2::new(5.0, 0.0), 16.0, solid);
        assert_eq!(walk.moved, Vec2::new(5.0, 0.0));
        assert!(!walk.contacts.any());
        // Falling fast lands on the floor instead of passing through it.
        let high = Rect::from_position_size((0.0, 0.0), (16.0, 16.0));
        let fall = move_and_slide(high, Vec2::new(0.0, 500.0), 16.0, solid);
        assert_eq!(fall.bounds.min.y, 32.0);
        assert!(fall.contacts.bottom);
        // Running into the block stops flush against it.
        let run = Rect::from_position_size((60.0, 16.0), (16.0, 16.0));
        let hit = move_and_slide(run, Vec2::new(30.0, 0.0), 16.0, solid);
        assert_eq!(hit.bounds.max.x, 96.0);
        assert!(hit.contacts.right);
        let back = move_and_slide(
            Rect::from_position_size((120.0, 16.0), (8.0, 8.0)),
            Vec2::new(-40.0, 0.0),
            16.0,
            solid,
        );
        assert_eq!(back.bounds.min.x, 112.0);
        assert!(back.contacts.left);
        // Jumping into it from below hits its underside.
        let jump = move_and_slide(
            Rect::from_position_size((100.0, 40.0), (8.0, 8.0)),
            Vec2::new(0.0, -30.0),
            16.0,
            solid,
        );
        assert_eq!(jump.bounds.min.y, 32.0);
        assert!(jump.contacts.top);
    }

    #[test]
    fn slide_against_rects() {
        let walls = [square(20.0, 0.0, 10.0), square(-50.0, 30.0, 100.0)];
        let slide = slide_against(square(0.0, 0.0, 10.0), Vec2::new(15.0, 25.0), &walls);
        assert_eq!(slide.bounds.min, Vec2::new(10.0, 20.0));
        assert!(slide.contacts.right && slide.contacts.bottom);
    }

    #[test]
    fn spatial_hash_finds_each_once() {
        let mut hash = SpatialHash::new(10.0);
        hash.insert(square(0.0, 0.0, 25.0), 'a'); // spans 9 cells
        hash.insert(square(20.0, 20.0, 5.0), 'b');
        hash.insert(square(100.0, 100.0, 5.0), 'c');
        let found: Vec<_> = hash.query(square(0.0, 0.0, 30.0)).collect();
        assert_eq!(found, [&'a', &'b']);
        assert_eq!(hash.query_point(Vec2::new(102.0, 102.0)).count(), 1);
        assert_eq!(hash.pairs(), [(&'a', &'b')]);
        hash.clear();
        assert!(hash.is_empty());
        assert_eq!(hash.query(square(0.0, 0.0, 30.0)).count(), 0);
    }
}

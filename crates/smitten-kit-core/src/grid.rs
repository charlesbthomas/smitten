//! A 2D grid of cells, with A* paths, flood fills and distance maps on
//! it.
//!
//! Cells are named by [`IVec2`] (`x` right, `y` down). [`astar`] also
//! works without a [`Grid`], on any graph a closure can describe.
//!
//! ```
//! use smitten_kit_core::glam::IVec2;
//! use smitten_kit_core::grid::{Grid, Neighbors};
//!
//! let map = Grid::from_rows(&[
//!     "#####",
//!     "#..##",
//!     "##..#",
//!     "#####",
//! ], |c| c == '#');
//!
//! let path = map
//!     .path(IVec2::new(1, 1), IVec2::new(3, 2), Neighbors::Four, |_, wall| !wall)
//!     .unwrap();
//! assert_eq!(path, [IVec2::new(2, 1), IVec2::new(2, 2), IVec2::new(3, 2)]);
//! ```

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::hash::Hash;
use std::ops::{Index, IndexMut};

use glam::{IVec2, Vec2};

/// Which cells count as next to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Neighbors {
    /// Up, down, left and right.
    Four,
    /// The four sides and the four corners. A path never cuts a corner:
    /// it steps diagonally only when both cells beside the step are open.
    Eight,
}

const SIDES: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];
const CORNERS: [IVec2; 4] = [
    IVec2::new(1, 1),
    IVec2::new(-1, 1),
    IVec2::new(1, -1),
    IVec2::new(-1, -1),
];

/// The cost of a step to a side; a diagonal costs 14, about 10 times √2.
pub const STEP: u32 = 10;
/// The cost of a diagonal step.
pub const DIAGONAL_STEP: u32 = 14;

impl Neighbors {
    /// The offsets to the neighboring cells.
    pub fn offsets(self) -> &'static [IVec2] {
        const ALL: [IVec2; 8] = [
            SIDES[0], SIDES[1], SIDES[2], SIDES[3], CORNERS[0], CORNERS[1], CORNERS[2], CORNERS[3],
        ];
        match self {
            Neighbors::Four => &SIDES,
            Neighbors::Eight => &ALL,
        }
    }

    /// The cheapest cost from `from` to `to` if nothing is in the way, in
    /// the same units as [`STEP`]. This is the A* estimate.
    pub fn estimate(self, from: IVec2, to: IVec2) -> u32 {
        let d = (to - from).abs();
        let (dx, dy) = (d.x as u32, d.y as u32);
        match self {
            Neighbors::Four => (dx + dy) * STEP,
            Neighbors::Eight => {
                let (low, high) = (dx.min(dy), dx.max(dy));
                low * DIAGONAL_STEP + (high - low) * STEP
            }
        }
    }
}

/// The cell holding world point `point` when each cell is `tile` units
/// wide.
pub fn cell_at(point: Vec2, tile: f32) -> IVec2 {
    (point / tile).floor().as_ivec2()
}

/// A `width` by `height` grid of `T`, stored row by row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Grid<T> {
    width: i32,
    height: i32,
    cells: Vec<T>,
}

impl<T> Grid<T> {
    /// A grid with every cell made by `make(cell)`.
    pub fn from_fn(width: u32, height: u32, mut make: impl FnMut(IVec2) -> T) -> Grid<T> {
        let (width, height) = (width as i32, height as i32);
        let mut cells = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                cells.push(make(IVec2::new(x, y)));
            }
        }
        Grid {
            width,
            height,
            cells,
        }
    }

    /// A grid from cells listed row by row. `None` when `cells` does not
    /// hold `width * height` of them.
    pub fn from_vec(width: u32, height: u32, cells: Vec<T>) -> Option<Grid<T>> {
        (cells.len() == width as usize * height as usize).then_some(Grid {
            width: width as i32,
            height: height as i32,
            cells,
        })
    }

    /// A grid from rows of text, one cell per character, as level layouts
    /// in a test or a small game are written. Short rows are padded with
    /// spaces.
    pub fn from_rows(rows: &[&str], mut cell: impl FnMut(char) -> T) -> Grid<T> {
        let width = rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0);
        let mut cells = Vec::with_capacity(width * rows.len());
        for row in rows {
            let mut chars = row.chars();
            for _ in 0..width {
                cells.push(cell(chars.next().unwrap_or(' ')));
            }
        }
        Grid {
            width: width as i32,
            height: rows.len() as i32,
            cells,
        }
    }

    /// Cells across.
    pub fn width(&self) -> u32 {
        self.width as u32
    }

    /// Cells down.
    pub fn height(&self) -> u32 {
        self.height as u32
    }

    /// Width and height.
    pub fn size(&self) -> IVec2 {
        IVec2::new(self.width, self.height)
    }

    /// Whether `cell` is on the grid.
    pub fn contains(&self, cell: IVec2) -> bool {
        cell.x >= 0 && cell.y >= 0 && cell.x < self.width && cell.y < self.height
    }

    fn index(&self, cell: IVec2) -> Option<usize> {
        self.contains(cell)
            .then(|| (cell.y * self.width + cell.x) as usize)
    }

    /// The cell, or `None` off the grid.
    pub fn get(&self, cell: IVec2) -> Option<&T> {
        self.index(cell).map(|index| &self.cells[index])
    }

    /// The cell to change, or `None` off the grid.
    pub fn get_mut(&mut self, cell: IVec2) -> Option<&mut T> {
        self.index(cell).map(|index| &mut self.cells[index])
    }

    /// Sets the cell. Returns false (and changes nothing) off the grid.
    pub fn set(&mut self, cell: IVec2, value: T) -> bool {
        match self.get_mut(cell) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    /// Every cell with its position, row by row.
    pub fn iter(&self) -> impl Iterator<Item = (IVec2, &T)> {
        let width = self.width;
        self.cells
            .iter()
            .enumerate()
            .map(move |(i, value)| (IVec2::new(i as i32 % width, i as i32 / width), value))
    }

    /// Every cell to change, with its position, row by row.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (IVec2, &mut T)> {
        let width = self.width;
        self.cells
            .iter_mut()
            .enumerate()
            .map(move |(i, value)| (IVec2::new(i as i32 % width, i as i32 / width), value))
    }

    /// The cells as one slice, row by row.
    pub fn as_slice(&self) -> &[T] {
        &self.cells
    }

    /// The neighbors of `cell` that are on the grid.
    pub fn neighbors(&self, cell: IVec2, neighbors: Neighbors) -> impl Iterator<Item = IVec2> + '_ {
        neighbors
            .offsets()
            .iter()
            .map(move |offset| cell + *offset)
            .filter(|next| self.contains(*next))
    }

    /// A grid of the same size with each cell mapped.
    pub fn map<U>(&self, mut f: impl FnMut(IVec2, &T) -> U) -> Grid<U> {
        Grid {
            width: self.width,
            height: self.height,
            cells: self.iter().map(|(cell, value)| f(cell, value)).collect(),
        }
    }

    /// The cells reachable from `start` through cells where `open` is
    /// true, nearest first. Empty when `start` itself is closed or off
    /// the grid.
    pub fn flood_fill(
        &self,
        start: IVec2,
        neighbors: Neighbors,
        mut open: impl FnMut(IVec2, &T) -> bool,
    ) -> Vec<IVec2> {
        let mut seen = vec![false; self.cells.len()];
        let mut out = Vec::new();
        let mut queue = VecDeque::new();
        if let Some(index) = self.index(start)
            && open(start, &self.cells[index])
        {
            seen[index] = true;
            queue.push_back(start);
        }
        while let Some(cell) = queue.pop_front() {
            out.push(cell);
            for &offset in neighbors.offsets() {
                let next = cell + offset;
                let Some(index) = self.index(next) else {
                    continue;
                };
                if !seen[index] && open(next, &self.cells[index]) {
                    seen[index] = true;
                    queue.push_back(next);
                }
            }
        }
        out
    }

    /// How many steps each cell is from `start` through open cells, or
    /// `None` where it cannot be reached. A monster that walks downhill
    /// on this map walks toward `start` (a "Dijkstra map").
    pub fn distances(
        &self,
        start: IVec2,
        neighbors: Neighbors,
        mut open: impl FnMut(IVec2, &T) -> bool,
    ) -> Grid<Option<u32>> {
        let mut out = Grid {
            width: self.width,
            height: self.height,
            cells: vec![None; self.cells.len()],
        };
        let mut queue = VecDeque::new();
        if let Some(index) = self.index(start)
            && open(start, &self.cells[index])
        {
            out.cells[index] = Some(0);
            queue.push_back((start, 0));
        }
        while let Some((cell, steps)) = queue.pop_front() {
            for &offset in neighbors.offsets() {
                let next = cell + offset;
                let Some(index) = self.index(next) else {
                    continue;
                };
                if out.cells[index].is_none() && open(next, &self.cells[index]) {
                    out.cells[index] = Some(steps + 1);
                    queue.push_back((next, steps + 1));
                }
            }
        }
        out
    }

    /// The shortest path from `from` to `to` through cells where `open`
    /// is true, as the cells stepped onto: `from` is left out and `to` is
    /// last. `to` need not be open, so a monster can path to the player
    /// it means to hit. `None` when there is no way.
    pub fn path(
        &self,
        from: IVec2,
        to: IVec2,
        neighbors: Neighbors,
        mut open: impl FnMut(IVec2, &T) -> bool,
    ) -> Option<Vec<IVec2>> {
        if !self.contains(from) || !self.contains(to) {
            return None;
        }
        let mut passable =
            |cell: IVec2| cell == to || self.get(cell).is_some_and(|value| open(cell, value));
        astar(
            from,
            to,
            |cell, out| {
                for &offset in neighbors.offsets() {
                    let next = cell + offset;
                    if !passable(next) {
                        continue;
                    }
                    let diagonal = offset.x != 0 && offset.y != 0;
                    if diagonal
                        && !(passable(cell + IVec2::new(offset.x, 0))
                            && passable(cell + IVec2::new(0, offset.y)))
                    {
                        continue;
                    }
                    out.push((next, if diagonal { DIAGONAL_STEP } else { STEP }));
                }
            },
            |cell| neighbors.estimate(cell, to),
            self.cells.len(),
        )
    }
}

impl<T: Clone> Grid<T> {
    /// A grid with every cell set to `fill`.
    pub fn new(width: u32, height: u32, fill: T) -> Grid<T> {
        Grid {
            width: width as i32,
            height: height as i32,
            cells: vec![fill; width as usize * height as usize],
        }
    }

    /// Sets every cell to `value`.
    pub fn fill(&mut self, value: T) {
        self.cells.fill(value);
    }
}

impl<T> Index<IVec2> for Grid<T> {
    type Output = T;

    /// # Panics
    ///
    /// Off the grid. Use [`get`](Grid::get) where that can happen.
    fn index(&self, cell: IVec2) -> &T {
        match self.get(cell) {
            Some(value) => value,
            None => panic!("{cell} is off a {}x{} grid", self.width, self.height),
        }
    }
}

impl<T> IndexMut<IVec2> for Grid<T> {
    fn index_mut(&mut self, cell: IVec2) -> &mut T {
        let (width, height) = (self.width, self.height);
        match self.get_mut(cell) {
            Some(value) => value,
            None => panic!("{cell} is off a {width}x{height} grid"),
        }
    }
}

/// A* on any graph: the cheapest path from `start` to `goal`.
///
/// - `neighbors(node, out)` pushes each node one step from `node`, with
///   the step's cost.
/// - `estimate(node)` guesses the cost from `node` to `goal`. A guess
///   that never overestimates gives the cheapest path; 0 everywhere makes
///   this Dijkstra's search.
/// - `limit` caps how many nodes are looked at, so a goal that cannot be
///   reached on a large or endless graph gives up instead of searching
///   forever.
///
/// The path leaves out `start` and ends at `goal`. Ties between equally
/// good nodes go to the one nearer the goal, then to the one found first,
/// so the same search gives the same path every time.
///
/// ```
/// use smitten_kit_core::grid::astar;
///
/// // A number line where a step of +1 costs 1 and doubling costs 1.
/// let path = astar(
///     1_i32,
///     10,
///     |n, out| {
///         out.push((n + 1, 1));
///         out.push((n * 2, 1));
///     },
///     |_| 0,
///     1_000,
/// );
/// assert_eq!(path, Some(vec![2, 4, 5, 10]));
/// ```
pub fn astar<N: Copy + Eq + Hash>(
    start: N,
    goal: N,
    mut neighbors: impl FnMut(N, &mut Vec<(N, u32)>),
    mut estimate: impl FnMut(N) -> u32,
    limit: usize,
) -> Option<Vec<N>> {
    if start == goal {
        return Some(Vec::new());
    }
    // node -> (cost so far, the node it was reached from)
    let mut best: HashMap<N, (u32, N)> = HashMap::new();
    best.insert(start, (0, start));
    let mut nodes = vec![start];
    let mut frontier = BinaryHeap::new();
    frontier.push(Reverse((estimate(start), u32::MAX, 0_usize, 0_u32)));
    let mut steps = Vec::new();
    let mut looked_at = 0;

    while let Some(Reverse((_, _, slot, so_far))) = frontier.pop() {
        let node = nodes[slot];
        if node == goal {
            let mut path = vec![goal];
            let mut at = goal;
            while let Some(&(_, previous)) = best.get(&at) {
                if previous == start {
                    break;
                }
                path.push(previous);
                at = previous;
            }
            path.reverse();
            return Some(path);
        }
        if best.get(&node).is_some_and(|&(cost, _)| so_far > cost) {
            continue; // a cheaper way here was found after this was queued
        }
        looked_at += 1;
        if looked_at > limit {
            return None;
        }
        steps.clear();
        neighbors(node, &mut steps);
        for &(next, step) in &steps {
            let cost = so_far.saturating_add(step);
            if best.get(&next).is_some_and(|&(known, _)| known <= cost) {
                continue;
            }
            best.insert(next, (cost, node));
            let guess = estimate(next);
            nodes.push(next);
            frontier.push(Reverse((
                cost.saturating_add(guess),
                guess,
                nodes.len() - 1,
                cost,
            )));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walls(rows: &[&str]) -> Grid<bool> {
        Grid::from_rows(rows, |c| c == '#')
    }

    #[test]
    fn get_set_and_bounds() {
        let mut grid = Grid::new(3, 2, 0);
        assert!(grid.set(IVec2::new(2, 1), 5));
        assert!(!grid.set(IVec2::new(3, 0), 5));
        assert_eq!(grid[IVec2::new(2, 1)], 5);
        assert_eq!(grid.get(IVec2::new(-1, 0)), None);
        assert_eq!(grid.iter().filter(|(_, v)| **v == 5).count(), 1);
        assert_eq!(grid.neighbors(IVec2::ZERO, Neighbors::Eight).count(), 3);
        assert_eq!(Grid::from_vec(2, 2, vec![1, 2, 3]), None);
    }

    #[test]
    fn path_goes_around_walls() {
        let map = walls(&[
            "......", //
            ".####.", //
            "......",
        ]);
        let path = map
            .path(
                IVec2::new(0, 1),
                IVec2::new(5, 1),
                Neighbors::Four,
                |_, w| !w,
            )
            .unwrap();
        assert_eq!(path.len(), 7);
        assert_eq!(path.last(), Some(&IVec2::new(5, 1)));
        // Every step is to a side.
        let mut at = IVec2::new(0, 1);
        for step in &path {
            assert_eq!((*step - at).abs().element_sum(), 1);
            at = *step;
        }
    }

    #[test]
    fn eight_way_paths_do_not_cut_corners() {
        let map = walls(&[
            ".#", //
            "..",
        ]);
        let path = map
            .path(
                IVec2::new(0, 0),
                IVec2::new(1, 1),
                Neighbors::Eight,
                |_, w| !w,
            )
            .unwrap();
        assert_eq!(path, [IVec2::new(0, 1), IVec2::new(1, 1)]);
        let open = walls(&["..", ".."]);
        let path = open
            .path(
                IVec2::new(0, 0),
                IVec2::new(1, 1),
                Neighbors::Eight,
                |_, w| !w,
            )
            .unwrap();
        assert_eq!(path, [IVec2::new(1, 1)]);
    }

    #[test]
    fn walled_off_goal_has_no_path() {
        let map = walls(&[
            "..#..", //
            "..#..",
        ]);
        assert_eq!(
            map.path(IVec2::ZERO, IVec2::new(4, 0), Neighbors::Eight, |_, w| !w),
            None
        );
        // The goal itself may be closed: a monster paths to the player.
        assert!(
            map.path(IVec2::ZERO, IVec2::new(2, 0), Neighbors::Four, |_, w| !w)
                .is_some()
        );
    }

    #[test]
    fn astar_gives_up_at_the_limit() {
        // An endless plane with the goal walled off can't be searched to
        // the end.
        let path = astar(
            IVec2::ZERO,
            IVec2::new(1000, 0),
            |cell, out| {
                for &o in Neighbors::Four.offsets() {
                    if cell + o != IVec2::new(1000, 0) {
                        out.push((cell + o, 1));
                    }
                }
            },
            |_| 0,
            500,
        );
        assert_eq!(path, None);
    }

    #[test]
    fn flood_fill_and_distances() {
        let map = walls(&[
            "..#.", //
            "..#.",
        ]);
        let filled = map.flood_fill(IVec2::ZERO, Neighbors::Four, |_, w| !w);
        assert_eq!(filled.len(), 4);
        assert_eq!(filled[0], IVec2::ZERO);
        let d = map.distances(IVec2::ZERO, Neighbors::Four, |_, w| !w);
        assert_eq!(d[IVec2::new(1, 1)], Some(2));
        assert_eq!(d[IVec2::new(3, 0)], None);
        assert!(
            map.flood_fill(IVec2::new(2, 0), Neighbors::Four, |_, w| !w)
                .is_empty()
        );
    }

    #[test]
    fn estimates() {
        assert_eq!(Neighbors::Four.estimate(IVec2::ZERO, IVec2::new(3, -2)), 50);
        assert_eq!(
            Neighbors::Eight.estimate(IVec2::ZERO, IVec2::new(3, -2)),
            38
        );
        assert_eq!(cell_at(Vec2::new(-0.5, 31.9), 16.0), IVec2::new(-1, 1));
    }
}

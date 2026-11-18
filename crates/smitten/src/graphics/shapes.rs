//! Turns shapes into triangles, in the shape's own coordinates.
//!
//! Everything here is plain geometry. The caller applies the current
//! transform and color afterwards.

type Point = (f32, f32);

/// How far a mitered corner may stick out, as a multiple of the half width.
/// Without a limit, very sharp corners would produce long spikes.
const MITER_LIMIT: f32 = 4.0;

/// Triangles filling a convex polygon.
pub(crate) fn fill_convex(points: &[Point]) -> Vec<Point> {
    if points.len() < 3 {
        return Vec::new();
    }
    let mut triangles = Vec::with_capacity((points.len() - 2) * 3);
    for pair in points[1..].windows(2) {
        triangles.extend_from_slice(&[points[0], pair[0], pair[1]]);
    }
    triangles
}

/// Returns a function giving each point its place in the box around
/// `points`: `[0, 0]` at the top-left and `[1, 1]` at the bottom-right.
pub(crate) fn spread_uv(points: &[Point]) -> impl Fn(f32, f32) -> [f32; 2] {
    let mut low = (f32::MAX, f32::MAX);
    let mut high = (f32::MIN, f32::MIN);
    for &(x, y) in points {
        low = (low.0.min(x), low.1.min(y));
        high = (high.0.max(x), high.1.max(y));
    }
    // A shape with no width or height has nothing to spread across.
    let span = |low: f32, high: f32| if high - low > 1e-9 { high - low } else { 1.0 };
    let (width, height) = (span(low.0, high.0), span(low.1, high.1));
    move |x, y| [(x - low.0) / width, (y - low.1) / height]
}

/// The shadow a convex polygon casts away from `light`, as triangles:
/// each edge facing away from the light, stretched `length` further.
pub(crate) fn shadow(light: Point, corners: &[Point], length: f32) -> Vec<Point> {
    let mut triangles = Vec::new();
    if corners.len() < 2 {
        return triangles;
    }
    let edges = || (0..corners.len()).map(|i| (corners[i], corners[(i + 1) % corners.len()]));
    // Which way round the corners go, so "facing away" can be told from
    // "facing toward" whatever order they were listed in.
    let winding = edges()
        .map(|((ax, ay), (bx, by))| ax * by - bx * ay)
        .sum::<f32>()
        .signum();
    let far = |(x, y): Point| {
        let (dx, dy) = (x - light.0, y - light.1);
        let distance = dx.hypot(dy).max(1e-6);
        (x + dx / distance * length, y + dy / distance * length)
    };
    for (a, b) in edges() {
        let outward = ((b.1 - a.1) * winding, (a.0 - b.0) * winding);
        let toward_light = outward.0 * (light.0 - a.0) + outward.1 * (light.1 - a.1);
        if toward_light >= 0.0 {
            continue;
        }
        let (far_a, far_b) = (far(a), far(b));
        triangles.extend_from_slice(&[a, b, far_b, a, far_b, far_a]);
    }
    triangles
}

/// How the corners of lines and outlines are drawn, as LÖVE's `LineJoin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineJoin {
    /// The edges carry on until they meet in a point. Very sharp corners
    /// are cut off as `Bevel` does, so they do not grow into long spikes.
    #[default]
    Miter,
    /// The corner is cut off flat.
    Bevel,
    /// Each stretch of line is drawn on its own, leaving a notch on the
    /// outside of every corner.
    None,
}

/// Whether the edges of lines are softened, as LÖVE's `LineStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineStyle {
    /// The edges fade out over one pixel, which hides the staircase on
    /// slanted lines.
    #[default]
    Smooth,
    /// The edges are hard. Right for pixel art.
    Rough,
}

/// How a line is drawn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Stroke {
    pub width: f32,
    pub join: LineJoin,
    /// How far the soft edge reaches, in the shape's own coordinates:
    /// the size of one pixel for a smooth line, zero for a rough one.
    pub fringe: f32,
}

/// A triangle corner and how solid the color is there, from 0 to 1.
pub(crate) type Covered = (f32, f32, f32);

/// The two sides of one stretch of line at some distance from its middle:
/// where each side starts and where it ends.
#[derive(Clone, Copy)]
struct Rails {
    start_left: Point,
    end_left: Point,
    start_right: Point,
    end_right: Point,
}

/// Triangles for a line through `points` in order. A closed line joins
/// its last point back to its first; an open one has square ends that
/// stop at the first and last point.
pub(crate) fn stroke(points: &[Point], closed: bool, stroke: &Stroke) -> Vec<Covered> {
    // Repeated points have no direction and would break the corners.
    let mut path: Vec<Point> = Vec::with_capacity(points.len());
    for &point in points {
        if path.last() != Some(&point) {
            path.push(point);
        }
    }
    if closed && path.len() > 1 && path.first() == path.last() {
        path.pop();
    }
    let count = path.len();
    if count < 2 || (closed && count < 3) {
        return Vec::new();
    }
    let segments = if closed { count } else { count - 1 };
    let normals: Vec<Point> = (0..segments)
        .map(|i| normal(path[i], path[(i + 1) % count]))
        .collect();

    let fringe = stroke.fringe.max(0.0);
    // The soft edge straddles the line's true edge, so that a smooth line
    // looks as wide as a rough one.
    let core = (stroke.width / 2.0 - fringe / 2.0).max(0.0);
    // A line thinner than a pixel is drawn fainter instead.
    let peak = if fringe > 0.0 {
        (stroke.width / fringe).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let inner = rails(&path, &normals, closed, stroke.join, core);
    let mut outer = rails(&path, &normals, closed, stroke.join, core + fringe);
    if fringe > 0.0 && !closed {
        // The ends fade out too.
        let (nx, ny) = normals[0];
        let first = &mut outer[0];
        first.start_left = (
            first.start_left.0 + ny * fringe,
            first.start_left.1 - nx * fringe,
        );
        first.start_right = (
            first.start_right.0 + ny * fringe,
            first.start_right.1 - nx * fringe,
        );
        let (nx, ny) = normals[segments - 1];
        let last = &mut outer[segments - 1];
        last.end_left = (last.end_left.0 - ny * fringe, last.end_left.1 + nx * fringe);
        last.end_right = (
            last.end_right.0 - ny * fringe,
            last.end_right.1 + nx * fringe,
        );
    }

    let mut triangles = Vec::with_capacity(segments * 24);
    let solid = |(x, y): Point| (x, y, peak);
    let clear = |(x, y): Point| (x, y, 0.0);
    let mut quad = |a: Covered, b: Covered, c: Covered, d: Covered| {
        triangles.extend_from_slice(&[a, b, c, a, c, d]);
    };
    for i in 0..segments {
        let (body, edge) = (inner[i], outer[i]);
        quad(
            solid(body.start_left),
            solid(body.end_left),
            solid(body.end_right),
            solid(body.start_right),
        );
        if fringe > 0.0 {
            quad(
                solid(body.start_left),
                solid(body.end_left),
                clear(edge.end_left),
                clear(edge.start_left),
            );
            quad(
                solid(body.start_right),
                solid(body.end_right),
                clear(edge.end_right),
                clear(edge.start_right),
            );
        }

        // The corner where this stretch hands over to the next.
        let has_next = closed || i + 1 < segments;
        if has_next && stroke.join != LineJoin::None {
            let next = (i + 1) % segments;
            let (next_body, next_edge) = (inner[next], outer[next]);
            // Whichever side is on the outside of the turn has a gap.
            for (from, to, pivot, edge_from, edge_to) in [
                (
                    body.end_left,
                    next_body.start_left,
                    next_body.start_right,
                    edge.end_left,
                    next_edge.start_left,
                ),
                (
                    body.end_right,
                    next_body.start_right,
                    next_body.start_left,
                    edge.end_right,
                    next_edge.start_right,
                ),
            ] {
                if edge_from == edge_to {
                    continue;
                }
                quad(solid(from), solid(to), solid(pivot), solid(pivot));
                if fringe > 0.0 {
                    quad(solid(from), solid(to), clear(edge_to), clear(edge_from));
                }
            }
        }
    }
    if fringe > 0.0 && !closed {
        let (first, first_edge) = (inner[0], outer[0]);
        quad(
            solid(first.start_left),
            solid(first.start_right),
            clear(first_edge.start_right),
            clear(first_edge.start_left),
        );
        let (last, last_edge) = (inner[segments - 1], outer[segments - 1]);
        quad(
            solid(last.end_left),
            solid(last.end_right),
            clear(last_edge.end_right),
            clear(last_edge.end_left),
        );
    }
    // Flat triangles, from corners that needed no filling, draw nothing.
    let mut kept = Vec::with_capacity(triangles.len());
    for triangle in triangles.as_chunks::<3>().0 {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        let doubled = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
        if doubled.abs() > 1e-9 {
            kept.extend_from_slice(triangle);
        }
    }
    kept
}

/// Cuts a path into dashes `dash` long with gaps of `gap` between them,
/// measured along the path. A dash that reaches a corner bends round it.
pub(crate) fn dashes(points: &[Point], dash: f32, gap: f32) -> Vec<Vec<Point>> {
    let mut pieces = Vec::new();
    if !(dash > 0.0 && gap >= 0.0 && dash.is_finite() && gap.is_finite()) {
        return pieces;
    }
    let mut current: Vec<Point> = Vec::new();
    // How much of the dash or gap in progress is left.
    let (mut drawing, mut left) = (true, dash);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        if length < 1e-6 {
            continue;
        }
        let at = |along: f32| {
            let t = along / length;
            (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
        };
        let mut along = 0.0;
        if drawing && current.is_empty() {
            current.push(from);
        }
        while length - along > left {
            along += left;
            if drawing {
                current.push(at(along));
                pieces.push(std::mem::take(&mut current));
                left = gap;
            } else {
                current.push(at(along));
                left = dash;
            }
            drawing = !drawing;
            // No gap at all means the dashes simply follow each other.
            if !drawing && gap == 0.0 {
                drawing = true;
                left = dash;
                current.push(at(along));
            }
        }
        left -= length - along;
        if drawing {
            current.push(to);
        }
    }
    if current.len() > 1 {
        pieces.push(current);
    }
    pieces
}

/// The sides of every stretch of a path, `distance` from its middle.
fn rails(
    path: &[Point],
    normals: &[Point],
    closed: bool,
    join: LineJoin,
    distance: f32,
) -> Vec<Rails> {
    let (count, segments) = (path.len(), normals.len());
    let offset = |(x, y): Point, (nx, ny): Point, by: f32| (x + nx * by, y + ny * by);
    let mut rails: Vec<Rails> = (0..segments)
        .map(|i| {
            let (start, end) = (path[i], path[(i + 1) % count]);
            Rails {
                start_left: offset(start, normals[i], distance),
                end_left: offset(end, normals[i], distance),
                start_right: offset(start, normals[i], -distance),
                end_right: offset(end, normals[i], -distance),
            }
        })
        .collect();
    if join == LineJoin::None {
        return rails;
    }
    for i in 0..segments {
        if !closed && i + 1 == segments {
            break;
        }
        let next = (i + 1) % segments;
        let corner = path[next % count];
        let (normal_in, normal_out) = (normals[i], normals[next]);
        let (mx, my) = (normal_in.0 + normal_out.0, normal_in.1 + normal_out.1);
        let length = (mx * mx + my * my).sqrt();
        if length < 1e-6 {
            // The path doubles back on itself, so its sides never meet.
            continue;
        }
        let (mx, my) = (mx / length, my / length);
        // The miter is longer than the distance by 1/cos of half the turn.
        let cos = mx * normal_in.0 + my * normal_in.1;
        let sharp = cos < 1.0 / MITER_LIMIT;
        let reach = distance / cos.max(1.0 / MITER_LIMIT);
        let left = offset(corner, (mx, my), reach);
        let right = offset(corner, (mx, my), -reach);
        // Positive when the path turns toward its right, which puts the
        // left side on the outside.
        let turn = normal_in.0 * normal_out.1 - normal_in.1 * normal_out.0;
        let cut = join == LineJoin::Bevel || sharp;
        if !(cut && turn > 0.0) {
            rails[i].end_left = left;
            rails[next].start_left = left;
        }
        if !(cut && turn < 0.0) {
            rails[i].end_right = right;
            rails[next].start_right = right;
        }
    }
    rails
}

/// Points around a circle, enough of them to look round at `radius` when
/// each unit is `scale` pixels.
pub(crate) fn circle_points(cx: f32, cy: f32, radius: f32, scale: f32) -> Vec<Point> {
    let radius = radius.abs();
    // Keeps each straight edge within about a third of a pixel of the true
    // curve, bounded so tiny and huge circles stay reasonable.
    let segments = segments_for(radius, scale);
    (0..segments)
        .map(|i| {
            let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect()
}

/// How many straight edges a full turn of radius `radius` needs to look
/// round when each unit is `scale` pixels.
fn segments_for(radius: f32, scale: f32) -> usize {
    (((radius.abs() * scale).sqrt() * 6.0).ceil() as usize).clamp(12, 256)
}

/// Points around an ellipse with radii `rx` and `ry`.
pub(crate) fn ellipse_points(cx: f32, cy: f32, rx: f32, ry: f32, scale: f32) -> Vec<Point> {
    let (rx, ry) = (rx.abs(), ry.abs());
    let segments = segments_for(rx.max(ry), scale);
    (0..segments)
        .map(|i| {
            let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect()
}

/// Points along part of an ellipse from angle `from` to angle `to`, both
/// ends included. Angles are in radians, clockwise from the x axis as y
/// points down. A sweep of more than a full turn is cut to one turn.
pub(crate) fn arc_points(
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    from: f32,
    to: f32,
    scale: f32,
) -> Vec<Point> {
    use std::f32::consts::TAU;
    let sweep = (to - from).clamp(-TAU, TAU);
    let whole = segments_for(rx.abs().max(ry.abs()), scale) as f32;
    let segments = ((whole * sweep.abs() / TAU).ceil() as usize).max(1);
    (0..=segments)
        .map(|i| {
            let angle = from + sweep * i as f32 / segments as f32;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect()
}

/// Points around a rectangle whose corners are quarter ellipses with radii
/// `rx` and `ry`. The radii are cut to half the rectangle's size.
pub(crate) fn rounded_rectangle_points(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rx: f32,
    ry: f32,
    scale: f32,
) -> Vec<Point> {
    use std::f32::consts::FRAC_PI_2;
    let rx = rx.abs().min(width.abs() / 2.0);
    let ry = ry.abs().min(height.abs() / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return vec![
            (x, y),
            (x + width, y),
            (x + width, y + height),
            (x, y + height),
        ];
    }
    let (left, right) = (x.min(x + width), x.max(x + width));
    let (top, bottom) = (y.min(y + height), y.max(y + height));
    // Clockwise on screen from the top-left corner.
    let corners = [
        (left + rx, top + ry, 2.0),
        (right - rx, top + ry, 3.0),
        (right - rx, bottom - ry, 0.0),
        (left + rx, bottom - ry, 1.0),
    ];
    let mut points: Vec<Point> = corners
        .into_iter()
        .flat_map(|(cx, cy, quarter)| {
            let from = quarter * FRAC_PI_2;
            arc_points(cx, cy, rx, ry, from, from + FRAC_PI_2, scale)
        })
        .collect();
    // Corners that meet, when a radius is half the size, share a point.
    points.dedup();
    if points.first() == points.last() {
        points.pop();
    }
    points
}

/// The unit vector perpendicular to the direction from `from` to `to`.
fn normal(from: Point, to: Point) -> Point {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1e-6 {
        (0.0, 0.0)
    } else {
        (dy / length, -dx / length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(points: &[Point]) -> (f32, f32, f32, f32) {
        points.iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        )
    }

    /// Total area of a triangle list.
    fn area(triangles: &[Point]) -> f32 {
        triangles
            .chunks(3)
            .map(|t| {
                ((t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[2].0 - t[0].0) * (t[1].1 - t[0].1))
                    .abs()
                    / 2.0
            })
            .sum()
    }

    const SQUARE: [Point; 4] = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];

    #[test]
    fn filling_a_square_covers_its_area_with_two_triangles() {
        let triangles = fill_convex(&SQUARE);
        assert_eq!(triangles.len(), 6);
        assert!((area(&triangles) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn a_shape_spreads_texture_coordinates_across_the_box_around_it() {
        let uv_of = spread_uv(&[(10.0, 20.0), (30.0, 20.0), (30.0, 60.0)]);
        assert_eq!(uv_of(10.0, 20.0), [0.0, 0.0]);
        assert_eq!(uv_of(30.0, 60.0), [1.0, 1.0]);
        assert_eq!(uv_of(20.0, 30.0), [0.5, 0.25]);
        // A shape with no size does not divide by zero.
        let uv_of = spread_uv(&[(5.0, 5.0), (5.0, 5.0)]);
        assert_eq!(uv_of(5.0, 5.0), [0.0, 0.0]);
    }

    #[test]
    fn a_shadow_stretches_the_far_edges_away_from_the_light() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        // A light to the left, level with the middle: only the left edge
        // faces it, so the other three cast shadows, all to the right.
        let triangles = shadow((-20.0, 5.0), &square, 100.0);
        assert_eq!(triangles.len(), 18);
        assert!(triangles.iter().all(|&(x, _)| x >= 0.0));
        assert!(triangles.iter().any(|&(x, _)| x > 100.0));
        // The other way round gives the same shadow.
        let reversed: Vec<Point> = square.iter().rev().copied().collect();
        let mirrored = shadow((-20.0, 5.0), &reversed, 100.0);
        assert_eq!(mirrored.len(), 18);
        assert!(mirrored.iter().all(|&(x, _)| x >= 0.0));
        // A light off a corner leaves two edges facing away.
        assert_eq!(shadow((-20.0, -20.0), &square, 100.0).len(), 12);
        // A light inside the shape is hidden on every side.
        assert_eq!(shadow((5.0, 5.0), &square, 100.0).len(), 24);
        assert!(shadow((0.0, 0.0), &[(1.0, 1.0)], 100.0).is_empty());
    }

    #[test]
    fn filling_needs_at_least_three_points() {
        assert!(fill_convex(&SQUARE[..2]).is_empty());
    }

    fn rough(width: f32, join: LineJoin) -> Stroke {
        Stroke {
            width,
            join,
            fringe: 0.0,
        }
    }

    fn flat(triangles: &[Covered]) -> Vec<Point> {
        triangles.iter().map(|&(x, y, _)| (x, y)).collect()
    }

    #[test]
    fn a_square_outline_is_centered_on_the_edge_with_square_corners() {
        let triangles = flat(&stroke(&SQUARE, true, &rough(2.0, LineJoin::Miter)));
        // One pixel outside and one inside on every side.
        let (left, top, right, bottom) = bounds(&triangles);
        for (actual, expected) in [(left, -1.0), (top, -1.0), (right, 11.0), (bottom, 11.0)] {
            assert!(
                (actual - expected).abs() < 1e-4,
                "bounds were off: {actual}"
            );
        }
        // The frame between a 12x12 and an 8x8 square.
        assert!((area(&triangles) - (144.0 - 64.0)).abs() < 1e-3);
    }

    #[test]
    fn a_closed_outline_may_repeat_its_first_point_at_the_end() {
        let mut points = SQUARE.to_vec();
        points.push(SQUARE[0]);
        let triangles = flat(&stroke(&points, true, &rough(2.0, LineJoin::Miter)));
        assert!((area(&triangles) - 80.0).abs() < 1e-3);
    }

    #[test]
    fn an_open_outline_follows_the_path_without_closing_it() {
        // An L: right 10, then down 10, 2 wide.
        let path = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        let triangles = flat(&stroke(&path, false, &rough(2.0, LineJoin::Miter)));
        assert_eq!(bounds(&triangles), (0.0, -1.0, 11.0, 10.0));
        // Two 10 by 2 strips sharing a mitered corner.
        assert!((area(&triangles) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn a_bevel_cuts_the_corner_off_and_no_join_leaves_a_notch() {
        let path = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        // The miter's corner square loses half of itself to the cut.
        let bevel = flat(&stroke(&path, false, &rough(2.0, LineJoin::Bevel)));
        assert_eq!(bounds(&bevel), (0.0, -1.0, 11.0, 10.0));
        assert!((area(&bevel) - 39.5).abs() < 1e-3);
        // Two plain 10 by 2 rectangles, which overlap inside the corner.
        let none = flat(&stroke(&path, false, &rough(2.0, LineJoin::None)));
        assert_eq!(none.len(), 12);
        assert!((area(&none) - 40.0).abs() < 1e-3);
        assert!(!none.contains(&(11.0, -1.0)));
    }

    #[test]
    fn a_very_sharp_miter_is_cut_off_instead_of_growing_a_spike() {
        // Out 100 and almost straight back.
        let path = [(0.0, 0.0), (100.0, 0.0), (0.0, 2.0)];
        let triangles = flat(&stroke(&path, false, &rough(2.0, LineJoin::Miter)));
        let (_, _, right, _) = bounds(&triangles);
        assert!(right < 102.0, "the corner reached {right}");
    }

    #[test]
    fn an_open_outline_of_two_points_is_a_rectangle_around_the_line() {
        let path = [(0.0, 5.0), (10.0, 5.0)];
        let triangles = flat(&stroke(&path, false, &rough(4.0, LineJoin::Miter)));
        assert_eq!(triangles.len(), 6);
        assert_eq!(bounds(&triangles), (0.0, 3.0, 10.0, 7.0));
        assert!((area(&triangles) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn an_open_outline_skips_repeated_points() {
        let style = rough(2.0, LineJoin::Miter);
        assert!(stroke(&[(1.0, 1.0), (1.0, 1.0)], false, &style).is_empty());
        let triangles = flat(&stroke(
            &[(0.0, 0.0), (0.0, 0.0), (10.0, 0.0)],
            false,
            &style,
        ));
        assert!((area(&triangles) - 20.0).abs() < 1e-3);
    }

    #[test]
    fn a_smooth_line_fades_out_half_a_pixel_either_side_of_its_edge() {
        let style = Stroke {
            width: 4.0,
            join: LineJoin::Miter,
            fringe: 1.0,
        };
        let triangles = stroke(&[(0.0, 5.0), (10.0, 5.0)], false, &style);
        // The solid part is half a pixel narrower on each side.
        let solid: Vec<Point> = triangles
            .iter()
            .filter(|corner| corner.2 == 1.0)
            .map(|&(x, y, _)| (x, y))
            .collect();
        assert_eq!(bounds(&solid), (0.0, 3.5, 10.0, 6.5));
        // The faded part reaches half a pixel beyond, and past the ends.
        assert_eq!(bounds(&flat(&triangles)), (-1.0, 2.5, 11.0, 7.5));
        assert!(
            triangles
                .iter()
                .all(|corner| corner.2 == 1.0 || corner.2 == 0.0)
        );
    }

    #[test]
    fn a_path_is_cut_into_dashes_measured_along_it() {
        fn same(pieces: &[Vec<Point>], expected: &[&[Point]]) -> bool {
            pieces.len() == expected.len()
                && pieces.iter().zip(expected).all(|(piece, wanted)| {
                    piece.len() == wanted.len()
                        && piece
                            .iter()
                            .zip(*wanted)
                            .all(|(a, b)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4)
                })
        }
        // 25 long: three dashes of 5 with gaps of 5 between them.
        let pieces = dashes(&[(0.0, 0.0), (25.0, 0.0)], 5.0, 5.0);
        assert!(same(
            &pieces,
            &[
                &[(0.0, 0.0), (5.0, 0.0)],
                &[(10.0, 0.0), (15.0, 0.0)],
                &[(20.0, 0.0), (25.0, 0.0)],
            ]
        ));
        // A dash carries on round a corner.
        let pieces = dashes(&[(0.0, 0.0), (8.0, 0.0), (8.0, 20.0)], 10.0, 4.0);
        assert!(same(
            &pieces,
            &[
                &[(0.0, 0.0), (8.0, 0.0), (8.0, 2.0)],
                &[(8.0, 6.0), (8.0, 16.0)],
            ]
        ));
        // A path that ends inside a dash keeps the part there is.
        let pieces = dashes(&[(0.0, 0.0), (12.0, 0.0)], 5.0, 5.0);
        assert!(same(
            &pieces,
            &[&[(0.0, 0.0), (5.0, 0.0)], &[(10.0, 0.0), (12.0, 0.0)]]
        ));
        // No gap still gives separate dashes that follow each other.
        assert_eq!(dashes(&[(0.0, 0.0), (12.0, 0.0)], 5.0, 0.0).len(), 3);
        // Nonsense lengths draw nothing instead of looping forever.
        assert!(dashes(&[(0.0, 0.0), (25.0, 0.0)], 0.0, 5.0).is_empty());
        assert!(dashes(&[(0.0, 0.0), (25.0, 0.0)], 5.0, -1.0).is_empty());
    }

    #[test]
    fn a_smooth_line_thinner_than_a_pixel_is_fainter() {
        let style = Stroke {
            width: 0.5,
            join: LineJoin::Miter,
            fringe: 1.0,
        };
        let triangles = stroke(&[(0.0, 5.0), (10.0, 5.0)], false, &style);
        let peak = triangles
            .iter()
            .fold(0.0f32, |peak, corner| peak.max(corner.2));
        assert_eq!(peak, 0.5);
    }

    #[test]
    fn ellipse_points_reach_both_radii() {
        let points = ellipse_points(0.0, 0.0, 40.0, 10.0, 1.0);
        let (left, top, right, bottom) = bounds(&points);
        assert!((left + 40.0).abs() < 0.5 && (right - 40.0).abs() < 1e-3);
        assert!((top + 10.0).abs() < 0.5 && (bottom - 10.0).abs() < 0.5);
    }

    #[test]
    fn an_arc_starts_and_ends_at_its_angles() {
        use std::f32::consts::FRAC_PI_2;
        let points = arc_points(0.0, 0.0, 10.0, 10.0, 0.0, FRAC_PI_2, 1.0);
        let (first, last) = (points[0], points[points.len() - 1]);
        assert!((first.0 - 10.0).abs() < 1e-4 && first.1.abs() < 1e-4);
        // A quarter turn clockwise on screen ends straight down.
        assert!(last.0.abs() < 1e-4 && (last.1 - 10.0).abs() < 1e-4);
        assert!(points.len() >= 4);
    }

    #[test]
    fn an_arc_never_sweeps_more_than_a_full_turn() {
        let once = arc_points(0.0, 0.0, 10.0, 10.0, 0.0, std::f32::consts::TAU, 1.0);
        let thrice = arc_points(0.0, 0.0, 10.0, 10.0, 0.0, 3.0 * std::f32::consts::TAU, 1.0);
        assert_eq!(once.len(), thrice.len());
    }

    #[test]
    fn a_rounded_rectangle_stays_inside_its_box_and_loses_its_corners() {
        let points = rounded_rectangle_points(10.0, 20.0, 100.0, 50.0, 8.0, 8.0, 1.0);
        let (left, top, right, bottom) = bounds(&points);
        for (actual, expected) in [(left, 10.0), (top, 20.0), (right, 110.0), (bottom, 70.0)] {
            assert!((actual - expected).abs() < 1e-3, "{actual} vs {expected}");
        }
        let filled = area(&fill_convex(&points));
        assert!(filled < 5000.0 && filled > 5000.0 - 64.0);
        // No radius gives the plain rectangle.
        assert_eq!(
            rounded_rectangle_points(0.0, 0.0, 4.0, 2.0, 0.0, 0.0, 1.0).len(),
            4
        );
    }

    #[test]
    fn circle_points_lie_on_the_circle() {
        let points = circle_points(50.0, 20.0, 30.0, 1.0);
        assert!(points.len() >= 12);
        for (x, y) in points {
            let distance = ((x - 50.0).powi(2) + (y - 20.0).powi(2)).sqrt();
            assert!((distance - 30.0).abs() < 1e-3);
        }
    }

    #[test]
    fn bigger_circles_get_more_points() {
        assert!(
            circle_points(0.0, 0.0, 200.0, 1.0).len() > circle_points(0.0, 0.0, 5.0, 1.0).len()
        );
        assert!(
            circle_points(0.0, 0.0, 50.0, 2.0).len() > circle_points(0.0, 0.0, 50.0, 1.0).len()
        );
    }
}

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

/// Triangles outlining a closed polygon with a line `width` wide, centered
/// on the polygon's edge, with mitered corners.
pub(crate) fn outline_closed(points: &[Point], width: f32) -> Vec<Point> {
    let count = points.len();
    if count < 3 {
        return Vec::new();
    }
    let half = width / 2.0;

    // For each corner, a point pushed outward and one pushed inward.
    let mut rims = Vec::with_capacity(count);
    for i in 0..count {
        let previous = points[(i + count - 1) % count];
        let current = points[i];
        let next = points[(i + 1) % count];

        let normal_in = normal(previous, current);
        let normal_out = normal(current, next);
        let (mut mx, mut my) = (normal_in.0 + normal_out.0, normal_in.1 + normal_out.1);
        let length = (mx * mx + my * my).sqrt();
        if length < 1e-6 {
            // The path doubles back on itself; any side will do.
            (mx, my) = normal_in;
        } else {
            (mx, my) = (mx / length, my / length);
        }
        // The miter is longer than the half width by 1/cos of half the turn.
        let cos = (mx * normal_in.0 + my * normal_in.1).max(1.0 / MITER_LIMIT);
        let reach = half / cos;

        rims.push((
            (current.0 + mx * reach, current.1 + my * reach),
            (current.0 - mx * reach, current.1 - my * reach),
        ));
    }

    let mut triangles = Vec::with_capacity(count * 6);
    for i in 0..count {
        let (a_out, a_in) = rims[i];
        let (b_out, b_in) = rims[(i + 1) % count];
        triangles.extend_from_slice(&[a_out, b_out, b_in, a_out, b_in, a_in]);
    }
    triangles
}

/// Triangles for a line through `points` in order, `width` wide, with
/// mitered corners and square ends that stop at the first and last point.
pub(crate) fn outline_open(points: &[Point], width: f32) -> Vec<Point> {
    // Repeated points have no direction and would break the corners.
    let mut path: Vec<Point> = Vec::with_capacity(points.len());
    for &point in points {
        if path.last() != Some(&point) {
            path.push(point);
        }
    }
    let count = path.len();
    if count < 2 {
        return Vec::new();
    }
    let half = width / 2.0;

    let mut rims = Vec::with_capacity(count);
    for i in 0..count {
        let current = path[i];
        let normal_in = normal(path[i.saturating_sub(1)], path[i.max(1)]);
        let normal_out = normal(path[i.min(count - 2)], path[(i + 1).min(count - 1)]);
        let (mut mx, mut my) = (normal_in.0 + normal_out.0, normal_in.1 + normal_out.1);
        let length = (mx * mx + my * my).sqrt();
        if length < 1e-6 {
            (mx, my) = normal_in;
        } else {
            (mx, my) = (mx / length, my / length);
        }
        let cos = (mx * normal_in.0 + my * normal_in.1).max(1.0 / MITER_LIMIT);
        let reach = half / cos;
        rims.push((
            (current.0 + mx * reach, current.1 + my * reach),
            (current.0 - mx * reach, current.1 - my * reach),
        ));
    }

    let mut triangles = Vec::with_capacity((count - 1) * 6);
    for pair in rims.windows(2) {
        let ((a_out, a_in), (b_out, b_in)) = (pair[0], pair[1]);
        triangles.extend_from_slice(&[a_out, b_out, b_in, a_out, b_in, a_in]);
    }
    triangles
}

/// Triangles for a straight line from `from` to `to`, `width` wide.
pub(crate) fn segment(from: Point, to: Point, width: f32) -> Vec<Point> {
    if from == to {
        return Vec::new();
    }
    let (nx, ny) = normal(from, to);
    let (ox, oy) = (nx * width / 2.0, ny * width / 2.0);
    let a = (from.0 + ox, from.1 + oy);
    let b = (to.0 + ox, to.1 + oy);
    let c = (to.0 - ox, to.1 - oy);
    let d = (from.0 - ox, from.1 - oy);
    vec![a, b, c, a, c, d]
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
    fn filling_needs_at_least_three_points() {
        assert!(fill_convex(&SQUARE[..2]).is_empty());
    }

    #[test]
    fn a_square_outline_is_centered_on_the_edge_with_square_corners() {
        let triangles = outline_closed(&SQUARE, 2.0);
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
    fn a_segment_is_a_rectangle_around_the_line() {
        let triangles = segment((0.0, 5.0), (10.0, 5.0), 4.0);
        assert_eq!(bounds(&triangles), (0.0, 3.0, 10.0, 7.0));
        assert!((area(&triangles) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn an_open_outline_follows_the_path_without_closing_it() {
        // An L: right 10, then down 10, 2 wide.
        let triangles = outline_open(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 2.0);
        assert_eq!(bounds(&triangles), (0.0, -1.0, 11.0, 10.0));
        // Two 10 by 2 strips sharing a mitered corner.
        assert!((area(&triangles) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn an_open_outline_of_two_points_is_a_segment() {
        let triangles = outline_open(&[(0.0, 5.0), (10.0, 5.0)], 4.0);
        assert_eq!(bounds(&triangles), (0.0, 3.0, 10.0, 7.0));
        assert!((area(&triangles) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn an_open_outline_skips_repeated_points() {
        assert!(outline_open(&[(1.0, 1.0), (1.0, 1.0)], 2.0).is_empty());
        let triangles = outline_open(&[(0.0, 0.0), (0.0, 0.0), (10.0, 0.0)], 2.0);
        assert!((area(&triangles) - 20.0).abs() < 1e-3);
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
    fn a_zero_length_segment_draws_nothing() {
        assert!(segment((1.0, 1.0), (1.0, 1.0), 4.0).is_empty());
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

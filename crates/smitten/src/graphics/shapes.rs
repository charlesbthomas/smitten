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
    let segments = (((radius * scale).sqrt() * 6.0).ceil() as usize).clamp(12, 256);
    (0..segments)
        .map(|i| {
            let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect()
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

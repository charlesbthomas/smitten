//! Drawing shapes: rectangles, circles, lines, polygons and the rest.

use super::*;

/// Draws a rectangle with its top-left corner at `(x, y)`.
pub fn rectangle(mode: DrawMode, position: impl Into<Vec2>, size: impl Into<Vec2>) {
    let Vec2 { x, y } = position.into();
    let Vec2 {
        x: width,
        y: height,
    } = size.into();
    polygon(
        mode,
        &[
            (x, y),
            (x + width, y),
            (x + width, y + height),
            (x, y + height),
        ],
    );
}

/// Draws a circle centered on `(x, y)`.
pub fn circle(mode: DrawMode, center: impl Into<Vec2>, radius: f32) {
    let Vec2 { x, y } = center.into();
    // More pixels per unit need more corners to look round.
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(mode, &shapes::circle_points(x, y, radius, scale));
}

/// Draws a polygon through `points`. Filling only works for convex polygons.
///
/// Under a custom shader, a filled shape's `uv` runs from `(0, 0)` at the
/// top-left of the box around the shape to `(1, 1)` at its bottom-right,
/// as it does across an image. So a shader can paint a rectangle, circle
/// or any other filled shape with no image at all. This holds for every
/// filled shape, not only polygons.
pub fn polygon(mode: DrawMode, points: &[impl Into<Vec2> + Copy]) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let triangles = match mode {
            DrawMode::Fill => shapes::fill_convex(points),
            DrawMode::Line => {
                let triangles = shapes::stroke(points, true, &ctx.gfx.stroke());
                return ctx.gfx.push_covered(&triangles);
            }
        };
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws a rectangle with rounded corners. `rx` and `ry` are the corner
/// radii along each axis.
pub fn rounded_rectangle(
    mode: DrawMode,
    position: impl Into<Vec2>,
    size: impl Into<Vec2>,
    radii: impl Into<Vec2>,
) {
    let Vec2 { x, y } = position.into();
    let Vec2 {
        x: width,
        y: height,
    } = size.into();
    let Vec2 { x: rx, y: ry } = radii.into();
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(
        mode,
        &shapes::rounded_rectangle_points(x, y, width, height, rx, ry, scale),
    );
}

/// Draws an ellipse centered on `(x, y)` with radii `rx` and `ry`.
pub fn ellipse(mode: DrawMode, center: impl Into<Vec2>, radii: impl Into<Vec2>) {
    let Vec2 { x, y } = center.into();
    let Vec2 { x: rx, y: ry } = radii.into();
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(mode, &shapes::ellipse_points(x, y, rx, ry, scale));
}

/// Draws part of a circle centered on `(x, y)`, from `angle1` to `angle2`
/// in radians. Angles grow clockwise from the right.
pub fn arc(
    mode: DrawMode,
    arc_type: ArcType,
    center: impl Into<Vec2>,
    radius: f32,
    angle1: f32,
    angle2: f32,
) {
    let Vec2 { x, y } = center.into();
    context::with(|ctx| {
        let scale = ctx.gfx.target_scale();
        let mut path = shapes::arc_points(x, y, radius, radius, angle1, angle2, scale);
        if arc_type == ArcType::Pie {
            // The center comes first so that filling fans out from it.
            path.insert(0, (x, y));
        }
        match mode {
            DrawMode::Fill => ctx.gfx.push_shape(&shapes::fill_convex(&path)),
            DrawMode::Line => {
                let closed = arc_type != ArcType::Open;
                let triangles = shapes::stroke(&path, closed, &ctx.gfx.stroke());
                ctx.gfx.push_covered(&triangles);
            }
        }
    });
}

/// Draws a square of the current point size centered on each point.
pub fn points(points: &[impl Into<Vec2> + Copy]) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let half = ctx.gfx.point_size / 2.0;
        let mut triangles = Vec::with_capacity(points.len() * 6);
        for &(x, y) in points {
            let (left, top, right, bottom) = (x - half, y - half, x + half, y + half);
            triangles.extend_from_slice(&[
                (left, top),
                (right, top),
                (right, bottom),
                (left, top),
                (right, bottom),
                (left, bottom),
            ]);
        }
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws the shadow that a convex shape casts away from a light at
/// `(light_x, light_y)`, in the current color: every edge of `polygon`
/// that faces away from the light, stretched `length` further from it.
///
/// The shape itself is not covered, as if light falls on its top. The
/// corners may be listed clockwise or counter-clockwise. A `length`
/// longer than the screen is wide gives shadows that run off its edge.
///
/// For lighting, draw the shadows of everything solid inside
/// [`stencil`], then draw the light's glow only where the stencil is
/// still zero. The `lighting` example does exactly this.
pub fn shadow(light: impl Into<Vec2>, polygon: &[impl Into<Vec2> + Copy], length: f32) {
    let polygon = crate::math::tuples(polygon);
    let polygon = &polygon[..];
    let Vec2 {
        x: light_x,
        y: light_y,
    } = light.into();
    context::with(|ctx| {
        let triangles = shapes::shadow((light_x, light_y), polygon, length);
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws connected lines through `points` in order, with joined corners.
pub fn polyline(points: &[impl Into<Vec2> + Copy]) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let triangles = shapes::stroke(points, false, &ctx.gfx.stroke());
        ctx.gfx.push_covered(&triangles);
    });
}

/// Draws a dashed line through `points` in order: dashes `dash` long
/// with gaps of `gap` between them, both measured along the line. The
/// dashes use the current line width, join and style.
pub fn dashed_polyline(points: &[impl Into<Vec2> + Copy], dash: f32, gap: f32) {
    let points = crate::math::tuples(points);
    let points = &points[..];
    context::with(|ctx| {
        let stroke = ctx.gfx.stroke();
        let mut triangles = Vec::new();
        for piece in shapes::dashes(points, dash, gap) {
            triangles.extend(shapes::stroke(&piece, false, &stroke));
        }
        ctx.gfx.push_covered(&triangles);
    });
}

/// Draws a straight dashed line between two points. See
/// [`dashed_polyline`].
pub fn dashed_line(from: impl Into<Vec2>, to: impl Into<Vec2>, dash: f32, gap: f32) {
    let Vec2 { x: x1, y: y1 } = from.into();
    let Vec2 { x: x2, y: y2 } = to.into();
    dashed_polyline(&[(x1, y1), (x2, y2)], dash, gap);
}

/// Draws a straight line between two points.
pub fn line(from: impl Into<Vec2>, to: impl Into<Vec2>) {
    let Vec2 { x: x1, y: y1 } = from.into();
    let Vec2 { x: x2, y: y2 } = to.into();
    context::with(|ctx| {
        let triangles = shapes::stroke(&[(x1, y1), (x2, y2)], false, &ctx.gfx.stroke());
        ctx.gfx.push_covered(&triangles);
    });
}

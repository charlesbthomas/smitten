//! Smooth joins between lines, the job of Box2D's ghost vertices.
//!
//! A body sliding along ground made of several lines can catch on the
//! join between two of them: near the end of a line the engine may push
//! the body back along the ground instead of up out of it, as if the next
//! line's end were a little wall. Box2D avoids this by knowing each line's
//! neighbours. The engine here has a one-sided fix for chains that would
//! let bodies fall through from behind, so instead each contact with a
//! line is checked against the line's neighbours as it is solved, and a
//! push that only makes sense at a lone corner is turned into a push
//! straight out of the line.
//!
//! The lines of a chain know their neighbours already. The ends of a
//! chain, and an edge, learn them from the ghost vertices a game sets
//! with [`Shape::set_previous_vertex`] and [`Shape::set_next_vertex`].
//!
//! [`Shape::set_previous_vertex`]: super::Shape::set_previous_vertex
//! [`Shape::set_next_vertex`]: super::Shape::set_next_vertex

use std::collections::HashMap;

use rapier2d::prelude::*;

/// The ghost vertices of an edge or chain, in the collider's own space.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Ghosts {
    pub previous: Option<Vector>,
    pub next: Option<Vector>,
}

/// One line of an edge or chain with the points beyond each of its ends,
/// all in world space.
struct Line {
    start: Vector,
    end: Vector,
    before: Option<Vector>,
    after: Option<Vector>,
}

/// The line of `collider` that a contact touched, if it is an edge or
/// chain with something beyond at least one end.
fn line_of(
    collider: &Collider,
    piece: u32,
    ghosts: &HashMap<ColliderHandle, Ghosts>,
    handle: ColliderHandle,
) -> Option<Line> {
    let ghost = ghosts.get(&handle).copied().unwrap_or_default();
    let shape = collider.shape();
    let (start, end, before, after) = match shape.as_segment() {
        Some(segment) => (segment.a, segment.b, ghost.previous, ghost.next),
        None => {
            let chain = shape.as_polyline()?;
            let indices = chain.indices();
            let vertices = chain.vertices();
            let piece = piece as usize;
            let [a, b] = *indices.get(piece)?;
            // Smitten builds a chain's lines in order, so a line's
            // neighbours sit either side of it, and a closed chain's last
            // line comes before its first.
            let count = indices.len();
            let before = indices[(piece + count - 1) % count];
            let after = indices[(piece + 1) % count];
            let before = (before[1] == a && before != [a, b])
                .then(|| vertices[before[0] as usize])
                .or(ghost.previous);
            let after = (after[0] == b && after != [a, b])
                .then(|| vertices[after[1] as usize])
                .or(ghost.next);
            (vertices[a as usize], vertices[b as usize], before, after)
        }
    };
    if before.is_none() && after.is_none() {
        return None;
    }
    let pose = collider.position();
    Some(Line {
        start: pose.transform_point(start),
        end: pose.transform_point(end),
        before: before.map(|point| pose.transform_point(point)),
        after: after.map(|point| pose.transform_point(point)),
    })
}

/// The unit normal of the line from `from` to `to`, on the side `side`
/// points to.
fn face_normal(from: Vector, to: Vector, side: Vector) -> Vector {
    let normal = (to - from).perp().normalize_or_zero();
    if normal.dot(side) < 0.0 {
        -normal
    } else {
        normal
    }
}

/// Whether `normal` lies between the unit vectors `a` and `b`, which are
/// less than half a turn apart.
fn between(normal: Vector, a: Vector, b: Vector) -> bool {
    let turn = a.perp_dot(b);
    a.perp_dot(normal) * turn >= 0.0 && normal.perp_dot(b) * turn >= 0.0 && normal.dot(a) > 0.0
}

/// How far from a line's own normal a contact's normal may be before it is
/// looked at, as the cosine of the angle.
const FACE_TOLERANCE: f32 = 0.9999;

/// The normal a contact with `line` should use, given the one the engine
/// found pointing from the line to the other body, where on the line it
/// touched, and where the other body's center is. `None` keeps the
/// engine's.
fn smoothed(line: &Line, normal: Vector, touched: Vector, center: Vector) -> Option<Vector> {
    // The body's side of the line is where its center is. The engine's
    // normal is no help with that when it runs along the line, which is
    // the very case to fix.
    let side = (line.end - line.start).perp();
    let side = match side.dot(center - line.start) {
        distance if distance.abs() > f32::EPSILON => side * distance.signum(),
        _ => normal,
    };
    let face = face_normal(line.start, line.end, side);
    if face == Vector::ZERO || normal.dot(face) >= FACE_TOLERANCE {
        return None;
    }
    // The engine pushes away from a corner. Which end is it?
    let along = line.end - line.start;
    let t = (touched - line.start).dot(along) / along.length_squared();
    let (corner, beyond) = if t < 0.5 {
        (line.start, line.before?)
    } else {
        (line.end, line.after?)
    };
    // Where the neighbour bends away from the body, the corner is real
    // and pushes between the two lines' normals are fair. Anywhere else,
    // including a straight join, the corner is hidden by the neighbour.
    let bends_away = (beyond - corner).dot(face) < 0.0;
    if bends_away {
        let neighbour = face_normal(corner, beyond, face);
        // The neighbour's normal on the body's side is the one leaning
        // away from this line.
        let neighbour = if neighbour.dot(along * if t < 0.5 { -1.0 } else { 1.0 }) < 0.0 {
            -neighbour
        } else {
            neighbour
        };
        if between(normal, face, neighbour) {
            return None;
        }
    }
    Some(face)
}

/// Smooths the contact in `context` if one side is an edge or chain.
pub(super) fn smooth(
    context: &mut ContactModificationContext,
    ghosts: &HashMap<ColliderHandle, Ghosts>,
) {
    let colliders = context.colliders;
    let context_bodies = context.bodies;
    let (handle1, handle2) = (context.collider1, context.collider2);
    let Some(manifold) = context.rigid_mut() else {
        return;
    };
    if manifold.solver_contacts.is_empty() {
        return;
    }
    let sides = [
        (handle1, manifold.manifold.subshape1, true),
        (handle2, manifold.manifold.subshape2, false),
    ];
    for (handle, piece, first) in sides {
        let Some(collider) = colliders.get(handle) else {
            continue;
        };
        let Some(line) = line_of(collider, piece, ghosts, handle) else {
            continue;
        };
        let other = if first { handle2 } else { handle1 };
        let Some(center) = colliders.get(other).map(|other| {
            other
                .parent()
                .and_then(|parent| context_bodies.get(parent))
                .map_or(other.position().translation, |body| body.center_of_mass())
        }) else {
            continue;
        };
        // The engine's normal points from the first collider to the second.
        let outward = if first {
            *manifold.normal
        } else {
            -*manifold.normal
        };
        let count = manifold.solver_contacts.len() as f32;
        let touched = manifold
            .solver_contacts
            .iter()
            .fold(Vector::ZERO, |sum, contact| {
                sum + if first {
                    contact.anchor1
                } else {
                    contact.anchor2
                }
            })
            / count;
        let Some(face) = smoothed(&line, outward, touched, center) else {
            continue;
        };
        let normal = if first { face } else { -face };
        *manifold.normal = normal;
        // Measure each contact's gap again along the new normal, so a
        // body that only grazed the corner is not pushed.
        for contact in manifold.solver_contacts.iter_mut() {
            contact.dist = (contact.anchor2 - contact.anchor1).dot(normal);
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(before: Option<Vector>, after: Option<Vector>) -> Line {
        Line {
            start: Vector::new(0.0, 0.0),
            end: Vector::new(10.0, 0.0),
            before,
            after,
        }
    }

    const UP: Vector = Vector::new(0.0, -1.0);

    #[test]
    fn a_push_back_at_a_straight_join_becomes_a_push_out() {
        let line = flat(None, Some(Vector::new(20.0, 0.0)));
        // A body coming from the left, caught on the far end.
        let caught = Vector::new(-1.0, 0.0);
        let above = Vector::new(5.0, -10.0);
        assert_eq!(
            smoothed(&line, caught, Vector::new(10.0, 0.0), above),
            Some(UP)
        );
        // From below, the line's normal on that side is used.
        let below = Vector::new(5.0, 10.0);
        assert_eq!(
            smoothed(&line, caught, Vector::new(10.0, 0.0), below),
            Some(Vector::new(0.0, 1.0))
        );
    }

    #[test]
    fn a_push_straight_out_is_left_alone() {
        let line = flat(Some(Vector::new(-10.0, 0.0)), Some(Vector::new(20.0, 0.0)));
        assert_eq!(
            smoothed(&line, UP, Vector::new(10.0, 0.0), Vector::new(5.0, -10.0)),
            None
        );
    }

    #[test]
    fn a_lone_corner_still_pushes_sideways() {
        let line = flat(Some(Vector::new(-10.0, 0.0)), None);
        assert_eq!(
            smoothed(
                &line,
                Vector::new(1.0, 0.0),
                Vector::new(10.0, 0.0),
                Vector::new(15.0, -1.0)
            ),
            None
        );
    }

    #[test]
    fn a_real_corner_pushes_only_between_its_lines() {
        // The next line drops away, as at the top of a slope down.
        let line = flat(None, Some(Vector::new(20.0, 10.0)));
        let corner = Vector::new(10.0, 0.0);
        let diagonal = Vector::new(1.0, -1.0).normalize();
        let center = Vector::new(14.0, -4.0);
        assert_eq!(smoothed(&line, diagonal, corner, center), None);
        // Backwards is never fair.
        let back = Vector::new(-1.0, -0.2).normalize();
        assert_eq!(smoothed(&line, back, corner, center), Some(UP));
    }
}

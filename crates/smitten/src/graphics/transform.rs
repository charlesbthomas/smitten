use glam::Vec2;
/// A 2D affine transformation: any mix of moving, rotating, scaling and
/// shearing, the equivalent of LÖVE's `Transform`.
///
/// Maps a point `(x, y)` to `(a*x + c*y + tx, b*x + d*y + ty)`.
///
/// Make one with [`Transform::from_transformation`] or
/// [`Transform::new`], change it with the same methods as the graphics
/// transform stack, and use it with
/// [`graphics::apply_transform`](crate::graphics::apply_transform) or
/// [`graphics::draw_with`](crate::graphics::draw_with). Multiplying two
/// transforms, `outer * inner`, applies `inner` first.
///
/// ```
/// use smitten::Vec2;
/// use smitten::graphics::Transform;
///
/// let mut transform = Transform::from_transformation(100.0, 50.0, 0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 0.0);
/// assert_eq!(transform.transform_point((1.0, 1.0)), Vec2::new(102.0, 52.0));
/// assert_eq!(transform.inverse_transform_point((102.0, 52.0)), Vec2::new(1.0, 1.0));
/// transform.translate((10.0, 0.0));
/// assert_eq!(transform.transform_point(Vec2::ZERO), Vec2::new(120.0, 50.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The transformation that leaves every point where it is.
    pub const IDENTITY: Transform = Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// The identity: a transformation that changes nothing.
    pub fn new() -> Self {
        Self::IDENTITY
    }

    /// Builds the transformation LÖVE's `newTransform` arguments describe:
    /// move the origin by `(-ox, -oy)`, shear by `(kx, ky)`, scale by
    /// `(sx, sy)`, rotate by `angle` radians, then place it at `(x, y)`.
    #[allow(clippy::too_many_arguments)] // Mirrors love.math.newTransform.
    pub fn from_transformation(
        x: f32,
        y: f32,
        angle: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
        kx: f32,
        ky: f32,
    ) -> Self {
        let mut transform = Self::IDENTITY;
        transform.set_transformation(x, y, angle, sx, sy, ox, oy, kx, ky);
        transform
    }

    /// Replaces this transformation with the one
    /// [`from_transformation`](Self::from_transformation) builds.
    #[allow(clippy::too_many_arguments)] // Mirrors transform:setTransformation.
    pub fn set_transformation(
        &mut self,
        x: f32,
        y: f32,
        angle: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
        kx: f32,
        ky: f32,
    ) {
        *self = Self::IDENTITY;
        self.translate((x, y));
        self.rotate(angle);
        self.scale(sx, sy);
        self.shear((kx, ky));
        self.translate((-ox, -oy));
    }

    /// Puts this back to the identity.
    pub fn reset(&mut self) {
        *self = Self::IDENTITY;
    }

    /// Builds the transformation LÖVE's `draw` arguments describe: scale and
    /// rotate around the origin offset `(ox, oy)`, then place it at `(x, y)`.
    pub fn from_draw_args(x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) -> Self {
        let mut transform = Self::IDENTITY;
        transform.translate((x, y));
        transform.rotate(r);
        transform.scale(sx, sy);
        transform.translate((-ox, -oy));
        transform
    }

    /// Moves the coordinate system by `(dx, dy)`.
    ///
    /// Like the other methods here, this affects what is drawn afterwards
    /// and is itself affected by the rotation and scale applied before it.
    pub fn translate(&mut self, delta: impl Into<Vec2>) {
        let Vec2 { x: dx, y: dy } = delta.into();
        self.tx += self.a * dx + self.c * dy;
        self.ty += self.b * dx + self.d * dy;
    }

    /// Rotates the coordinate system by `angle` radians, clockwise on screen.
    pub fn rotate(&mut self, angle: f32) {
        let (sin, cos) = angle.sin_cos();
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        self.a = a * cos + c * sin;
        self.b = b * cos + d * sin;
        self.c = c * cos - a * sin;
        self.d = d * cos - b * sin;
    }

    /// Scales the coordinate system by `sx` horizontally and `sy` vertically.
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.a *= sx;
        self.b *= sx;
        self.c *= sy;
        self.d *= sy;
    }

    /// Shears the coordinate system: x gains `kx` times y, and y gains `ky`
    /// times x.
    pub fn shear(&mut self, shear: impl Into<Vec2>) {
        let Vec2 { x: kx, y: ky } = shear.into();
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        self.a = a + c * ky;
        self.b = b + d * ky;
        self.c = a * kx + c;
        self.d = b * kx + d;
    }

    /// Applies `other` after everything this transformation already does
    /// to what is drawn, as if its calls had been made on this one. The
    /// equivalent of LÖVE's `transform:apply(other)`.
    pub fn apply_transform(&mut self, other: &Transform) {
        *self = self.then_inner(other);
    }

    /// The transformation that undoes this one, or `None` if it squashes
    /// everything onto a line or a point, as a scale of zero does.
    pub fn inverse(&self) -> Option<Transform> {
        let determinant = self.a * self.d - self.b * self.c;
        if determinant == 0.0 || !determinant.is_finite() {
            return None;
        }
        let (a, b, c, d) = (
            self.d / determinant,
            -self.b / determinant,
            -self.c / determinant,
            self.a / determinant,
        );
        Some(Transform {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        })
    }

    /// Transforms the point `(x, y)`. The same as [`apply`](Self::apply),
    /// under LÖVE's name.
    pub fn transform_point(&self, position: impl Into<Vec2>) -> Vec2 {
        let Vec2 { x, y } = position.into();
        self.apply((x, y))
    }

    /// Finds the point that this transformation moves to `(x, y)`, such as
    /// where in the game's world the mouse is. Without an
    /// [`inverse`](Self::inverse) the result is not finite.
    pub fn inverse_transform_point(&self, position: impl Into<Vec2>) -> Vec2 {
        let Vec2 { x, y } = position.into();
        match self.inverse() {
            Some(inverse) => inverse.apply((x, y)),
            None => (f32::NAN, f32::NAN).into(),
        }
    }

    /// The transformation as a 3 by 3 matrix in rows, as LÖVE's
    /// `getMatrix` gives it without the third dimension. The last row is
    /// always `[0, 0, 1]`.
    pub fn matrix(&self) -> [[f32; 3]; 3] {
        [
            [self.a, self.c, self.tx],
            [self.b, self.d, self.ty],
            [0.0, 0.0, 1.0],
        ]
    }

    /// Replaces the transformation with a 3 by 3 matrix in rows, laid out
    /// as [`matrix`](Self::matrix) gives it. The last row is ignored, since
    /// a 2D affine transformation cannot use it.
    pub fn set_matrix(&mut self, matrix: [[f32; 3]; 3]) {
        let [[a, c, tx], [b, d, ty], _] = matrix;
        *self = Transform { a, b, c, d, tx, ty };
    }

    /// The transformation as a 4 by 4 matrix in column-major order, for
    /// the GPU, leaving z as it is.
    pub(crate) fn gpu_matrix(&self) -> [f32; 16] {
        [
            self.a, self.b, 0.0, 0.0, //
            self.c, self.d, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            self.tx, self.ty, 0.0, 1.0,
        ]
    }

    /// Returns the transformation that applies `inner` first and then `self`.
    pub fn then_inner(&self, inner: &Transform) -> Transform {
        Transform {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }

    /// Whether this is a 2D affine transformation, like LÖVE's
    /// `isAffine2DTransform`. Always `true`: a smitten `Transform` has no
    /// third dimension, so it cannot be anything else. It is here so code
    /// ported from LÖVE can keep its checks.
    pub fn is_affine_2d(&self) -> bool {
        true
    }

    /// Transforms the point `(x, y)`.
    pub fn apply(&self, position: impl Into<Vec2>) -> Vec2 {
        let Vec2 { x, y } = position.into();
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
            .into()
    }
}

impl std::ops::Mul for Transform {
    type Output = Transform;

    /// `outer * inner` applies `inner` first and then `outer`.
    fn mul(self, inner: Transform) -> Transform {
        self.then_inner(&inner)
    }
}

impl std::ops::MulAssign for Transform {
    /// The same as [`Transform::apply_transform`].
    fn mul_assign(&mut self, inner: Transform) {
        self.apply_transform(&inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn assert_close(actual: impl Into<Vec2>, expected: impl Into<Vec2>) {
        let (actual, expected) = (actual.into(), expected.into());
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn identity_leaves_points_alone() {
        assert_close(Transform::IDENTITY.apply((3.0, -4.0)), (3.0, -4.0));
    }

    #[test]
    fn later_calls_happen_in_the_earlier_coordinate_system() {
        // As in LÖVE: after scaling by 2, a translation of 10 moves 20 pixels.
        let mut transform = Transform::IDENTITY;
        transform.scale(2.0, 2.0);
        transform.translate((10.0, 0.0));
        assert_close(transform.apply((1.0, 1.0)), (22.0, 2.0));
    }

    #[test]
    fn positive_rotation_turns_x_toward_y() {
        // With y pointing down the screen, this is clockwise.
        let mut transform = Transform::IDENTITY;
        transform.rotate(FRAC_PI_2);
        assert_close(transform.apply((1.0, 0.0)), (0.0, 1.0));
    }

    #[test]
    fn draw_args_rotate_around_the_origin_offset() {
        // A 10x10 sprite centered on (100, 100) and turned a quarter: its
        // center stays put and its top-left corner swings to the top-right.
        let transform = Transform::from_draw_args(100.0, 100.0, FRAC_PI_2, 1.0, 1.0, 5.0, 5.0);
        assert_close(transform.apply((5.0, 5.0)), (100.0, 100.0));
        assert_close(transform.apply((0.0, 0.0)), (105.0, 95.0));
    }

    #[test]
    fn then_inner_matches_applying_one_after_the_other() {
        let mut outer = Transform::IDENTITY;
        outer.translate((7.0, -2.0));
        outer.rotate(0.6);
        let mut inner = Transform::IDENTITY;
        inner.scale(3.0, 0.5);
        inner.translate((1.0, 4.0));

        let Vec2 { x: ix, y: iy } = inner.apply((2.0, 9.0));
        assert_close(
            outer.then_inner(&inner).apply((2.0, 9.0)),
            outer.apply((ix, iy)),
        );
    }

    #[test]
    fn shear_moves_x_by_y() {
        let mut transform = Transform::IDENTITY;
        transform.shear((0.5, 0.0));
        assert_close(transform.apply((0.0, 2.0)), (1.0, 2.0));
        assert_close(transform.apply((2.0, 0.0)), (2.0, 0.0));
        let mut transform = Transform::IDENTITY;
        transform.shear((0.0, 0.25));
        assert_close(transform.apply((4.0, 0.0)), (4.0, 1.0));
    }

    #[test]
    fn full_transformation_matches_the_steps_in_loves_order() {
        let built = Transform::from_transformation(30.0, 40.0, 0.7, 2.0, 3.0, 5.0, 6.0, 0.2, 0.1);
        let mut by_hand = Transform::IDENTITY;
        by_hand.translate((30.0, 40.0));
        by_hand.rotate(0.7);
        by_hand.scale(2.0, 3.0);
        by_hand.shear((0.2, 0.1));
        by_hand.translate((-5.0, -6.0));
        assert_close(built.apply((1.5, -2.0)), by_hand.apply((1.5, -2.0)));
        // Without a shear it is what the draw arguments describe.
        let plain = Transform::from_transformation(30.0, 40.0, 0.7, 2.0, 3.0, 5.0, 6.0, 0.0, 0.0);
        let draw = Transform::from_draw_args(30.0, 40.0, 0.7, 2.0, 3.0, 5.0, 6.0);
        assert_close(plain.apply((9.0, 1.0)), draw.apply((9.0, 1.0)));
    }

    #[test]
    fn inverse_undoes_the_transformation() {
        let transform =
            Transform::from_transformation(30.0, 40.0, 0.7, 2.0, 3.0, 5.0, 6.0, 0.2, 0.1);
        let Vec2 { x, y } = transform.apply((7.0, -3.0));
        assert_close(transform.inverse_transform_point((x, y)), (7.0, -3.0));
        let round_trip = transform * transform.inverse().unwrap();
        assert_close(round_trip.apply((11.0, 12.0)), (11.0, 12.0));
    }

    #[test]
    fn a_flattening_transformation_has_no_inverse() {
        let mut transform = Transform::IDENTITY;
        transform.scale(0.0, 1.0);
        assert!(transform.inverse().is_none());
        assert!(transform.inverse_transform_point((1.0, 1.0)).x.is_nan());
    }

    #[test]
    fn apply_transform_and_multiplication_compose_like_then_inner() {
        let outer = Transform::from_draw_args(3.0, 4.0, 0.5, 2.0, 2.0, 0.0, 0.0);
        let inner = Transform::from_draw_args(-1.0, 8.0, -0.2, 1.0, 0.5, 2.0, 2.0);
        let mut applied = outer;
        applied.apply_transform(&inner);
        let mut assigned = outer;
        assigned *= inner;
        let expected = outer.then_inner(&inner).apply((5.0, 6.0));
        assert_close(applied.apply((5.0, 6.0)), expected);
        assert_close(assigned.apply((5.0, 6.0)), expected);
        assert_close((outer * inner).apply((5.0, 6.0)), expected);
    }

    #[test]
    fn matrix_round_trips_and_reset_restores_the_identity() {
        let transform = Transform::from_transformation(1.0, 2.0, 0.3, 4.0, 5.0, 0.0, 0.0, 0.1, 0.0);
        let matrix = transform.matrix();
        assert_eq!(matrix[2], [0.0, 0.0, 1.0]);
        assert_eq!(matrix[0][2], 1.0);
        let mut copy = Transform::IDENTITY;
        copy.set_matrix(matrix);
        assert_eq!(copy, transform);
        copy.reset();
        assert_eq!(copy, Transform::new());
    }
}

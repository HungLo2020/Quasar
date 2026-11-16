//! Column-major matrices, right-handed world, `[0,1]` clip depth.
//!
//! [`Mat4`] keeps its historical array layout and is interchangeable with
//! [`quasar_math::Mat4`] (glam) through `From` conversions; both follow the
//! conventions documented in `quasar-math`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);
impl Mat4 {
    pub const IDENTITY: Self = Self([
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]);
    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.0[3] = [x, y, z, 1.];
        m
    }
    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        Self([
            [x, 0., 0., 0.],
            [0., y, 0., 0.],
            [0., 0., z, 0.],
            [0., 0., 0., 1.],
        ])
    }
    /// Right-handed view matrix looking from `eye` toward `target`.
    pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Option<Self> {
        if !eye
            .iter()
            .chain(&target)
            .chain(&up)
            .all(|value| value.is_finite())
        {
            return None;
        }
        let forward = normalize(sub(target, eye))?;
        let right = normalize(cross(forward, up))?;
        let up = cross(right, forward);
        Some(Self([
            [right[0], up[0], -forward[0], 0.],
            [right[1], up[1], -forward[1], 0.],
            [right[2], up[2], -forward[2], 0.],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.],
        ]))
    }
    pub fn rotation_y(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self([
            [c, 0., -s, 0.],
            [0., 1., 0., 0.],
            [s, 0., c, 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn rotation_x(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self([
            [1., 0., 0., 0.],
            [0., c, s, 0.],
            [0., -s, c, 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Option<Self> {
        if ![fov_y, aspect, near, far].iter().all(|v| v.is_finite())
            || !(0.0..std::f32::consts::PI).contains(&fov_y)
            || fov_y == 0.
            || aspect <= 0.
            || near <= 0.
            || far <= near
        {
            return None;
        }
        let f = 1. / (fov_y / 2.).tan();
        let z = far / (near - far);
        Some(Self([
            [f / aspect, 0., 0., 0.],
            [0., f, 0., 0.],
            [0., 0., z, -1.],
            [0., 0., near * z, 0.],
        ]))
    }
    /// Right-handed orthographic projection with `[0,1]` clip depth.
    pub fn orthographic(
        left: f32,
        right: f32,
        bottom: f32,
        top: f32,
        near: f32,
        far: f32,
    ) -> Option<Self> {
        if ![left, right, bottom, top, near, far]
            .iter()
            .all(|x| x.is_finite())
            || right <= left
            || top <= bottom
            || near < 0.
            || far <= near
        {
            return None;
        }
        let projection = Self([
            [2. / (right - left), 0., 0., 0.],
            [0., 2. / (top - bottom), 0., 0.],
            [0., 0., 1. / (near - far), 0.],
            [
                -(right + left) / (right - left),
                -(top + bottom) / (top - bottom),
                near / (near - far),
                1.,
            ],
        ]);
        projection.finite().then_some(projection)
    }
    pub fn transform(self, v: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|r| (0..4).map(|c| self.0[c][r] * v[c]).sum())
    }
    pub fn finite(self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
    }
}
fn sub(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}
fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}
fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}
fn normalize(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(value, value).sqrt();
    (length > 1e-6).then(|| [value[0] / length, value[1] / length, value[2] / length])
}
impl From<quasar_math::Mat4> for Mat4 {
    fn from(matrix: quasar_math::Mat4) -> Self {
        Self(matrix.to_cols_array_2d())
    }
}
impl From<Mat4> for quasar_math::Mat4 {
    fn from(matrix: Mat4) -> Self {
        quasar_math::Mat4::from_cols_array_2d(&matrix.0)
    }
}
impl Mat4 {
    /// The same matrix as a `quasar-math`/glam matrix.
    pub fn to_glam(self) -> quasar_math::Mat4 {
        self.into()
    }
    pub fn from_glam(matrix: quasar_math::Mat4) -> Self {
        matrix.into()
    }
    /// Inverse, or `None` when singular or non-finite.
    pub fn inverse(self) -> Option<Self> {
        let glam = self.to_glam();
        if glam.determinant().abs() <= f32::EPSILON * 1e-3 || !self.finite() {
            return None;
        }
        let inverse = glam.inverse();
        inverse.is_finite().then(|| inverse.into())
    }
    /// Treats `self` as a view-projection: world point to screen pixels plus
    /// `[0,1]` depth. See [`quasar_math::project`].
    pub fn project(self, world: [f32; 3], viewport: quasar_math::Viewport) -> Option<[f32; 3]> {
        quasar_math::project(self.into(), world.into(), viewport).map(|v| v.to_array())
    }
    /// Screen pixels plus `[0,1]` depth back to a world point.
    pub fn unproject(self, screen: [f32; 3], viewport: quasar_math::Viewport) -> Option<[f32; 3]> {
        quasar_math::unproject(self.into(), screen.into(), viewport).map(|v| v.to_array())
    }
    /// World-space picking ray `(origin on the near plane, unit direction)`
    /// through a screen pixel.
    pub fn screen_ray(
        self,
        screen: [f32; 2],
        viewport: quasar_math::Viewport,
    ) -> Option<([f32; 3], [f32; 3])> {
        quasar_math::screen_ray(self.into(), screen.into(), viewport)
            .map(|(origin, direction)| (origin.to_array(), direction.to_array()))
    }
}
impl std::ops::Mul for Mat4 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|c| self.transform(rhs.0[c])))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orthographic_bounds_and_depth_match_shadow_clip_convention() {
        let p = Mat4::orthographic(-2., 6., -3., 1., 0.1, 20.).unwrap();
        for (v, expected) in [
            ([-2., -3., -0.1, 1.], [-1., -1., 0., 1.]),
            ([6., 1., -20., 1.], [1., 1., 1., 1.]),
        ] {
            for (a, b) in p.transform(v).into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-5);
            }
        }
        assert!(Mat4::orthographic(1., -1., -1., 1., 0.1, 10.).is_none());
        assert!(Mat4::orthographic(-1., 1., 1., 1., 0.1, 10.).is_none());
        assert!(Mat4::orthographic(-1., 1., -1., 1., -1., 10.).is_none());
        assert!(Mat4::orthographic(-1., 1., -1., 1., 10., 1.).is_none());
        assert!(Mat4::orthographic(f32::NAN, 1., -1., 1., 0.1, 10.).is_none());
        assert!(Mat4::orthographic(0., f32::MIN_POSITIVE / 100., -1., 1., 0.1, 10.).is_none());
    }
    #[test]
    fn perspective_maps_near_and_far_to_vulkan_depth() {
        let p = Mat4::perspective(1., 1.5, 0.1, 100.).unwrap();
        for (z, expected) in [(-0.1, 0.), (-100., 1.)] {
            let v = p.transform([0., 0., z, 1.]);
            assert!((v[2] / v[3] - expected).abs() < 1e-5);
        }
        assert!(Mat4::perspective(1., 0., 0.1, 10.).is_none());
    }
    #[test]
    fn scale_and_view_matrices_are_right_handed() {
        assert_eq!(
            Mat4::scale(2., 3., 4.).transform([1., 1., 1., 1.]),
            [2., 3., 4., 1.]
        );
        let view = Mat4::look_at([0., 0., 5.], [0., 0., 0.], [0., 1., 0.]).unwrap();
        let origin = view.transform([0., 0., 0., 1.]);
        assert!((origin[0]).abs() < 1e-6 && (origin[1]).abs() < 1e-6);
        assert!((origin[2] + 5.).abs() < 1e-6);
        assert!(Mat4::look_at([0.; 3], [0.; 3], [0., 1., 0.]).is_none());
    }

    #[test]
    fn constructors_match_quasar_math_conventions() {
        use quasar_math as qm;
        let close = |a: Mat4, b: qm::Mat4| {
            a.0.iter()
                .flatten()
                .zip(b.to_cols_array())
                .all(|(x, y)| (x - y).abs() < 1e-5)
        };
        let eye = qm::Vec3::new(1., 2., 5.);
        assert!(close(
            Mat4::look_at(eye.to_array(), [0.; 3], [0., 1., 0.]).unwrap(),
            qm::look_at(eye, qm::Vec3::ZERO, qm::Vec3::Y).unwrap()
        ));
        assert!(close(
            Mat4::perspective(1.1, 1.6, 0.1, 50.).unwrap(),
            qm::perspective(1.1, 1.6, 0.1, 50.).unwrap()
        ));
        assert!(close(
            Mat4::orthographic(-2., 6., -3., 1., 0.1, 20.).unwrap(),
            qm::orthographic(-2., 6., -3., 1., 0.1, 20.).unwrap()
        ));
        assert!(close(Mat4::rotation_y(0.7), qm::Mat4::from_rotation_y(0.7)));
        assert!(close(Mat4::rotation_x(0.7), qm::Mat4::from_rotation_x(0.7)));
        let m = Mat4::translation(1., 2., 3.) * Mat4::rotation_y(0.4);
        assert_eq!(Mat4::from_glam(m.to_glam()), m);
        assert!(close(m.inverse().unwrap() * m, qm::Mat4::IDENTITY));
        assert!(Mat4::scale(0., 1., 1.).inverse().is_none());
    }

    #[test]
    fn picking_helpers_round_trip_through_the_view_projection() {
        let viewport = quasar_math::Viewport::new(640., 480.);
        let vp = Mat4::perspective(1., viewport.aspect(), 0.1, 100.).unwrap()
            * Mat4::look_at([0., 3., 8.], [0., 0., 0.], [0., 1., 0.]).unwrap();
        let screen = vp.project([0., 0., 0.], viewport).unwrap();
        assert!((screen[0] - 320.).abs() < 1e-3 && (screen[1] - 240.).abs() < 1e-3);
        let back = vp.unproject(screen, viewport).unwrap();
        assert!(back.iter().all(|v| v.abs() < 1e-3));
        let (origin, direction) = vp.screen_ray([320., 240.], viewport).unwrap();
        // The centre ray passes through the target.
        let to_target = [-origin[0], -origin[1], -origin[2]];
        let t: f32 = (0..3).map(|i| to_target[i] * direction[i]).sum();
        assert!((0..3).all(|i| (origin[i] + direction[i] * t).abs() < 1e-2));
    }

    #[test]
    fn multiplication_applies_right_operand_first() {
        assert_eq!(
            (Mat4::translation(1., 2., 3.) * Mat4::IDENTITY).transform([2., 3., 4., 1.]),
            [3., 5., 7., 1.]
        );
    }
}

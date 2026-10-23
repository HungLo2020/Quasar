//! Column-major matrices, right-handed world, Vulkan [0,1] clip depth.
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
    pub fn transform(self, v: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|r| (0..4).map(|c| self.0[c][r] * v[c]).sum())
    }
    pub fn finite(self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
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
    fn perspective_maps_near_and_far_to_vulkan_depth() {
        let p = Mat4::perspective(1., 1.5, 0.1, 100.).unwrap();
        for (z, expected) in [(-0.1, 0.), (-100., 1.)] {
            let v = p.transform([0., 0., z, 1.]);
            assert!((v[2] / v[3] - expected).abs() < 1e-5);
        }
        assert!(Mat4::perspective(1., 0., 0.1, 10.).is_none());
    }
    #[test]
    fn multiplication_applies_right_operand_first() {
        assert_eq!(
            (Mat4::translation(1., 2., 3.) * Mat4::IDENTITY).transform([2., 3., 4., 1.]),
            [3., 5., 7., 1.]
        );
    }
}

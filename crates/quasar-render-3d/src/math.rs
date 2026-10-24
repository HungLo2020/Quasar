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
    fn multiplication_applies_right_operand_first() {
        assert_eq!(
            (Mat4::translation(1., 2., 3.) * Mat4::IDENTITY).transform([2., 3., 4., 1.]),
            [3., 5., 7., 1.]
        );
    }
}

//! Shared math types for Quasar crates and games.
//!
//! `quasar-math` re-exports [`glam`] so every crate and game uses the same
//! `Vec2`/`Vec3`/`Vec4`/`Quat`/`Mat4` types and interoperates with the wider
//! Rust ecosystem (Rapier/parry use the same glam release). It adds only what
//! glam leaves to the caller: one documented set of conventions and checked
//! helpers that follow them.
//!
//! # Conventions
//!
//! * **Right-handed, Y up.** +X is right, +Y is up, and the camera looks down
//!   **−Z**. A yaw of 0 faces −Z; positive yaw turns left (counter-clockwise
//!   seen from above), so `forward_from_yaw(FRAC_PI_2)` is −X.
//! * **Column-major, column vectors.** `Mat4 * Vec4` transforms a point;
//!   `a * b` applies `b` first. Arrays from [`Mat4::to_cols_array_2d`] are the
//!   layout WGSL `mat4x4<f32>` uniforms expect.
//! * **Clip depth `[0, 1]`, NDC +Y up** — wgpu's convention, which glam
//!   calls `camera::rh::proj::directx`: the near plane maps to 0 and the far
//!   plane to 1.
//! * **Screen space** is in physical pixels with the origin at the top-left
//!   and +Y down, matching windows and `quasar-render-2d`.
//! * Angles are radians.
//!
//! ```
//! use quasar_math::{Vec3, look_at, perspective, project, Viewport};
//!
//! let view = look_at(Vec3::new(0.0, 2.0, 5.0), Vec3::ZERO, Vec3::Y).unwrap();
//! let projection = perspective(60f32.to_radians(), 16.0 / 9.0, 0.1, 100.0).unwrap();
//! let viewport = Viewport::new(1280.0, 720.0);
//! let screen = project(projection * view, Vec3::ZERO, viewport).unwrap();
//! assert!((screen.x - 640.0).abs() < 1e-3 && (screen.y - 360.0).abs() < 1e-3);
//! ```
pub use glam;
pub use glam::{
    Affine2, Affine3A, BVec2, BVec3, BVec4, DMat3, DMat4, DQuat, DVec2, DVec3, DVec4, EulerRot,
    IVec2, IVec3, IVec4, Mat2, Mat3, Mat3A, Mat4, Quat, UVec2, UVec3, UVec4, Vec2, Vec3, Vec3A,
    Vec4,
};

use std::f32::consts::{PI, TAU};

/// World up axis (+Y).
pub const UP: Vec3 = Vec3::Y;
/// Camera/look forward at yaw 0 (−Z).
pub const FORWARD: Vec3 = Vec3::NEG_Z;
/// Right axis (+X).
pub const RIGHT: Vec3 = Vec3::X;

fn finite3(values: &[Vec3]) -> bool {
    values.iter().all(|value| value.is_finite())
}

/// Right-handed view matrix looking from `eye` toward `target`. `None` when
/// inputs are non-finite, `eye == target`, or `up` is parallel to the view
/// direction.
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Option<Mat4> {
    look_to(eye, target - eye, up)
}

/// Right-handed view matrix looking from `eye` along `direction`.
pub fn look_to(eye: Vec3, direction: Vec3, up: Vec3) -> Option<Mat4> {
    if !finite3(&[eye, direction, up]) {
        return None;
    }
    let forward = direction.try_normalize()?;
    forward.cross(up).try_normalize()?;
    Some(glam::camera::rh::view::look_to_mat4(eye, forward, up))
}

/// Right-handed perspective projection with `[0, 1]` clip depth. `fov_y` is
/// the full vertical field of view in radians.
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Option<Mat4> {
    let valid = [fov_y, aspect, near, far].iter().all(|v| v.is_finite())
        && fov_y > 0.0
        && fov_y < PI
        && aspect > 0.0
        && near > 0.0
        && far > near;
    valid.then(|| glam::camera::rh::proj::directx::perspective(fov_y, aspect, near, far))
}

/// Right-handed perspective with an infinite far plane and reversed depth
/// (near → 1, infinity → 0), for large outdoor scenes.
pub fn perspective_infinite_reverse(fov_y: f32, aspect: f32, near: f32) -> Option<Mat4> {
    let valid = [fov_y, aspect, near].iter().all(|v| v.is_finite())
        && fov_y > 0.0
        && fov_y < PI
        && aspect > 0.0
        && near > 0.0;
    valid
        .then(|| glam::camera::rh::proj::directx::perspective_infinite_reverse(fov_y, aspect, near))
}

/// Right-handed orthographic projection with `[0, 1]` clip depth.
pub fn orthographic(
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    near: f32,
    far: f32,
) -> Option<Mat4> {
    let valid = [left, right, bottom, top, near, far]
        .iter()
        .all(|v| v.is_finite())
        && right > left
        && top > bottom
        && near >= 0.0
        && far > near;
    if !valid {
        return None;
    }
    let matrix = glam::camera::rh::proj::directx::orthographic(left, right, bottom, top, near, far);
    matrix.is_finite().then_some(matrix)
}

/// Rectangle of the render target in physical pixels (origin top-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub const fn new(width: f32, height: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }
    pub const fn with_origin(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }
    pub fn aspect(self) -> f32 {
        self.width / self.height
    }
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
    /// NDC `[-1, 1]` (+Y up) to screen pixels (+Y down).
    pub fn ndc_to_screen(self, ndc: Vec2) -> Vec2 {
        Vec2::new(
            self.x + (ndc.x + 1.0) * 0.5 * self.width,
            self.y + (1.0 - ndc.y) * 0.5 * self.height,
        )
    }
    /// Screen pixels to NDC.
    pub fn screen_to_ndc(self, screen: Vec2) -> Vec2 {
        Vec2::new(
            (screen.x - self.x) / self.width * 2.0 - 1.0,
            1.0 - (screen.y - self.y) / self.height * 2.0,
        )
    }
    /// Whether a screen position lies inside the viewport.
    pub fn contains(self, screen: Vec2) -> bool {
        screen.x >= self.x
            && screen.y >= self.y
            && screen.x <= self.x + self.width
            && screen.y <= self.y + self.height
    }
}

/// World point to screen: `xy` in pixels, `z` the `[0, 1]` depth. `None` for
/// points at or behind the camera plane (`w <= 0`) or invalid input. Points
/// outside the viewport still project, so off-screen markers can clamp them.
pub fn project(view_projection: Mat4, world: Vec3, viewport: Viewport) -> Option<Vec3> {
    if !viewport.valid() || !world.is_finite() {
        return None;
    }
    let clip = view_projection * world.extend(1.0);
    // Also rejects NaN.
    if clip.w.is_nan() || clip.w <= f32::EPSILON {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let screen = viewport.ndc_to_screen(ndc.truncate());
    let result = screen.extend(ndc.z);
    result.is_finite().then_some(result)
}

/// Screen pixels plus `[0, 1]` depth back to a world point.
pub fn unproject(view_projection: Mat4, screen: Vec3, viewport: Viewport) -> Option<Vec3> {
    if !viewport.valid() || !screen.is_finite() {
        return None;
    }
    let inverse = view_projection.inverse();
    if !inverse.is_finite() {
        return None;
    }
    let ndc = viewport.screen_to_ndc(screen.truncate()).extend(screen.z);
    let world = inverse * ndc.extend(1.0);
    if world.w.abs() <= f32::EPSILON {
        return None;
    }
    let point = world.truncate() / world.w;
    point.is_finite().then_some(point)
}

/// World-space picking ray through a screen pixel: `(origin, unit
/// direction)`. The origin lies on the near plane, so it works for both
/// perspective and orthographic projections.
pub fn screen_ray(view_projection: Mat4, screen: Vec2, viewport: Viewport) -> Option<(Vec3, Vec3)> {
    let near = unproject(view_projection, screen.extend(0.0), viewport)?;
    // Depth 0.5 avoids precision loss at the far plane and works with
    // infinite projections.
    let far = unproject(view_projection, screen.extend(0.5), viewport)?;
    let direction = (far - near).try_normalize()?;
    Some((near, direction))
}

/// Wraps an angle to `(-π, π]`.
pub fn wrap_angle(angle: f32) -> f32 {
    if !angle.is_finite() {
        return angle;
    }
    let wrapped = (angle + PI).rem_euclid(TAU) - PI;
    if wrapped <= -PI {
        wrapped + TAU
    } else {
        wrapped
    }
}

/// Signed shortest rotation from `from` to `to`, in `(-π, π]`.
pub fn angle_delta(from: f32, to: f32) -> f32 {
    wrap_angle(to - from)
}

/// Interpolates angles along the shortest arc; `t` is not clamped.
pub fn lerp_angle(from: f32, to: f32, t: f32) -> f32 {
    wrap_angle(from + angle_delta(from, to) * t)
}

/// Rotates `current` toward `target` by at most `max_step` radians (the
/// usual "turn at N radians per second" helper). Returns a wrapped angle.
pub fn turn_towards(current: f32, target: f32, max_step: f32) -> f32 {
    let delta = angle_delta(current, target);
    let step = max_step.abs();
    if delta.abs() <= step {
        wrap_angle(target)
    } else {
        wrap_angle(current + step.copysign(delta))
    }
}

/// Horizontal forward vector for a yaw (0 → −Z, π/2 → −X).
pub fn forward_from_yaw(yaw: f32) -> Vec3 {
    let (sin, cos) = yaw.sin_cos();
    Vec3::new(-sin, 0.0, -cos)
}

/// Forward vector for yaw and pitch (positive pitch looks up).
pub fn forward_from_yaw_pitch(yaw: f32, pitch: f32) -> Vec3 {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    Vec3::new(-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
}

/// Yaw of a direction's horizontal component; inverse of
/// [`forward_from_yaw`]. `None` for vertical or zero directions.
pub fn yaw_of(direction: Vec3) -> Option<f32> {
    let horizontal = Vec2::new(direction.x, direction.z);
    (horizontal.length_squared() > f32::EPSILON && horizontal.is_finite())
        .then(|| (-direction.x).atan2(-direction.z))
}

/// Rotation about +Y matching [`forward_from_yaw`]: it maps −Z to
/// `forward_from_yaw(yaw)`.
pub fn yaw_rotation(yaw: f32) -> Quat {
    Quat::from_rotation_y(yaw)
}

/// glTF's model forward axis. glTF assets face **+Z** (the spec's "front"),
/// while Quasar yaw 0 faces [`FORWARD`] (−Z).
pub const GLTF_FORWARD: Vec3 = Vec3::Z;

/// Rotation that makes a model authored facing `model_forward` face
/// `forward_from_yaw(yaw)`. Pass [`GLTF_FORWARD`] for glTF/GLB models so an
/// object steered with Quasar yaw does not end up turned by π:
///
/// ```
/// use quasar_math::{GLTF_FORWARD, Vec3, forward_from_yaw, yaw_rotation_for};
/// let yaw = 0.7;
/// let facing = yaw_rotation_for(GLTF_FORWARD, yaw) * GLTF_FORWARD;
/// assert!(facing.abs_diff_eq(forward_from_yaw(yaw), 1e-5));
/// ```
///
/// Only the horizontal part of `model_forward` matters; a vertical or zero
/// axis is treated as [`FORWARD`].
pub fn yaw_rotation_for(model_forward: Vec3, yaw: f32) -> Quat {
    let offset = yaw_of(model_forward).unwrap_or(0.0);
    Quat::from_rotation_y(wrap_angle(yaw - offset))
}

/// Translation · rotation · scale model matrix.
pub fn trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(scale, rotation, translation)
}

/// Frame-rate-independent exponential smoothing factor: use as
/// `value.lerp(target, damp_factor(rate, dt))`.
pub fn damp_factor(rate: f32, dt: f32) -> f32 {
    if !(rate.is_finite() && dt.is_finite()) || rate <= 0.0 || dt <= 0.0 {
        return if rate.is_infinite() && dt > 0.0 {
            1.0
        } else {
            0.0
        };
    }
    1.0 - (-rate * dt).exp()
}

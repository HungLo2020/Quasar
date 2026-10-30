//! Explicit collision geometry and render-neutral rigid/model transforms.
use crate::rapier3d::{
    geometry::SharedShape,
    math::{Pose, Rotation, Vector},
    prelude::ColliderBuilder,
};
use std::{error::Error, fmt};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeError3(pub &'static str);
impl fmt::Display for ShapeError3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for ShapeError3 {}
impl ShapeError3 {
    pub const fn code(self) -> &'static str {
        "physics.invalid_shape"
    }
}
type Result<T> = std::result::Result<T, ShapeError3>;
/// Rigid local/world pose. Quaternions are xyzw, unit length; no scale or shear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidPose3 {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}
impl RigidPose3 {
    pub const IDENTITY: Self = Self {
        translation: [0.; 3],
        rotation: [0., 0., 0., 1.],
    };
    pub fn translated(translation: [f32; 3]) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }
    pub fn yaw(translation: [f32; 3], radians: f32) -> Self {
        let (s, c) = (radians * 0.5).sin_cos();
        Self {
            translation,
            rotation: [0., s, 0., c],
        }
    }
    pub fn rapier(self) -> Result<Pose> {
        if self
            .translation
            .iter()
            .chain(&self.rotation)
            .any(|v| !v.is_finite())
            || (self.rotation.iter().map(|v| v * v).sum::<f32>() - 1.).abs() > 1e-4
        {
            return Err(ShapeError3(
                "pose needs finite translation and a unit xyzw quaternion",
            ));
        }
        Ok(Pose::from_parts(
            Vector::from_array(self.translation),
            Rotation::from_array(self.rotation),
        ))
    }
    /// Column-major matrix, shared with rendering adapters.
    pub fn matrix(self) -> Result<[[f32; 4]; 4]> {
        Ok(self.rapier()?.to_mat4().to_cols_array_2d())
    }
}
/// Dimensions are baked into primitives, in local units. Round shapes remain round.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape3 {
    Cuboid { half: [f32; 3] },
    CylinderY { half_height: f32, radius: f32 },
    CapsuleY { half_segment: f32, radius: f32 },
    Compound(Vec<PlacedShape3>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedShape3 {
    pub pose: RigidPose3,
    pub shape: Shape3,
}
impl Shape3 {
    pub fn shared(&self) -> Result<SharedShape> {
        let mut budget = 256;
        let mut parts = Vec::new();
        self.flatten(Pose::IDENTITY, 0, &mut budget, &mut parts)?;
        if matches!(self, Self::Compound(_)) {
            Ok(SharedShape::compound(parts))
        } else {
            Ok(parts.pop().unwrap().1)
        }
    }
    fn flatten(
        &self,
        pose: Pose,
        depth: usize,
        budget: &mut usize,
        out: &mut Vec<(Pose, SharedShape)>,
    ) -> Result<()> {
        if depth > 8 || *budget == 0 {
            return Err(ShapeError3("compound exceeds depth/256-shape budget"));
        }
        *budget -= 1;
        let positive = |v: f32| v.is_finite() && v > 0.;
        let shape = match self {
            Self::Cuboid { half } if half.iter().all(|v| positive(*v)) => {
                SharedShape::cuboid(half[0], half[1], half[2])
            }
            Self::CylinderY {
                half_height,
                radius,
            } if positive(*half_height) && positive(*radius) => {
                SharedShape::cylinder(*half_height, *radius)
            }
            Self::CapsuleY {
                half_segment,
                radius,
            } if half_segment.is_finite() && *half_segment >= 0. && positive(*radius) => {
                SharedShape::capsule_y(*half_segment, *radius)
            }
            Self::Compound(parts) if !parts.is_empty() => {
                for part in parts {
                    let combined = pose * part.pose.rapier()?;
                    if !combined.translation.is_finite() {
                        return Err(ShapeError3("compound transform overflow"));
                    }
                    part.shape.flatten(combined, depth + 1, budget, out)?;
                }
                return Ok(());
            }
            _ => return Err(ShapeError3("invalid or empty collision shape")),
        };
        out.push((pose, shape));
        Ok(())
    }
}
impl PlacedShape3 {
    pub fn collider(&self) -> Result<crate::rapier3d::geometry::Collider> {
        Ok(ColliderBuilder::new(self.shape.shared()?)
            .position(self.pose.rapier()?)
            .build())
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds3 {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveFit3 {
    Cuboid,
    CylinderY,
    CapsuleY,
}
impl Bounds3 {
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Result<Self> {
        if (0..3).any(|i| {
            !min[i].is_finite()
                || !max[i].is_finite()
                || max[i] <= min[i]
                || !(max[i] - min[i]).is_finite()
        }) {
            return Err(ShapeError3("bounds need finite, positive extents"));
        }
        Ok(Self { min, max })
    }
    pub fn center(self) -> [f32; 3] {
        std::array::from_fn(|i| self.min[i] * 0.5 + self.max[i] * 0.5)
    }
    pub fn half(self) -> [f32; 3] {
        std::array::from_fn(|i| (self.max[i] - self.min[i]) * 0.5)
    }
    /// Fits source bounds into an explicit local cuboid, then applies a rigid pose.
    /// This is a rendering transform; collider dimensions are authored separately.
    pub fn fit_matrix(self, pose: RigidPose3, target_half: [f32; 3]) -> Result<[[f32; 4]; 4]> {
        Self::new(self.min, self.max)?;
        if target_half.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return Err(ShapeError3("fit dimensions must be finite and positive"));
        }
        let scale = Vector::from_array(std::array::from_fn(|i| target_half[i] / self.half()[i]));
        let source = Vector::from_array(self.center());
        let mut matrix = pose.matrix()?;
        for (column, factor) in matrix[..3].iter_mut().zip(scale.to_array()) {
            for value in &mut column[..3] {
                *value *= factor;
            }
        }
        let offset: [f32; 3] =
            std::array::from_fn(|row| (0..3).map(|col| matrix[col][row] * source[col]).sum());
        for (value, offset) in matrix[3][..3].iter_mut().zip(offset) {
            *value -= offset;
        }
        if matrix.iter().flatten().any(|v| !v.is_finite()) {
            return Err(ShapeError3("fit transform overflow"));
        }
        Ok(matrix)
    }
    /// Explicit opt-in primitive approximation. A cylinder/capsule requires a
    /// square XZ footprint; asymmetric scaling must be authored as another shape.
    pub fn collider(self, fit: PrimitiveFit3) -> Result<PlacedShape3> {
        Self::new(self.min, self.max)?;
        let half = self.half();
        if fit != PrimitiveFit3::Cuboid && (half[0] - half[2]).abs() > 1e-4 * half[0].max(half[2]) {
            return Err(ShapeError3("round fit requires equal X/Z radii"));
        }
        let shape = match fit {
            PrimitiveFit3::Cuboid => Shape3::Cuboid { half },
            PrimitiveFit3::CylinderY => Shape3::CylinderY {
                half_height: half[1],
                radius: half[0],
            },
            PrimitiveFit3::CapsuleY if half[1] >= half[0] => Shape3::CapsuleY {
                half_segment: half[1] - half[0],
                radius: half[0],
            },
            _ => return Err(ShapeError3("capsule height is smaller than its diameter")),
        };
        Ok(PlacedShape3 {
            pose: RigidPose3::translated(self.center()),
            shape,
        })
    }
}

/// Bounds include scene-instance transforms in the model's default pose. No
/// decoding, GPU, mesh collider generation, or ECS is involved.
#[cfg(feature = "model-assets")]
pub fn model_bounds(model: &quasar_assets::model::Model3D) -> Result<Bounds3> {
    model_bounds_selected(model, |_| true)
}
#[cfg(feature = "model-assets")]
pub fn model_bounds_selected(
    model: &quasar_assets::model::Model3D,
    mut include: impl FnMut(&quasar_assets::model::ModelInstance) -> bool,
) -> Result<Bounds3> {
    bounds_from_instances(model, model.instances.iter().filter(|i| include(i)))
}
#[cfg(feature = "model-assets")]
fn bounds_from_instances<'a>(
    model: &quasar_assets::model::Model3D,
    instances: impl Iterator<Item = &'a quasar_assets::model::ModelInstance>,
) -> Result<Bounds3> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for instance in instances {
        let primitive = model
            .primitives
            .get(instance.primitive)
            .ok_or(ShapeError3("invalid model primitive"))?;
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for v in &primitive.vertices {
            for axis in 0..3 {
                if !v.position[axis].is_finite() {
                    return Err(ShapeError3("non-finite model vertex"));
                }
                lo[axis] = lo[axis].min(v.position[axis]);
                hi[axis] = hi[axis].max(v.position[axis]);
            }
        }
        for bits in 0..8 {
            let p: [f32; 4] = std::array::from_fn(|i| {
                if i == 3 {
                    1.
                } else if bits & (1 << i) == 0 {
                    lo[i]
                } else {
                    hi[i]
                }
            });
            let out: [f32; 4] = std::array::from_fn(|row| {
                (0..4)
                    .map(|col| instance.transform[col][row] * p[col])
                    .sum()
            });
            if out.iter().any(|v| !v.is_finite()) || (out[3] - 1.).abs() > 1e-4 {
                return Err(ShapeError3("model transform must be finite and affine"));
            }
            for axis in 0..3 {
                min[axis] = min[axis].min(out[axis]);
                max[axis] = max[axis].max(out[axis]);
            }
        }
    }
    Bounds3::new(min, max)
}

/// Explicit containing Y-cylinder fitted to transformed model vertices. Unlike
/// an AABB half-width, this also encloses faceted/diagonal points. It is a proxy,
/// not an exact mesh collider; select structural instances to exclude decoration.
#[cfg(feature = "model-assets")]
pub fn model_cylinder_y(
    model: &quasar_assets::model::Model3D,
    mut include: impl FnMut(&quasar_assets::model::ModelInstance) -> bool,
) -> Result<PlacedShape3> {
    let selected: Vec<_> = model.instances.iter().filter(|i| include(i)).collect();
    let bounds = bounds_from_instances(model, selected.iter().copied())?;
    let center = bounds.center();
    let mut radius = 0.0f32;
    for instance in selected {
        for vertex in &model.primitives[instance.primitive].vertices {
            let x = instance.transform[3][0]
                + (0..3)
                    .map(|c| instance.transform[c][0] * vertex.position[c])
                    .sum::<f32>();
            let z = instance.transform[3][2]
                + (0..3)
                    .map(|c| instance.transform[c][2] * vertex.position[c])
                    .sum::<f32>();
            radius = radius.max((x - center[0]).hypot(z - center[2]));
        }
    }
    let shape = Shape3::CylinderY {
        radius,
        half_height: bounds.half()[1],
    };
    shape.shared()?;
    Ok(PlacedShape3 {
        pose: RigidPose3::translated(center),
        shape,
    })
}

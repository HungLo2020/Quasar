use crate::error::finite;
use crate::{AnimationError, AnimationResult, Pose, Value};
use std::collections::BTreeSet;

pub const BONE_TRANSLATION_SUFFIX: &str = "translation";
pub const BONE_ROTATION_SUFFIX: &str = "rotation";
pub const BONE_SCALE_SUFFIX: &str = "scale";

/// Local bone transform. Rotation is `[x, y, z, w]`; scale defaults to one.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

impl Transform {
    pub fn new(
        translation: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
    ) -> AnimationResult<Self> {
        if !translation
            .iter()
            .chain(&rotation)
            .chain(&scale)
            .all(|value| value.is_finite())
        {
            return Err(AnimationError::NonFinite);
        }
        let norm = rotation
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            return Err(AnimationError::NonFinite);
        }
        Ok(Self {
            translation,
            rotation: [
                rotation[0] / norm,
                rotation[1] / norm,
                rotation[2] / norm,
                rotation[3] / norm,
            ],
            scale,
        })
    }

    pub fn blended(self, other: Self, weight: f32) -> Self {
        let weight = weight.clamp(0.0, 1.0);
        let rotation = Value::Quat(self.rotation)
            .lerp(Value::Quat(other.rotation), weight)
            .and_then(Value::as_quat)
            .unwrap_or(self.rotation);
        Self {
            translation: lerp3(self.translation, other.translation, weight),
            rotation,
            scale: lerp3(self.scale, other.scale, weight),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Parent index must refer to an earlier bone.
    pub parent: Option<usize>,
    pub rest: Transform,
}

impl Bone {
    pub fn new(
        name: impl Into<String>,
        parent: Option<usize>,
        rest: Transform,
    ) -> AnimationResult<Self> {
        let name = name.into();
        finite(rest.translation[0])?;
        let rotation_norm = rest
            .rotation
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if name.is_empty()
            || !rest
                .translation
                .iter()
                .chain(&rest.rotation)
                .chain(&rest.scale)
                .all(|value| value.is_finite())
            || !rotation_norm.is_finite()
            || rotation_norm <= f32::EPSILON
        {
            return Err(AnimationError::InvalidSkeleton);
        }
        Ok(Self { name, parent, rest })
    }
}

/// Renderer-independent skeleton definition. Skinning, inverse bind matrices
/// and GPU buffers remain renderer-owned (Quasar's 3D renderer currently has no
/// skeletal pipeline).
#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    bones: Vec<Bone>,
}

impl Skeleton {
    pub fn new(bones: Vec<Bone>) -> AnimationResult<Self> {
        if bones.is_empty() {
            return Err(AnimationError::InvalidSkeleton);
        }
        let mut names = BTreeSet::new();
        for (index, bone) in bones.iter().enumerate() {
            if !names.insert(bone.name.clone()) || bone.parent.is_some_and(|parent| parent >= index)
            {
                return Err(AnimationError::InvalidSkeleton);
            }
        }
        Ok(Self { bones })
    }

    pub fn bones(&self) -> &[Bone] {
        &self.bones
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    /// Samples `bone.<name>.translation|rotation|scale` properties. Missing
    /// properties fall back to the rest pose.
    pub fn sample(&self, pose: &Pose) -> SkeletonPose {
        let bones = self
            .bones
            .iter()
            .map(|bone| {
                let prefix = format!("bone.{}.", bone.name);
                Transform {
                    translation: pose
                        .vec3(&(prefix.clone() + BONE_TRANSLATION_SUFFIX))
                        .unwrap_or(bone.rest.translation),
                    rotation: pose
                        .quat(&(prefix.clone() + BONE_ROTATION_SUFFIX))
                        .unwrap_or(bone.rest.rotation),
                    scale: pose
                        .vec3(&(prefix + BONE_SCALE_SUFFIX))
                        .unwrap_or(bone.rest.scale),
                }
            })
            .collect();
        SkeletonPose { bones }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkeletonPose {
    bones: Vec<Transform>,
}

impl SkeletonPose {
    pub fn bones(&self) -> &[Transform] {
        &self.bones
    }

    pub fn blend(&self, other: &Self, weight: f32) -> Self {
        let bones = self
            .bones
            .iter()
            .zip(&other.bones)
            .map(|(left, right)| left.blended(*right, weight))
            .collect();
        Self { bones }
    }
}

fn lerp3(left: [f32; 3], right: [f32; 3], weight: f32) -> [f32; 3] {
    [
        left[0] + (right[0] - left[0]) * weight,
        left[1] + (right[1] - left[1]) * weight,
        left[2] + (right[2] - left[2]) * weight,
    ]
}

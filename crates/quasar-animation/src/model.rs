//! Optional adapters from `quasar-assets` glTF data to clips and skeletons.
//!
//! Enabled by the `model` feature. Conversion stays renderer-neutral:
//!
//! * [`clip_from_model`] turns a [`ModelAnimation`] into a [`Clip`] whose
//!   tracks are `bone.<node>.translation|rotation|scale`, the property names
//!   [`Skeleton::sample`] reads. Any node can be animated, so the same clip
//!   drives skinned joints and rigid-part rigs alike.
//! * [`skeleton_from_model`] builds a [`Skeleton`] for one skin plus a
//!   [`SkinBinding`] that turns a sampled [`SkeletonPose`] into the joint
//!   palette (`world(joint) * inverse_bind`) a renderer uploads.
//! * [`node_local_matrices`] evaluates a [`Pose`] for every node, for
//!   [`Model3D::world_transforms`] on rigid rigs.
//!
//! Markers (footsteps, attack frames) are not part of glTF; pass them to
//! [`clip_from_model_with_markers`] and route the resulting playback events
//! to audio or gameplay in game code.
use crate::{
    AnimationError, AnimationResult, BONE_ROTATION_SUFFIX, BONE_SCALE_SUFFIX,
    BONE_TRANSLATION_SUFFIX, Bone, Clip, Interpolation, Keyframe, Marker, Pose, Skeleton,
    SkeletonPose, Track, Transform, Value,
};
use quasar_assets::model::{
    Model3D, ModelAnimation, ModelChannel, ModelChannelTarget, ModelInterpolation,
    multiply_matrices, trs_matrix,
};

/// Cubic-spline channels are resampled at this rate (plus every authored
/// key) into linear tracks.
pub const CUBIC_RESAMPLE_HZ: f32 = 60.0;

/// Bone/property name for a node: its glTF name, or `name#index` when the
/// name is shared with another node.
pub fn node_bone_name(model: &Model3D, node: usize) -> String {
    let name = &model.nodes[node].name;
    let shared = model
        .nodes
        .iter()
        .enumerate()
        .any(|(index, other)| index != node && &other.name == name);
    if shared {
        format!("{name}#{node}")
    } else {
        name.clone()
    }
}

fn property(model: &Model3D, node: usize, target: ModelChannelTarget) -> String {
    let suffix = match target {
        ModelChannelTarget::Translation => BONE_TRANSLATION_SUFFIX,
        ModelChannelTarget::Rotation => BONE_ROTATION_SUFFIX,
        ModelChannelTarget::Scale => BONE_SCALE_SUFFIX,
    };
    format!("bone.{}.{suffix}", node_bone_name(model, node))
}

fn value(target: ModelChannelTarget, v: [f32; 4]) -> Value {
    match target {
        ModelChannelTarget::Rotation => {
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > f32::EPSILON {
                Value::Quat(v.map(|x| x / norm))
            } else {
                Value::Quat([0., 0., 0., 1.])
            }
        }
        _ => Value::Vec3([v[0], v[1], v[2]]),
    }
}

fn hermite(p0: [f32; 4], m0: [f32; 4], p1: [f32; 4], m1: [f32; 4], dt: f32, s: f32) -> [f32; 4] {
    let s2 = s * s;
    let s3 = s2 * s;
    let h00 = 2. * s3 - 3. * s2 + 1.;
    let h10 = s3 - 2. * s2 + s;
    let h01 = -2. * s3 + 3. * s2;
    let h11 = s3 - s2;
    std::array::from_fn(|i| h00 * p0[i] + h10 * dt * m0[i] + h01 * p1[i] + h11 * dt * m1[i])
}

fn keyframes(channel: &ModelChannel) -> Vec<(f32, [f32; 4])> {
    let mut keys = match channel.interpolation {
        ModelInterpolation::Step | ModelInterpolation::Linear => channel
            .times
            .iter()
            .copied()
            .zip(channel.values.iter().copied())
            .collect::<Vec<_>>(),
        ModelInterpolation::CubicSpline => {
            let point = |k: usize| channel.values[k * 3 + 1];
            let in_tangent = |k: usize| channel.values[k * 3];
            let out_tangent = |k: usize| channel.values[k * 3 + 2];
            let mut out = Vec::new();
            for k in 0..channel.times.len() {
                out.push((channel.times[k], point(k)));
                if k + 1 == channel.times.len() {
                    break;
                }
                let (t0, t1) = (channel.times[k], channel.times[k + 1]);
                let dt = t1 - t0;
                let steps = ((dt * CUBIC_RESAMPLE_HZ).ceil() as usize).clamp(1, 4096);
                for step in 1..steps {
                    let s = step as f32 / steps as f32;
                    out.push((
                        t0 + dt * s,
                        hermite(
                            point(k),
                            out_tangent(k),
                            point(k + 1),
                            in_tangent(k + 1),
                            dt,
                            s,
                        ),
                    ));
                }
            }
            out
        }
    };
    // glTF allows repeated times (instant changes); clips need strictly
    // increasing keys, so the later value wins.
    let mut deduped: Vec<(f32, [f32; 4])> = Vec::with_capacity(keys.len());
    for key in keys.drain(..) {
        match deduped.last_mut() {
            Some(last) if key.0 <= last.0 => last.1 = key.1,
            _ => deduped.push(key),
        }
    }
    deduped
}

/// Converts animation `index` of `model` into a [`Clip`].
pub fn clip_from_model(model: &Model3D, index: usize) -> AnimationResult<Clip> {
    clip_from_model_with_markers(model, index, Vec::new())
}

/// [`clip_from_model`] with game-defined markers (for example footstep
/// frames) attached to the clip.
pub fn clip_from_model_with_markers(
    model: &Model3D,
    index: usize,
    markers: Vec<Marker>,
) -> AnimationResult<Clip> {
    let animation: &ModelAnimation = model
        .animations
        .get(index)
        .ok_or(AnimationError::EmptyClip)?;
    let mut tracks = Vec::with_capacity(animation.channels.len());
    for channel in &animation.channels {
        if channel.node >= model.nodes.len() {
            return Err(AnimationError::InvalidSkeleton);
        }
        let interpolation = match channel.interpolation {
            ModelInterpolation::Step => Interpolation::Step,
            ModelInterpolation::Linear | ModelInterpolation::CubicSpline => Interpolation::Linear,
        };
        let keys = keyframes(channel)
            .into_iter()
            .map(|(time, v)| Keyframe::new(time, value(channel.target, v)))
            .collect();
        tracks.push(Track::new(
            property(model, channel.node, channel.target),
            interpolation,
            keys,
        )?);
    }
    // Single-key clips still need a positive duration.
    let duration = animation.duration.max(1.0 / CUBIC_RESAMPLE_HZ);
    Clip::new(animation.name.clone(), duration, tracks, markers)
}

/// Converts every animation in the model, in file order.
pub fn clips_from_model(model: &Model3D) -> AnimationResult<Vec<Clip>> {
    (0..model.animations.len())
        .map(|index| clip_from_model(model, index))
        .collect()
}

fn rest(model: &Model3D, node: usize) -> AnimationResult<Transform> {
    let n = &model.nodes[node];
    Transform::new(n.translation, n.rotation, n.scale)
}

/// Maps a sampled [`SkeletonPose`] to the skin's joint palette.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinBinding {
    /// For each skin joint (vertex joint index), its bone in the skeleton.
    pub bone_for_joint: Vec<usize>,
    /// Bone parents within the skeleton (earlier indices), as in [`Bone`].
    pub parents: Vec<Option<usize>>,
    /// For root bones, the default world matrix of their non-joint parent
    /// node (armature/root transforms); identity otherwise.
    pub root_parents: Vec<[[f32; 4]; 4]>,
    /// Inverse bind matrix per skin joint.
    pub inverse_bind_matrices: Vec<[[f32; 4]; 4]>,
}

impl SkinBinding {
    /// World matrix per bone for a pose (bones in skeleton order).
    pub fn bone_world(&self, pose: &SkeletonPose) -> Vec<[[f32; 4]; 4]> {
        let mut world: Vec<[[f32; 4]; 4]> = Vec::with_capacity(pose.bones().len());
        for (index, transform) in pose.bones().iter().enumerate() {
            let local = trs_matrix(transform.translation, transform.rotation, transform.scale);
            let parent = match self.parents.get(index).copied().flatten() {
                Some(parent) => world[parent],
                None => self.root_parents[index],
            };
            world.push(multiply_matrices(parent, local));
        }
        world
    }

    /// Skinning palette `world(joint) * inverse_bind` in skin joint order.
    pub fn joint_matrices(&self, pose: &SkeletonPose) -> Vec<[[f32; 4]; 4]> {
        let world = self.bone_world(pose);
        self.bone_for_joint
            .iter()
            .zip(&self.inverse_bind_matrices)
            .map(|(bone, inverse)| multiply_matrices(world[*bone], *inverse))
            .collect()
    }
}

/// Skeleton for skin `skin` of `model`, with bones ordered parents first and
/// named by [`node_bone_name`], plus the binding that produces its palette.
pub fn skeleton_from_model(
    model: &Model3D,
    skin: usize,
) -> AnimationResult<(Skeleton, SkinBinding)> {
    let skin_data = model
        .skins
        .get(skin)
        .ok_or(AnimationError::InvalidSkeleton)?;
    if skin_data
        .joints
        .iter()
        .any(|joint| *joint >= model.nodes.len())
    {
        return Err(AnimationError::InvalidSkeleton);
    }
    let depth = |mut node: usize| {
        let mut depth = 0usize;
        while let Some(parent) = model.nodes[node].parent {
            depth += 1;
            node = parent;
            if depth > model.nodes.len() {
                break;
            }
        }
        depth
    };
    // Parents-first order: sort joints by tree depth, stable on skin order.
    let mut order: Vec<usize> = (0..skin_data.joints.len()).collect();
    order.sort_by_key(|&joint| depth(skin_data.joints[joint]));
    let mut bone_for_joint = vec![0usize; skin_data.joints.len()];
    for (bone, &joint) in order.iter().enumerate() {
        bone_for_joint[joint] = bone;
    }
    let mut bones = Vec::with_capacity(order.len());
    let mut parents = Vec::with_capacity(order.len());
    let mut root_parents = Vec::with_capacity(order.len());
    for &joint in &order {
        let node = skin_data.joints[joint];
        // Nearest ancestor that is also a joint of this skin.
        let mut ancestor = model.nodes[node].parent;
        let mut parent_bone = None;
        while let Some(candidate) = ancestor {
            if let Some(position) = skin_data.joints.iter().position(|j| *j == candidate) {
                parent_bone = Some(bone_for_joint[position]);
                break;
            }
            ancestor = model.nodes[candidate].parent;
        }
        let root_parent = match (parent_bone, model.nodes[node].parent) {
            (None, Some(parent)) => model.nodes[parent].world,
            _ => IDENTITY,
        };
        bones.push(Bone::new(
            node_bone_name(model, node),
            parent_bone,
            rest(model, node)?,
        )?);
        parents.push(parent_bone);
        root_parents.push(root_parent);
    }
    let skeleton = Skeleton::new(bones)?;
    Ok((
        skeleton,
        SkinBinding {
            bone_for_joint,
            parents,
            root_parents,
            inverse_bind_matrices: skin_data.inverse_bind_matrices.clone(),
        },
    ))
}

const IDENTITY: [[f32; 4]; 4] = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

/// Local matrix per node from a sampled pose, falling back to each node's
/// authored translation/rotation/scale for properties the pose lacks. Feed
/// the result to [`Model3D::world_transforms`] to animate rigid-part rigs.
pub fn node_local_matrices(model: &Model3D, pose: &Pose) -> Vec<[[f32; 4]; 4]> {
    (0..model.nodes.len())
        .map(|node| {
            let n = &model.nodes[node];
            let prefix = format!("bone.{}.", node_bone_name(model, node));
            let translation = pose
                .vec3(&(prefix.clone() + BONE_TRANSLATION_SUFFIX))
                .unwrap_or(n.translation);
            let rotation = pose
                .quat(&(prefix.clone() + BONE_ROTATION_SUFFIX))
                .unwrap_or(n.rotation);
            let scale = pose.vec3(&(prefix + BONE_SCALE_SUFFIX)).unwrap_or(n.scale);
            trs_matrix(translation, rotation, scale)
        })
        .collect()
}

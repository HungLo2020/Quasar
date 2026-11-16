//! Voxel colliders for Rapier 3D (feature `voxel`).
//!
//! Cubic chunks become one compound collider of boxes merged greedily from
//! solid cells (far fewer shapes than one cube per cell); smooth (density)
//! chunks become triangle meshes from `quasar-voxel-mesh` output. Colliders
//! are keyed by chunk, rebuilt from edit dirtiness, and either placed in the
//! world (static grids) or attached to a rigid body (dynamic grids such as
//! ships), so a grid moves as one body.
use crate::shapes3::RigidPose3;
use quasar_math::{IVec3, Vec3};
use quasar_voxel::{ChannelId, ChunkLayout, Value, Volume};
use quasar_voxel_mesh::MeshBuffers;
use rapier3d::geometry::{ColliderBuilder, ColliderHandle, SharedShape};
use rapier3d::math::{Pose, Rotation, Vector};
use rapier3d::pipeline::PhysicsWorld as RapierWorld;
use rapier3d::prelude::{RigidBodyBuilder, RigidBodyHandle};
use std::collections::BTreeMap;

/// An axis-aligned box of cells, `[min, max)` in grid voxel coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VoxelBox {
    pub min: IVec3,
    pub max: IVec3,
}

impl VoxelBox {
    pub fn volume(&self) -> i64 {
        let s = self.max - self.min;
        i64::from(s.x) * i64::from(s.y) * i64::from(s.z)
    }
}

/// Greedily merges a chunk's solid cells into boxes: runs along X, grown
/// along Z, then Y. Deterministic; every solid cell is covered exactly once.
pub fn merge_boxes<L: ChunkLayout, T: Value>(
    volume: &Volume<L>,
    channel: ChannelId<T>,
    coord: IVec3,
    mut solid: impl FnMut(T) -> bool,
) -> Vec<VoxelBox> {
    let n = L::EDGE as i32;
    let origin = L::origin(coord);
    let Some(chunk) = volume.chunk(coord) else {
        return Vec::new();
    };
    if let Some(raw) = chunk.storage(channel).uniform() {
        return if solid(T::from_raw(raw)) {
            vec![VoxelBox {
                min: origin,
                max: origin + IVec3::splat(n),
            }]
        } else {
            Vec::new()
        };
    }
    let idx = |x: i32, y: i32, z: i32| ((y * n + z) * n + x) as usize;
    let mut open = vec![false; L::VOLUME];
    for y in 0..n {
        for z in 0..n {
            for x in 0..n {
                open[idx(x, y, z)] = solid(chunk.get(
                    channel,
                    quasar_math::UVec3::new(x as u32, y as u32, z as u32),
                ));
            }
        }
    }
    let mut boxes = Vec::new();
    for y in 0..n {
        for z in 0..n {
            let mut x = 0;
            while x < n {
                if !open[idx(x, y, z)] {
                    x += 1;
                    continue;
                }
                let mut w = 1;
                while x + w < n && open[idx(x + w, y, z)] {
                    w += 1;
                }
                let mut d = 1;
                while z + d < n && (x..x + w).all(|i| open[idx(i, y, z + d)]) {
                    d += 1;
                }
                let mut h = 1;
                while y + h < n && (z..z + d).all(|k| (x..x + w).all(|i| open[idx(i, y + h, k)])) {
                    h += 1;
                }
                for yy in y..y + h {
                    for zz in z..z + d {
                        for xx in x..x + w {
                            open[idx(xx, yy, zz)] = false;
                        }
                    }
                }
                boxes.push(VoxelBox {
                    min: origin + IVec3::new(x, y, z),
                    max: origin + IVec3::new(x + w, y + h, z + d),
                });
                x += w;
            }
        }
    }
    boxes
}

/// A compound of cuboids for `boxes` in metres (`cell_size` per voxel).
pub fn boxes_shape(boxes: &[VoxelBox], cell_size: f32) -> Option<SharedShape> {
    if boxes.is_empty() {
        return None;
    }
    let parts = boxes
        .iter()
        .map(|b| {
            let min = b.min.as_vec3() * cell_size;
            let max = b.max.as_vec3() * cell_size;
            let half = (max - min) * 0.5;
            (
                Pose::from_parts(Vector::from((min + max) * 0.5), Rotation::IDENTITY),
                SharedShape::cuboid(half.x, half.y, half.z),
            )
        })
        .collect();
    Some(SharedShape::compound(parts))
}

/// A triangle mesh from every smooth layer of `mesh` (chunk-local units),
/// placed at grid voxel `origin` with `scale` voxels per unit.
pub fn smooth_trimesh(
    mesh: &MeshBuffers,
    origin: IVec3,
    scale: f32,
    cell_size: f32,
) -> Option<SharedShape> {
    let mut vertices: Vec<Vector> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();
    for (_, layer) in &mesh.layers {
        let base = vertices.len() as u32;
        vertices.extend(
            layer
                .vertices
                .iter()
                .map(|v| Vector::from((origin.as_vec3() + v.position * scale) * cell_size)),
        );
        indices.extend(
            layer
                .indices
                .chunks_exact(3)
                .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
        );
    }
    if indices.is_empty() {
        return None;
    }
    SharedShape::trimesh(vertices, indices).ok()
}

/// Chunk-keyed colliders of one grid.
#[derive(Debug)]
pub struct VoxelColliders {
    cell_size: f32,
    /// Grid pose for static grids (ignored when attached to a body).
    pose: RigidPose3,
    body: Option<RigidBodyHandle>,
    chunks: BTreeMap<(i32, i32, i32), ColliderHandle>,
}

impl VoxelColliders {
    /// Colliders placed directly in the world at `pose` (static grids).
    pub fn new_static(cell_size: f32, pose: RigidPose3) -> Self {
        Self {
            cell_size,
            pose,
            body: None,
            chunks: BTreeMap::new(),
        }
    }
    /// Colliders attached to a rigid body (dynamic grids).
    pub fn attached(cell_size: f32, body: RigidBodyHandle) -> Self {
        Self {
            cell_size,
            pose: RigidPose3::IDENTITY,
            body: Some(body),
            chunks: BTreeMap::new(),
        }
    }
    /// Creates a dynamic body at `pose` for a grid and returns colliders
    /// attached to it.
    pub fn dynamic_body(world: &mut RapierWorld, cell_size: f32, pose: RigidPose3) -> Option<Self> {
        let rapier_pose = pose.rapier().ok()?;
        let body = world.insert_body(RigidBodyBuilder::dynamic().pose(rapier_pose));
        Some(Self::attached(cell_size, body))
    }
    pub fn body(&self) -> Option<RigidBodyHandle> {
        self.body
    }
    pub fn len(&self) -> usize {
        self.chunks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
    pub fn collider(&self, coord: IVec3) -> Option<ColliderHandle> {
        self.chunks.get(&(coord.x, coord.y, coord.z)).copied()
    }
    fn place(&mut self, world: &mut RapierWorld, coord: IVec3, shape: Option<SharedShape>) {
        self.remove(world, coord);
        let Some(shape) = shape else {
            return;
        };
        let builder = ColliderBuilder::new(shape);
        let handle = match self.body {
            Some(body) => world.insert_collider(builder, Some(body)),
            None => {
                let pose = self.pose.rapier().unwrap_or(Pose::IDENTITY);
                world.insert_collider(builder.position(pose), None)
            }
        };
        self.chunks.insert((coord.x, coord.y, coord.z), handle);
    }
    /// Rebuilds a cubic chunk's collider from solid cells.
    pub fn rebuild_cubic<L: ChunkLayout, T: Value>(
        &mut self,
        world: &mut RapierWorld,
        volume: &Volume<L>,
        channel: ChannelId<T>,
        coord: IVec3,
        solid: impl FnMut(T) -> bool,
    ) -> usize {
        let boxes = merge_boxes(volume, channel, coord, solid);
        let count = boxes.len();
        self.place(world, coord, boxes_shape(&boxes, self.cell_size));
        count
    }
    /// Rebuilds a smooth chunk's collider from its extracted mesh.
    pub fn rebuild_smooth(
        &mut self,
        world: &mut RapierWorld,
        coord: IVec3,
        mesh: &MeshBuffers,
        origin: IVec3,
        scale: f32,
    ) {
        self.place(
            world,
            coord,
            smooth_trimesh(mesh, origin, scale, self.cell_size),
        );
    }
    /// Rebuilds every dirty cubic chunk (e.g. `EditResult::dirty`).
    pub fn apply_dirty<L: ChunkLayout, T: Value>(
        &mut self,
        world: &mut RapierWorld,
        volume: &Volume<L>,
        channel: ChannelId<T>,
        dirty: &[IVec3],
        mut solid: impl FnMut(T) -> bool,
    ) {
        for coord in dirty {
            self.rebuild_cubic(world, volume, channel, *coord, &mut solid);
        }
    }
    pub fn remove(&mut self, world: &mut RapierWorld, coord: IVec3) {
        if let Some(handle) = self.chunks.remove(&(coord.x, coord.y, coord.z)) {
            world.remove_collider(handle);
        }
    }
    /// Removes every collider (and the body, for dynamic grids).
    pub fn clear(&mut self, world: &mut RapierWorld) {
        let coords: Vec<_> = self.chunks.keys().copied().collect();
        for (x, y, z) in coords {
            self.remove(world, IVec3::new(x, y, z));
        }
        if let Some(body) = self.body.take() {
            world.remove_body(body);
        }
    }
    /// World pose of the grid (body pose for dynamic grids).
    pub fn grid_pose(&self, world: &RapierWorld) -> RigidPose3 {
        match self.body.and_then(|b| world.bodies.get(b)) {
            Some(body) => {
                let p = body.position();
                RigidPose3 {
                    translation: p.translation.to_array(),
                    rotation: p.rotation.to_array(),
                }
            }
            None => self.pose,
        }
    }
}

/// `Vec3` helper used by tests and callers converting voxel points.
pub fn voxel_point(voxel: Vec3, cell_size: f32) -> [f32; 3] {
    (voxel * cell_size).to_array()
}

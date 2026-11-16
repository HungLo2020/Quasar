//! Grids (volumes placed in the world) and multi-grid scenes.
//!
//! A [`Grid`] is a volume plus an `f64` transform and a cell size. Voxel
//! coordinates stay grid-local `i32`, so each grid (a world, an asteroid, a
//! ship) has its own origin while the scene spans any distance. Render code
//! rebases grid transforms to the camera in `f64` before converting to `f32`.
use crate::chunk::ChannelId;
use crate::layout::ChunkLayout;
use crate::query::{RayHit, TieBreak};
use crate::storage::Value;
use crate::volume::Volume;
use quasar_math::{DMat4, DQuat, DVec3, IVec3, Vec3};
use std::any::Any;

/// Stable identifier of a grid in a scene.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GridId(pub u64);

/// Whether a grid moves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GridKind {
    #[default]
    Static,
    Dynamic,
}

/// Placement of a grid's voxel origin in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GridTransform {
    pub position: DVec3,
    pub rotation: DQuat,
}

impl Default for GridTransform {
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
        }
    }
}

impl GridTransform {
    pub fn new(position: DVec3, rotation: DQuat) -> Self {
        Self {
            position,
            rotation: rotation.normalize(),
        }
    }
    /// Matrix from voxel space (scaled by `cell_size`) to world space.
    pub fn matrix(&self, cell_size: f64) -> DMat4 {
        DMat4::from_scale_rotation_translation(
            DVec3::splat(cell_size),
            self.rotation,
            self.position,
        )
    }
    /// Voxel-to-world matrix relative to `camera` (subtracted in `f64`), for
    /// precise `f32` rendering far from the origin.
    pub fn camera_relative(&self, cell_size: f64, camera: DVec3) -> quasar_math::Mat4 {
        DMat4::from_scale_rotation_translation(
            DVec3::splat(cell_size),
            self.rotation,
            self.position - camera,
        )
        .as_mat4()
    }
}

/// A volume placed in the world.
#[derive(Clone, Debug)]
pub struct Grid<L: ChunkLayout> {
    pub id: GridId,
    pub volume: Volume<L>,
    pub transform: GridTransform,
    /// World metres per voxel.
    pub cell_size: f64,
    pub kind: GridKind,
}

impl<L: ChunkLayout> Grid<L> {
    pub fn new(id: GridId, volume: Volume<L>, cell_size: f64) -> Self {
        assert!(
            cell_size > 0.0 && cell_size.is_finite(),
            "cell size must be positive"
        );
        Self {
            id,
            volume,
            transform: GridTransform::default(),
            cell_size,
            kind: GridKind::Static,
        }
    }
    pub fn with_transform(mut self, transform: GridTransform) -> Self {
        self.transform = transform;
        self
    }
    pub fn with_kind(mut self, kind: GridKind) -> Self {
        self.kind = kind;
        self
    }
    /// World point to grid voxel coordinates.
    pub fn world_to_voxel(&self, world: DVec3) -> DVec3 {
        self.transform.rotation.inverse() * (world - self.transform.position) / self.cell_size
    }
    pub fn voxel_to_world(&self, voxel: DVec3) -> DVec3 {
        self.transform.position + self.transform.rotation * (voxel * self.cell_size)
    }
    /// World direction to voxel-space direction (unnormalised; scaled by 1/cell size).
    pub fn world_dir_to_voxel(&self, dir: DVec3) -> DVec3 {
        self.transform.rotation.inverse() * dir / self.cell_size
    }
    /// Inclusive voxel bounds of loaded chunks.
    pub fn voxel_bounds(&self) -> Option<(IVec3, IVec3)> {
        let coords = self.volume.chunk_coords();
        let first = *coords.first()?;
        let (lo, hi) = coords
            .iter()
            .fold((first, first), |(lo, hi), c| (lo.min(*c), hi.max(*c)));
        Some((
            L::origin(lo),
            L::origin(hi) + IVec3::splat(L::EDGE as i32 - 1),
        ))
    }
    /// World-space AABB of loaded chunks.
    pub fn world_bounds(&self) -> Option<(DVec3, DVec3)> {
        let (lo, hi) = self.voxel_bounds()?;
        let (lo, hi) = (lo.as_dvec3(), (hi + IVec3::ONE).as_dvec3());
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for i in 0..8 {
            let corner = DVec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let w = self.voxel_to_world(corner);
            min = min.min(w);
            max = max.max(w);
        }
        Some((min, max))
    }
    /// Moves `cells` into a new grid with the same transform, cell size and
    /// kind (used after [`crate::split_check`]).
    pub fn split_off(&mut self, id: GridId, cells: &[IVec3]) -> Grid<L> {
        Grid {
            id,
            volume: self.volume.extract_cells(cells),
            transform: self.transform,
            cell_size: self.cell_size,
            kind: self.kind,
        }
    }
}

/// A grid in a [`Scene`], type-erased for world-space queries.
pub trait SceneGrid: Send + Sync + 'static {
    fn id(&self) -> GridId;
    fn transform(&self) -> GridTransform;
    fn cell_size(&self) -> f64;
    fn world_bounds(&self) -> Option<(DVec3, DVec3)>;
    /// Ray in grid voxel space (direction normalised in voxel units).
    fn raycast_voxels(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A grid whose solid cells are decided by a predicate on one channel.
pub struct SolidGrid<L: ChunkLayout, T: Value> {
    pub grid: Grid<L>,
    pub channel: ChannelId<T>,
    pub solid: fn(T) -> bool,
}

impl<L: ChunkLayout, T: Value> SceneGrid for SolidGrid<L, T> {
    fn id(&self) -> GridId {
        self.grid.id
    }
    fn transform(&self) -> GridTransform {
        self.grid.transform
    }
    fn cell_size(&self) -> f64 {
        self.grid.cell_size
    }
    fn world_bounds(&self) -> Option<(DVec3, DVec3)> {
        self.grid.world_bounds()
    }
    fn raycast_voxels(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit> {
        let solid = self.solid;
        self.grid.volume.raycast(
            self.channel,
            origin,
            direction,
            max_distance,
            TieBreak::default(),
            |_, v| solid(v),
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A grid hit by its density surface.
pub struct DensityGrid<L: ChunkLayout> {
    pub grid: Grid<L>,
    pub density: ChannelId<i8>,
}

impl<L: ChunkLayout> SceneGrid for DensityGrid<L> {
    fn id(&self) -> GridId {
        self.grid.id
    }
    fn transform(&self) -> GridTransform {
        self.grid.transform
    }
    fn cell_size(&self) -> f64 {
        self.grid.cell_size
    }
    fn world_bounds(&self) -> Option<(DVec3, DVec3)> {
        self.grid.world_bounds()
    }
    fn raycast_voxels(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit> {
        self.grid
            .volume
            .raycast_density(self.density, origin, direction, max_distance, 0.25)
            .map(|(hit, _)| hit)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A world-space hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneHit {
    pub grid: GridId,
    /// Hit in the grid's voxel space.
    pub hit: RayHit,
    pub world_point: DVec3,
    /// World distance from the ray origin.
    pub distance: f64,
}

fn ray_aabb(origin: DVec3, dir: DVec3, min: DVec3, max: DVec3, max_distance: f64) -> Option<f64> {
    let mut t0 = 0.0f64;
    let mut t1 = max_distance;
    for axis in 0..3 {
        if dir[axis].abs() < 1e-15 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let (mut a, mut b) = (
            (min[axis] - origin[axis]) * inv,
            (max[axis] - origin[axis]) * inv,
        );
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// Many grids of any chunk layout.
#[derive(Default)]
pub struct Scene {
    grids: Vec<Box<dyn SceneGrid>>,
    next_id: u64,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            grids: Vec::new(),
            next_id: 1,
        }
    }
    /// A fresh, never-reused grid ID.
    pub fn allocate_id(&mut self) -> GridId {
        self.next_id = self.next_id.max(1);
        let id = GridId(self.next_id);
        self.next_id += 1;
        id
    }
    pub fn insert(&mut self, grid: Box<dyn SceneGrid>) {
        self.next_id = self.next_id.max(grid.id().0 + 1);
        self.grids.push(grid);
        self.grids.sort_by_key(|g| g.id());
    }
    pub fn remove(&mut self, id: GridId) -> Option<Box<dyn SceneGrid>> {
        let index = self.grids.iter().position(|g| g.id() == id)?;
        Some(self.grids.remove(index))
    }
    /// Grids in ID order.
    pub fn grids(&self) -> impl Iterator<Item = &dyn SceneGrid> {
        self.grids.iter().map(|g| g.as_ref())
    }
    pub fn len(&self) -> usize {
        self.grids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.grids.is_empty()
    }
    pub fn get<G: SceneGrid>(&self, id: GridId) -> Option<&G> {
        self.grids
            .iter()
            .find(|g| g.id() == id)?
            .as_any()
            .downcast_ref()
    }
    pub fn get_mut<G: SceneGrid>(&mut self, id: GridId) -> Option<&mut G> {
        self.grids
            .iter_mut()
            .find(|g| g.id() == id)?
            .as_any_mut()
            .downcast_mut()
    }
    /// Grids whose world bounds overlap the box.
    pub fn grids_in_box(&self, min: DVec3, max: DVec3) -> Vec<GridId> {
        self.grids
            .iter()
            .filter(|g| {
                g.world_bounds()
                    .is_some_and(|(lo, hi)| lo.cmple(max).all() && hi.cmpge(min).all())
            })
            .map(|g| g.id())
            .collect()
    }
    /// Nearest hit across all grids; ties keep the lower grid ID.
    pub fn raycast(&self, origin: DVec3, direction: DVec3, max_distance: f64) -> Option<SceneHit> {
        let dir = direction.normalize_or_zero();
        if dir == DVec3::ZERO {
            return None;
        }
        let mut best: Option<SceneHit> = None;
        for grid in &self.grids {
            let Some((min, max)) = grid.world_bounds() else {
                continue;
            };
            let limit = best.map_or(max_distance, |b| b.distance);
            let Some(enter) = ray_aabb(origin, dir, min, max, limit) else {
                continue;
            };
            let t = grid.transform();
            let cell = grid.cell_size();
            let local_origin = t.rotation.inverse() * (origin + dir * enter - t.position) / cell;
            let local_dir = (t.rotation.inverse() * dir).normalize();
            let remaining = ((limit - enter) / cell) as f32;
            if let Some(hit) =
                grid.raycast_voxels(local_origin.as_vec3(), local_dir.as_vec3(), remaining)
            {
                let distance = enter + f64::from(hit.t) * cell;
                if best.is_none_or(|b| distance < b.distance) {
                    best = Some(SceneHit {
                        grid: grid.id(),
                        hit,
                        world_point: origin + dir * distance,
                        distance,
                    });
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Schema;
    use crate::layout::{Cube16, Cube32};
    use crate::storage::StorageKind;

    fn block_grid(id: u64, cell: f64) -> (Grid<Cube16>, ChannelId<u16>) {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        (Grid::new(GridId(id), Volume::new(schema), cell), block)
    }

    #[test]
    fn transforms_round_trip_and_bounds_follow_rotation() {
        let (mut grid, block) = block_grid(1, 2.5);
        grid.transform = GridTransform::new(
            DVec3::new(1e9, -3.0, 4.0),
            DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
        );
        let p = DVec3::new(3.0, 4.0, 5.0);
        assert!((grid.world_to_voxel(grid.voxel_to_world(p)) - p).length() < 1e-6);
        grid.volume.set(block, IVec3::ZERO, 1);
        let (min, max) = grid.world_bounds().unwrap();
        assert!((max.y - min.y - 40.0).abs() < 1e-6, "16 voxels × 2.5 m");
        let rel = grid
            .transform
            .camera_relative(2.5, DVec3::new(1e9, 0.0, 0.0));
        assert!((rel.w_axis.x).abs() < 1e-3, "precise after rebasing");
    }

    #[test]
    fn scene_raycasts_pick_the_nearest_grid_in_world_space() {
        let mut scene = Scene::new();
        let (mut near, block) = block_grid(1, 1.0);
        near.transform.position = DVec3::new(10.0, 0.0, 0.0);
        near.volume.set(block, IVec3::new(0, 0, 0), 1);
        let (mut far, block2) = block_grid(2, 2.5);
        far.transform = GridTransform::new(DVec3::new(30.0, 0.0, 0.0), DQuat::from_rotation_z(0.3));
        far.volume.set(block2, IVec3::new(0, 0, 0), 1);
        scene.insert(Box::new(SolidGrid {
            grid: far,
            channel: block2,
            solid: |v| v != 0,
        }));
        scene.insert(Box::new(SolidGrid {
            grid: near,
            channel: block,
            solid: |v| v != 0,
        }));
        let hit = scene
            .raycast(DVec3::new(0.0, 0.5, 0.5), DVec3::X, 100.0)
            .unwrap();
        assert_eq!(hit.grid, GridId(1));
        assert!((hit.distance - 10.0).abs() < 1e-4, "{}", hit.distance);
        assert_eq!(hit.hit.voxel, IVec3::ZERO);
        let behind = scene
            .raycast(DVec3::new(20.0, 0.5, 0.5), DVec3::X, 100.0)
            .unwrap();
        assert_eq!(behind.grid, GridId(2));
        assert_eq!(
            scene.grids_in_box(DVec3::splat(-1.0), DVec3::splat(12.0)),
            vec![GridId(1)]
        );
        assert!(scene.get::<SolidGrid<Cube16, u16>>(GridId(2)).is_some());
        assert!(scene.get::<DensityGrid<Cube32>>(GridId(2)).is_none());
        assert_eq!(scene.allocate_id(), GridId(3));
        assert!(scene.remove(GridId(1)).is_some());
    }
}

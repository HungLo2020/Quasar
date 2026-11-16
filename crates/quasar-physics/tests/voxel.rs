#![cfg(feature = "voxel")]
use quasar_math::{IVec3, Vec3};
use quasar_physics::StepConfig;
use quasar_physics::dim3::RigidPose3;
use quasar_physics::dim3::{CollisionWorld, PhysicsWorld, WorldConfig3};
use quasar_physics::rapier3d::prelude::*;
use quasar_physics::voxel3::*;
use quasar_voxel::{
    Brush, BrushOp, BrushShape, Cube16, EditBatch, EditOptions, Schema, Snapshot, StorageKind,
    Volume,
};
use quasar_voxel_mesh::{MeshBuffers, SmoothOptions, SurfaceNetsExtractor};
use std::collections::HashSet;

fn terrain() -> (Volume<Cube16>, quasar_voxel::ChannelId<u16>) {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    let mut volume = Volume::new(schema);
    let mut rng = quasar_core::Rng::seed_from_u64(3);
    for x in 0..16 {
        for z in 0..16 {
            let h = 2 + rng.range_i32(0..5);
            for y in 0..h {
                volume.set(block, IVec3::new(x, y, z), 1);
            }
        }
    }
    (volume, block)
}

#[test]
fn merged_boxes_cover_every_solid_cell_exactly_once() {
    let (volume, block) = terrain();
    let boxes = merge_boxes(&volume, block, IVec3::ZERO, |v| v != 0);
    let mut cells = HashSet::new();
    for b in &boxes {
        for y in b.min.y..b.max.y {
            for z in b.min.z..b.max.z {
                for x in b.min.x..b.max.x {
                    assert!(cells.insert((x, y, z)), "no overlap");
                    assert_eq!(volume.get(block, IVec3::new(x, y, z)), 1);
                }
            }
        }
    }
    let solid = (0..16 * 16 * 16)
        .filter(|i| volume.get(block, IVec3::new(i % 16, i / 256, (i / 16) % 16)) != 0)
        .count();
    assert_eq!(cells.len(), solid);
    assert!(
        boxes.len() * 4 < solid,
        "merging reduces shapes: {} boxes for {solid} cells",
        boxes.len()
    );
    let mut full = Volume::<Cube16>::new(volume.schema().as_ref().clone());
    let mut chunk = quasar_voxel::Chunk::new(full.schema());
    chunk.fill(block, 1);
    full.insert_chunk(IVec3::ZERO, chunk);
    assert_eq!(
        merge_boxes(&full, block, IVec3::ZERO, |v| v != 0).len(),
        1,
        "uniform chunks are one box"
    );
}

#[test]
fn rays_hit_voxel_colliders_and_edits_rebuild_them() {
    let (mut volume, block) = terrain();
    let mut world = CollisionWorld::new();
    let mut colliders = VoxelColliders::new_static(2.0, RigidPose3::IDENTITY);
    colliders.rebuild_cubic(world.raw_mut(), &volume, block, IVec3::ZERO, |v| v != 0);
    world.update();
    let down = world
        .raycast([5.0, 40.0, 5.0], [0.0, -1.0, 0.0], 100.0, true)
        .unwrap();
    let top = (0..16)
        .rev()
        .find(|y| volume.get(block, IVec3::new(2, *y, 2)) != 0)
        .unwrap();
    assert!(
        (down.point[1] - (top + 1) as f32 * 2.0).abs() < 1e-3,
        "{:?}",
        down.point
    );
    // Dig the column out and rebuild only the dirty chunk.
    let mut batch = EditBatch::new();
    batch.fill(block, IVec3::new(2, 0, 2), IVec3::new(2, 15, 2), 0);
    let result = volume.apply(&batch, EditOptions::default());
    colliders.apply_dirty(world.raw_mut(), &volume, block, &result.dirty, |v| v != 0);
    world.update();
    assert!(
        world
            .raycast([5.0, 40.0, 5.0], [0.0, -1.0, 0.0], 100.0, true)
            .is_none(),
        "the hole goes through"
    );
    assert!(!colliders.is_empty());
}

#[test]
fn dynamic_bodies_land_on_voxel_floors_and_grids_move_as_bodies() {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    let mut floor = Volume::<Cube16>::new(schema.clone());
    let mut batch = EditBatch::new();
    batch.fill(block, IVec3::new(0, 0, 0), IVec3::new(15, 0, 15), 1);
    floor.apply(&batch, EditOptions::default());
    let mut world = PhysicsWorld::new(WorldConfig3::default()).unwrap();
    world.set_gravity([0.0, -9.81, 0.0]).unwrap();
    let mut ground = VoxelColliders::new_static(1.0, RigidPose3::IDENTITY);
    ground.rebuild_cubic(world.raw_mut(), &floor, block, IVec3::ZERO, |v| v != 0);
    // A one-block ship as a dynamic grid falling onto the floor.
    let mut ship = Volume::<Cube16>::new(schema);
    ship.set(block, IVec3::ZERO, 1);
    let mut grid = VoxelColliders::dynamic_body(
        world.raw_mut(),
        1.0,
        RigidPose3::translated([7.5, 6.0, 7.5]),
    )
    .unwrap();
    grid.rebuild_cubic(world.raw_mut(), &ship, block, IVec3::ZERO, |v| v != 0);
    world.set_step_config(StepConfig::default()).unwrap();
    for _ in 0..240 {
        world.step_fixed();
    }
    let pose = grid.grid_pose(world.raw());
    assert!(
        (pose.translation[1] - 1.0).abs() < 0.05,
        "rests on the floor (y = 1): {:?}",
        pose.translation
    );
    assert!((pose.translation[0] - 7.5).abs() < 0.05);
    grid.clear(world.raw_mut());
    assert!(grid.is_empty() && grid.body().is_none());
}

#[test]
fn smooth_chunks_become_trimesh_colliders() {
    let mut schema = Schema::new();
    let density = schema.channel("density", StorageKind::Dense, -127i8);
    let mut volume = Volume::<Cube16>::new(schema);
    let mut batch = EditBatch::new();
    batch.brush(Brush::new(
        BrushShape::Sphere {
            center: Vec3::splat(8.0),
            radius: 5.0,
        },
        BrushOp::Add,
        density,
    ));
    volume.apply(&batch, EditOptions::default());
    let mut snap = Snapshot::new();
    volume.snapshot(density, IVec3::ZERO, 1, &mut snap);
    let mut mesh = MeshBuffers::new();
    SurfaceNetsExtractor::new().extract::<u8>(&snap, None, SmoothOptions::default(), &mut mesh);
    let mut world = CollisionWorld::new();
    let mut colliders = VoxelColliders::new_static(1.0, RigidPose3::IDENTITY);
    colliders.rebuild_smooth(world.raw_mut(), IVec3::ZERO, &mesh, IVec3::ZERO, 1.0);
    world.update();
    let hit = world
        .raycast([8.0, 30.0, 8.0], [0.0, -1.0, 0.0], 100.0, true)
        .unwrap();
    assert!((hit.point[1] - 13.0).abs() < 0.3, "{:?}", hit.point);
    assert!(smooth_trimesh(&MeshBuffers::new(), IVec3::ZERO, 1.0, 1.0).is_none());
    let _ = ColliderBuilder::ball(1.0);
}

use quasar_math::{DVec3, IVec3};
use quasar_voxel::Face;
use voxel_drift::world::{DriftWorld, autoplay};

fn run(ticks: u64, workers: usize) -> DriftWorld {
    let mut world = DriftWorld::new(0xd51f7, workers);
    for _ in 0..ticks {
        let input = autoplay(&mut world);
        world.update(input);
    }
    world
}

#[test]
fn autoplay_is_deterministic_across_thread_counts() {
    let a = run(420, 1);
    let b = run(420, 3);
    assert_eq!(a.state_hash(), b.state_hash());
    assert!(a.stats.digs > 0 && a.stats.fills > 0, "{:?}", a.stats);
    assert_eq!(a.stats.splits, 1);
    assert_eq!(a.stats.ships, 2);
}

#[test]
fn cutting_the_neck_splits_the_ship_and_the_halves_drift_apart() {
    let mut world = DriftWorld::new(5, 2);
    let blocks = world.stats.ship_blocks;
    let before = world.ship_cell_world(0, IVec3::new(0, 0, 8));
    assert!(world.remove_ship_block(0, IVec3::new(0, 0, 5)));
    assert_eq!(
        world.fleet.ships.len(),
        2,
        "front half becomes its own grid"
    );
    for _ in 0..180 {
        world.update(Default::default());
    }
    assert_eq!(world.stats.ship_blocks, blocks - 1);
    // The front half kept its cells and moved away from the rear half.
    let front = &world.fleet.ships[1];
    assert_ne!(
        front
            .grid
            .volume
            .get(world.fleet.ch.block, IVec3::new(0, 0, 8)),
        quasar_voxel::AIR
    );
    let after = world.ship_cell_world(1, IVec3::new(0, 0, 8));
    let rear = world.ship_cell_world(0, IVec3::new(0, 0, 2));
    assert!(
        (after - before).length() > 1.0,
        "drifted {:.2} m",
        (after - before).length()
    );
    assert!(
        (after - rear).length() > (before - rear).length(),
        "moving apart"
    );
    // Removing an end block does not split.
    assert!(world.remove_ship_block(0, IVec3::new(1, 1, 0)));
    assert_eq!(world.fleet.ships.len(), 2);
}

#[test]
fn placing_oriented_blocks_and_multi_cell_thrusters() {
    let mut world = DriftWorld::new(5, 2);
    let thruster = world.content.blocks.thruster;
    let slope = world.content.blocks.slope;
    // A thruster mounted on the hull's right side extends two cells.
    assert!(world.place_ship_block(0, IVec3::new(2, 1, 1), Face::PosX, thruster, 1));
    let ch = world.fleet.ch;
    let ship = &world.fleet.ships[0];
    // Up = +X (the mounted face), turn 1 = forward +Y: the second cell sits above.
    for cell in [IVec3::new(2, 1, 1), IVec3::new(2, 2, 1)] {
        assert_eq!(ship.grid.volume.get(ch.block, cell), thruster, "{cell}");
    }
    assert_eq!(
        world
            .fleet
            .content
            .catalog
            .anchor_of(&ship.grid.volume, ch, IVec3::new(2, 2, 1)),
        Some(IVec3::new(2, 1, 1))
    );
    // Occupied cells refuse placement.
    assert!(!world.place_ship_block(0, IVec3::new(0, 0, 0), Face::PosY, slope, 0));
    assert!(world.place_ship_block(0, IVec3::new(-2, 0, 3), Face::NegX, slope, 2));
}

#[test]
fn drilling_digs_the_asteroid_and_its_colliders() {
    use quasar_physics::rapier3d::prelude::*;
    let mut world = DriftWorld::new(9, 2);
    let surface = |w: &DriftWorld| {
        w.asteroid
            .grid
            .volume
            .raycast_density(
                w.asteroid.density,
                Vec3::new(0.0, 0.0, 90.0),
                Vec3::NEG_Z,
                120.0,
                0.25,
            )
            .map(|(hit, _)| hit.point.z)
            .unwrap()
    };
    let before = surface(&world);
    world.dig(Vec3::new(0.0, 0.0, before), false);
    world.update(Default::default());
    let after = surface(&world);
    assert!(
        after < before - 2.0,
        "the drill made a hole: {before} -> {after}"
    );
    // Physics sees the hole too.
    let ray = Ray::new(Vector::new(0.0, 0.0, 90.0), Vector::new(0.0, 0.0, -1.0));
    let raw = world.physics.raw();
    let hit = raw
        .query_pipeline()
        .cast_ray(&ray, 200.0, true)
        .map(|(_, t)| 90.0 - t);
    let hit = hit.expect("collider hit");
    assert!(
        (hit - after).abs() < 1.5,
        "collider surface {hit} vs density surface {after}"
    );
    let _ = DVec3::ZERO;
}

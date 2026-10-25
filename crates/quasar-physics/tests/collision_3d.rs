#![cfg(feature = "dim3")]

use quasar_physics::dim3::CollisionWorld;
use quasar_physics::rapier3d::prelude::ColliderBuilder;

#[test]
fn collision_only_world_compiles_and_queries_without_dynamics() {
    let mut world = CollisionWorld::new();
    let ground = world.add_collider(ColliderBuilder::cuboid(5.0, 0.5, 5.0));
    world.update();
    let hit = world
        .raycast([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], 20.0, true)
        .unwrap();
    assert_eq!(hit.collider, ground);
    assert!((hit.distance - 4.5).abs() < 1e-4);
}

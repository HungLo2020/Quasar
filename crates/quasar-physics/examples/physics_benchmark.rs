//! Local Rapier workload probe. Not a portable benchmark.
//!
//! ```sh
//! cargo run --release -p quasar-physics --example physics_benchmark
//! ```
#[cfg(all(feature = "dim2", feature = "dynamics"))]
fn benchmark_2d() {
    use quasar_physics::dim2::{PhysicsWorld, WorldConfig};
    use quasar_physics::rapier2d::prelude::*;
    use std::time::Instant;

    let mut world = PhysicsWorld::new(WorldConfig::default()).unwrap();
    world.insert(
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(50.0, 0.5),
    );
    for index in 0..500 {
        world.insert(
            RigidBodyBuilder::dynamic().translation(Vector::new(
                (index % 25) as f32 - 12.0,
                2.0 + index as f32 * 0.1,
            )),
            ColliderBuilder::ball(0.35).restitution(0.4),
        );
    }
    let start = Instant::now();
    for _ in 0..600 {
        world.step_fixed();
    }
    let elapsed = start.elapsed();
    println!(
        "2d bodies={} colliders={} steps={} elapsed_ms={:.3} step_us={:.3}",
        world.bodies.len(),
        world.colliders.len(),
        world.fixed_steps(),
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000_000.0 / 600.0
    );
    let start = Instant::now();
    let mut hits = 0usize;
    for index in 0..2_000 {
        let x = (index % 50) as f32 - 25.0;
        hits += usize::from(world.raycast([x, 20.0], [0.0, -1.0], 100.0, true).is_some());
    }
    println!(
        "2d raycasts=2000 hits={hits} mean_us={:.3}",
        start.elapsed().as_secs_f64() * 1_000_000.0 / 2_000.0
    );
}

#[cfg(all(feature = "dim3", feature = "dynamics"))]
fn benchmark_3d() {
    use quasar_physics::dim3::{PhysicsWorld, WorldConfig};
    use quasar_physics::rapier3d::prelude::*;
    use std::time::Instant;

    let mut world = PhysicsWorld::new(WorldConfig::default()).unwrap();
    world.insert(
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(50.0, 0.5, 50.0),
    );
    for index in 0..500 {
        world.insert(
            RigidBodyBuilder::dynamic().translation(Vector::new(
                (index % 25) as f32 - 12.0,
                2.0 + index as f32 * 0.1,
                (index / 25) as f32 - 10.0,
            )),
            ColliderBuilder::cuboid(0.4, 0.4, 0.4).restitution(0.2),
        );
    }
    let start = Instant::now();
    for _ in 0..600 {
        world.step_fixed();
    }
    let elapsed = start.elapsed();
    println!(
        "3d bodies={} colliders={} steps={} elapsed_ms={:.3} step_us={:.3}",
        world.bodies.len(),
        world.colliders.len(),
        world.fixed_steps(),
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000_000.0 / 600.0
    );
    let start = Instant::now();
    let mut hits = 0usize;
    for index in 0..2_000 {
        let x = (index % 50) as f32 - 25.0;
        hits += usize::from(
            world
                .raycast([x, 20.0, 0.0], [0.0, -1.0, 0.0], 100.0, true)
                .is_some(),
        );
    }
    println!(
        "3d raycasts=2000 hits={hits} mean_us={:.3}",
        start.elapsed().as_secs_f64() * 1_000_000.0 / 2_000.0
    );
}

fn main() {
    #[cfg(all(feature = "dim2", feature = "dynamics"))]
    benchmark_2d();
    #[cfg(all(feature = "dim3", feature = "dynamics"))]
    benchmark_3d();
}

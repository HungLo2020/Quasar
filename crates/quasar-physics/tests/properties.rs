#![cfg(all(feature = "dim2", feature = "dim3", feature = "dynamics"))]
use proptest::prelude::*;
use quasar_physics::dim2::{PhysicsWorld as Physics2, WorldConfig as World2};
use quasar_physics::dim3::{PhysicsWorld as Physics3, WorldConfig as World3};
use quasar_physics::rapier2d::prelude::{
    ColliderBuilder as Collider2Builder, RigidBodyBuilder as Body2Builder, Vector as Vector2,
};
use quasar_physics::rapier3d::prelude::{
    ColliderBuilder as Collider3Builder, RigidBodyBuilder as Body3Builder, Vector as Vector3,
};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn dropped_balls_never_penetrate_the_ground_2d(
        x in -12.0f32..12.0,
        height in 2.0f32..10.0,
        vx in -4.0f32..4.0,
        restitution in 0.0f32..1.0,
        friction in 0.0f32..1.0,
    ) {
        let mut world = Physics2::new(World2::default()).unwrap();
        world.insert(
            Body2Builder::fixed(),
            Collider2Builder::cuboid(40.0, 0.5).translation(Vector2::new(0.0, -0.5)),
        );
        let (ball, _) = world.insert(
            Body2Builder::dynamic()
                .translation(Vector2::new(x, height))
                .linvel(Vector2::new(vx, 0.0)),
            Collider2Builder::ball(0.5).restitution(restitution).friction(friction),
        );
        for _ in 0..300 {
            world.step_fixed();
        }
        let position = world.bodies[ball].translation();
        prop_assert!(position.x.is_finite() && position.y.is_finite());
        prop_assert!(position.y >= 0.5 - 2e-2, "ball sank to {}", position.y);
    }

    #[test]
    fn dropped_balls_never_penetrate_the_ground_3d(
        x in -8.0f32..8.0,
        z in -8.0f32..8.0,
        height in 2.0f32..10.0,
        restitution in 0.0f32..0.9,
        friction in 0.0f32..1.0,
    ) {
        let mut world = Physics3::new(World3::default()).unwrap();
        world.insert(
            Body3Builder::fixed(),
            Collider3Builder::cuboid(40.0, 0.5, 40.0).translation(Vector3::new(0.0, -0.5, 0.0)),
        );
        let (ball, _) = world.insert(
            Body3Builder::dynamic().translation(Vector3::new(x, height, z)),
            Collider3Builder::ball(0.5).restitution(restitution).friction(friction),
        );
        for _ in 0..300 {
            world.step_fixed();
        }
        let position = world.bodies[ball].translation();
        prop_assert!(position.x.is_finite() && position.y.is_finite() && position.z.is_finite());
        prop_assert!(position.y >= 0.5 - 2e-2, "ball sank to {}", position.y);
    }

    #[test]
    fn convenience_raycast_matches_raw_rapier_2d(
        origin in proptest::collection::vec(-20.0f32..20.0, 2),
        direction in proptest::collection::vec(-1.0f32..1.0, 2),
        max_distance in 0.1f32..50.0,
    ) {
        let mut world = Physics2::new(World2::default()).unwrap();
        world.insert(
            Body2Builder::fixed(),
            Collider2Builder::cuboid(10.0, 0.2).translation(Vector2::new(0.0, 0.0)),
        );
        world.insert(
            Body2Builder::fixed(),
            Collider2Builder::cuboid(0.2, 10.0).translation(Vector2::new(3.0, 0.0)),
        );
        world.step_fixed();
        let origin: [f32; 2] = origin.try_into().unwrap();
        let direction: [f32; 2] = direction.try_into().unwrap();
        let norm = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
        if norm <= f32::EPSILON {
            return Ok(());
        }
        let normalized = Vector2::new(direction[0] / norm, direction[1] / norm);
        let raw = world.cast_ray(
            &quasar_physics::rapier2d::geometry::Ray::new(
                Vector2::new(origin[0], origin[1]),
                normalized,
            ),
            max_distance,
            true,
            Default::default(),
        );
        let convenience = world.raycast(origin, direction, max_distance, true);
        match (raw, convenience) {
            (Some((handle, toi)), Some(hit)) => {
                prop_assert_eq!(hit.collider, handle);
                prop_assert!((hit.distance - toi).abs() < 1e-3);
            }
            (None, None) => {}
            other => prop_assert!(false, "ray mismatch: {:?}", other),
        }
    }

    #[test]
    fn convenience_ray_invalid_inputs_return_none(
        origin in proptest::collection::vec(-20.0f32..20.0, 3),
        direction in proptest::collection::vec(-1.0f32..1.0, 3),
    ) {
        let mut world = Physics3::new(World3::default()).unwrap();
        world.insert(
            Body3Builder::fixed(),
            Collider3Builder::cuboid(10.0, 0.2, 10.0),
        );
        world.step_fixed();
        let origin: [f32; 3] = origin.try_into().unwrap();
        let direction: [f32; 3] = direction.try_into().unwrap();
        prop_assert!(world.raycast(origin, direction, f32::NAN, true).is_none());
        prop_assert!(world.raycast(origin, direction, -1.0, true).is_none());
        prop_assert!(world.raycast(origin, [0.0; 3], 10.0, true).is_none());
        let mut invalid = origin;
        invalid[0] = f32::NAN;
        prop_assert!(world.raycast(invalid, direction, 10.0, true).is_none());
    }
}

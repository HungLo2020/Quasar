#![cfg(all(feature = "dim2", feature = "dim3", feature = "dynamics"))]
#[cfg(feature = "events")]
use quasar_physics::dim2::{ActiveEvents, CollisionEvent};
use quasar_physics::dim2::{PhysicsWorld as Physics2, StepConfig, WorldConfig as World2};
use quasar_physics::dim3::{PhysicsWorld as Physics3, WorldConfig as World3};
#[cfg(feature = "events")]
use quasar_physics::rapier2d::prelude::CollisionEventFlags;
use quasar_physics::rapier2d::prelude::{
    ColliderBuilder as Collider2Builder, RigidBodyBuilder as Body2Builder, Vector as Vector2,
};
use quasar_physics::rapier3d::prelude::{
    ColliderBuilder as Collider3Builder, RigidBodyBuilder as Body3Builder, Vector as Vector3,
};

#[test]
fn gravity_moves_a_ball_and_the_ground_stops_it() {
    let mut world = Physics2::new(World2::default()).unwrap();
    world.insert(
        Body2Builder::fixed(),
        Collider2Builder::cuboid(20.0, 0.5).translation(Vector2::new(0.0, -0.5)),
    );
    let (ball, _) = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(0.0, 5.0)),
        Collider2Builder::ball(0.5).restitution(0.4).friction(0.8),
    );
    for _ in 0..240 {
        world.step_fixed();
    }
    let position = world.bodies[ball].translation();
    assert!(position.y.is_finite());
    assert!((0.45..0.8).contains(&position.y), "ball y={}", position.y);
    assert!(world.bodies[ball].linvel().length() < 0.2);
    assert_eq!(world.fixed_steps(), 240);
    assert!((world.simulated_seconds() - 4.0).abs() < 1e-3);
}

#[test]
fn restitution_changes_bounce_and_friction_slows_sliding() {
    fn drop_height(restitution: f32) -> f32 {
        let mut world = Physics2::new(World2::default()).unwrap();
        world.insert(
            Body2Builder::fixed(),
            Collider2Builder::cuboid(20.0, 0.5).translation(Vector2::new(0.0, -0.5)),
        );
        let (ball, _) = world.insert(
            Body2Builder::dynamic().translation(Vector2::new(0.0, 5.0)),
            Collider2Builder::ball(0.5).restitution(restitution),
        );
        for _ in 0..90 {
            world.step_fixed();
        }
        world.bodies[ball].translation().y
    }
    let bouncy = drop_height(0.9);
    let dead = drop_height(0.0);
    assert!(bouncy > dead, "bouncy={bouncy} dead={dead}");

    let mut world = Physics2::new(World2::default()).unwrap();
    world.insert(
        Body2Builder::fixed(),
        Collider2Builder::cuboid(50.0, 0.5).translation(Vector2::new(0.0, -0.5)),
    );
    let (block, _) = world.insert(
        Body2Builder::dynamic()
            .translation(Vector2::new(0.0, 0.5))
            .linvel(Vector2::new(10.0, 0.0)),
        Collider2Builder::cuboid(0.5, 0.5).friction(1.0),
    );
    for _ in 0..120 {
        world.step_fixed();
    }
    assert!(world.bodies[block].linvel().x.abs() < 8.0);
}

#[test]
fn zero_gravity_keeps_a_body_at_rest() {
    let mut world = Physics2::new(World2::new([0.0, 0.0], StepConfig::default()).unwrap()).unwrap();
    let (body, _) = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(3.0, 4.0)),
        Collider2Builder::ball(0.5),
    );
    for _ in 0..120 {
        world.step_fixed();
    }
    let position = world.bodies[body].translation();
    assert!((position.x - 3.0).abs() < 1e-4);
    assert!((position.y - 4.0).abs() < 1e-4);
}

#[test]
fn advance_is_bounded_and_reports_saturation() {
    let config = World2::new([0.0, -9.81], StepConfig::new(1.0 / 60.0, 0.25, 2).unwrap()).unwrap();
    let mut world = Physics2::new(config).unwrap();
    let report = world.advance(1.0 / 30.0).unwrap();
    assert_eq!(report.steps, 2);
    assert!(!report.saturated);
    assert!((report.simulated_seconds - 2.0 / 60.0).abs() < 1e-6);

    let report = world.advance(0.25).unwrap();
    assert_eq!(report.steps, 2);
    assert!(report.saturated);
    assert_eq!(world.fixed_steps(), 4);

    assert!(world.advance(f32::NAN).is_err());
    assert!(world.advance(-1.0).is_err());
    assert!(world.set_gravity([f32::INFINITY, 0.0]).is_err());
    assert!(StepConfig::new(0.0, 0.25, 2).is_err());
    assert!(StepConfig::new(0.5, 0.25, 2).is_err());
    assert!(StepConfig::new(1.0 / 60.0, 0.25, 0).is_err());
}

#[test]
fn collision_filtering_prevents_unwanted_contact() {
    use quasar_physics::dim2::{Group, InteractionGroups, InteractionTestMode};

    let ground_groups =
        InteractionGroups::new(Group::GROUP_1, Group::GROUP_1, InteractionTestMode::And);
    let ball_groups =
        InteractionGroups::new(Group::GROUP_2, Group::GROUP_2, InteractionTestMode::And);
    let mut world = Physics2::new(World2::default()).unwrap();
    world.insert(
        Body2Builder::fixed(),
        Collider2Builder::cuboid(20.0, 0.5)
            .translation(Vector2::new(0.0, -0.5))
            .collision_groups(ground_groups),
    );
    let (ball, _) = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(0.0, 2.0)),
        Collider2Builder::ball(0.5).collision_groups(ball_groups),
    );
    for _ in 0..120 {
        world.step_fixed();
    }
    assert!(world.bodies[ball].translation().y < -2.0);

    let mut world = Physics2::new(World2::default()).unwrap();
    world.insert(
        Body2Builder::fixed(),
        Collider2Builder::cuboid(20.0, 0.5)
            .translation(Vector2::new(0.0, -0.5))
            .collision_groups(InteractionGroups::all()),
    );
    let (ball, _) = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(0.0, 2.0)),
        Collider2Builder::ball(0.5).collision_groups(InteractionGroups::all()),
    );
    for _ in 0..240 {
        world.step_fixed();
    }
    assert!(world.bodies[ball].translation().y > 0.4);
}

#[cfg(feature = "events")]
#[test]
fn sensor_events_are_collected_and_drained() {
    let mut world = Physics2::new(World2::default()).unwrap();
    let sensor = world.insert_collider(
        Collider2Builder::cuboid(2.0, 0.5)
            .translation(Vector2::new(0.0, 3.0))
            .sensor(true)
            .active_events(ActiveEvents::COLLISION_EVENTS),
        None,
    );
    let (_, ball) = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(0.0, 8.0)),
        Collider2Builder::ball(0.5).active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let mut started = false;
    for _ in 0..120 {
        world.step_fixed();
        for event in world.drain_events().collisions {
            if let CollisionEvent::Started(left, right, flags) = event
                && ((left == sensor && right == ball) || (right == sensor && left == ball))
            {
                assert!(flags.contains(CollisionEventFlags::SENSOR));
                started = true;
            }
        }
    }
    assert!(started, "sensor started event was not emitted");
    assert!(world.drain_events().collisions.is_empty());
}

#[cfg(feature = "events")]
#[test]
fn contact_force_events_are_available() {
    let mut world = Physics2::new(World2::default()).unwrap();
    world.insert(
        Body2Builder::fixed(),
        Collider2Builder::cuboid(10.0, 0.5).translation(Vector2::new(0.0, -0.5)),
    );
    let (body, ball) = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(0.0, 4.0)),
        Collider2Builder::ball(0.5)
            .active_events(ActiveEvents::CONTACT_FORCE_EVENTS)
            .contact_force_event_threshold(0.0),
    );
    let mut force_seen = false;
    for _ in 0..180 {
        world.step_fixed();
        if !world.drain_events().contact_forces.is_empty() {
            force_seen = true;
        }
    }
    assert!(force_seen);
    assert_eq!(world.colliders[ball].parent(), Some(body));
}

#[test]
fn deterministic_fixed_steps_repeat_exactly_in_2d() {
    fn simulate() -> Vec<u32> {
        let mut world = Physics2::new(World2::default()).unwrap();
        world.insert(
            Body2Builder::fixed(),
            Collider2Builder::cuboid(20.0, 0.5).translation(Vector2::new(0.0, -0.5)),
        );
        for index in 0..32 {
            world.insert(
                Body2Builder::dynamic()
                    .translation(Vector2::new(
                        (index % 8) as f32 - 4.0,
                        2.0 + index as f32 * 0.2,
                    ))
                    .linvel(Vector2::new(0.1 * index as f32, 0.0)),
                Collider2Builder::ball(0.25).restitution(0.6),
            );
        }
        for _ in 0..180 {
            world.step_fixed();
        }
        world
            .bodies
            .iter()
            .flat_map(|(_, body)| {
                let position = body.translation();
                [position.x.to_bits(), position.y.to_bits()]
            })
            .collect()
    }
    assert_eq!(simulate(), simulate());
}

#[test]
fn deterministic_fixed_steps_repeat_exactly_in_3d() {
    fn simulate() -> Vec<u32> {
        let mut world = Physics3::new(World3::default()).unwrap();
        world.insert(
            Body3Builder::fixed(),
            Collider3Builder::cuboid(20.0, 0.5, 20.0).translation(Vector3::new(0.0, -0.5, 0.0)),
        );
        for index in 0..32 {
            world.insert(
                Body3Builder::dynamic().translation(Vector3::new(
                    (index % 8) as f32 - 4.0,
                    2.0 + index as f32 * 0.2,
                    (index / 8) as f32 - 2.0,
                )),
                Collider3Builder::cuboid(0.2, 0.2, 0.2).restitution(0.3),
            );
        }
        for _ in 0..180 {
            world.step_fixed();
        }
        world
            .bodies
            .iter()
            .flat_map(|(_, body)| {
                let position = body.translation();
                [
                    position.x.to_bits(),
                    position.y.to_bits(),
                    position.z.to_bits(),
                ]
            })
            .collect()
    }
    assert_eq!(simulate(), simulate());
}

#[test]
fn removal_and_reclamation_are_explicit() {
    let mut world = Physics3::new(World3::default()).unwrap();
    let body = world.insert(
        Body3Builder::dynamic().translation(Vector3::new(0.0, 3.0, 0.0)),
        Collider3Builder::ball(0.5),
    );
    assert_eq!(world.bodies.len(), 1);
    assert!(world.remove_body(body.0).is_some());
    assert_eq!(world.bodies.len(), 0);
    assert_eq!(world.colliders.len(), 0);
    assert!(world.remove_body(body.0).is_none());
}

#[test]
fn ball_3d_rests_on_the_ground_and_raycasts_hit() {
    let mut world = Physics3::new(World3::default()).unwrap();
    let ground = world.insert(
        Body3Builder::fixed(),
        Collider3Builder::cuboid(20.0, 0.5, 20.0).translation(Vector3::new(0.0, -0.5, 0.0)),
    );
    let (ball, _) = world.insert(
        Body3Builder::dynamic().translation(Vector3::new(0.0, 6.0, 0.0)),
        Collider3Builder::ball(0.5).restitution(0.5),
    );
    for _ in 0..300 {
        world.step_fixed();
    }
    let y = world.bodies[ball].translation().y;
    assert!((0.4..1.2).contains(&y), "ball y={y}");
    let hit = world
        .raycast([5.0, 10.0, 0.0], [0.0, -1.0, 0.0], 30.0, true)
        .unwrap();
    assert_eq!(hit.collider, ground.1);
    assert!((hit.distance - 10.0).abs() < 1e-3);
    assert!(hit.normal[1] > 0.9);
}

#[test]
fn stress_many_dynamic_bodies_remain_finite() {
    let mut world = Physics2::new(World2::default()).unwrap();
    world.insert(
        Body2Builder::fixed(),
        Collider2Builder::cuboid(30.0, 0.5).translation(Vector2::new(0.0, -0.5)),
    );
    for index in 0..300 {
        world.insert(
            Body2Builder::dynamic().translation(Vector2::new(
                (index % 30) as f32 - 15.0,
                1.0 + (index / 30) as f32 * 1.2,
            )),
            Collider2Builder::ball(0.35).restitution(0.3).friction(0.4),
        );
    }
    for _ in 0..120 {
        world.step_fixed();
    }
    for (_, body) in world.bodies.iter() {
        let position = body.translation();
        assert!(position.x.is_finite() && position.y.is_finite());
        let velocity = body.linvel();
        assert!(velocity.x.is_finite() && velocity.y.is_finite());
    }
    assert_eq!(world.bodies.len(), 301);
}

#[test]
fn repeated_body_and_collider_churn_is_reclaimed() {
    let mut world = Physics3::new(World3::default()).unwrap();
    for index in 0..500 {
        let (body, collider) = world.insert(
            Body3Builder::dynamic().translation(Vector3::new(index as f32 * 0.01, 2.0, 0.0)),
            Collider3Builder::ball(0.2),
        );
        world.step_fixed();
        assert!(world.remove_body(body).is_some());
        assert!(world.bodies.get(body).is_none());
        assert!(world.colliders.get(collider).is_none());
    }
    assert_eq!(world.bodies.len(), 0);
    assert_eq!(world.colliders.len(), 0);
    assert_eq!(world.fixed_steps(), 500);
    assert!(world.quarantine().is_empty());
}

#[test]
fn non_finite_user_state_is_quarantined_not_propagated() {
    let mut world = Physics2::new(World2::default()).unwrap();
    let body = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(0.0, 2.0)),
        Collider2Builder::ball(0.5),
    );
    world.bodies[body.0].set_translation(Vector2::new(f32::NAN, 2.0), true);
    world.step_fixed();
    assert!(!world.quarantine().is_empty());
    assert!(world.quarantine().bodies().contains(&body.0));
    assert!(!world.bodies[body.0].is_enabled());
    let healthy = world.insert(
        Body2Builder::dynamic().translation(Vector2::new(5.0, 2.0)),
        Collider2Builder::ball(0.5),
    );
    for _ in 0..30 {
        world.step_fixed();
    }
    let position = world.bodies[healthy.0].translation();
    assert!(position.x.is_finite() && position.y.is_finite());
}

#[test]
fn exact_fixed_step_accumulation_does_not_drift() {
    let config = World2::new([0.0, -9.81], StepConfig::new(1.0 / 60.0, 0.25, 8).unwrap()).unwrap();
    let mut world = Physics2::new(config).unwrap();
    for tick in 1..=600 {
        let report = world.advance(1.0 / 60.0).unwrap();
        assert_eq!(report.steps, 1, "tick {tick}");
        assert!(!report.saturated);
    }
    assert_eq!(world.fixed_steps(), 600);
    let before = world.fixed_steps();
    let report = world.advance(1.0 / 30.0).unwrap();
    assert_eq!(report.steps, 2);
    assert_eq!(world.fixed_steps(), before + 2);
}

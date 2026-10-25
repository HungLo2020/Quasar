#![cfg(all(feature = "dim2", feature = "dim3"))]
use quasar_physics::dim2::{CollisionWorld as Collision2, Group, InteractionGroups};
use quasar_physics::dim3::CollisionWorld as Collision3;
use quasar_physics::rapier2d::prelude::{ColliderBuilder as Collider2Builder, Group as Group2};
use quasar_physics::rapier3d::prelude::{ColliderBuilder as Collider3Builder, Group as Group3};

#[test]
fn collision_only_2d_queries_work_without_rigid_bodies() {
    let mut world = Collision2::new();
    let ground = world.add_collider(
        Collider2Builder::cuboid(10.0, 0.5)
            .translation(quasar_physics::rapier2d::math::Vector::new(0.0, -0.5)),
    );
    let wall = world.add_collider(
        Collider2Builder::cuboid(0.5, 4.0)
            .translation(quasar_physics::rapier2d::math::Vector::new(4.0, 3.5)),
    );
    assert_eq!(world.bodies.len(), 0);
    world.update();
    assert_eq!(world.updates(), 1);

    let hit = world.raycast([0.0, 5.0], [0.0, -1.0], 20.0, true).unwrap();
    assert_eq!(hit.collider, ground);
    assert!((hit.distance - 5.0).abs() < 1e-4);
    assert!((hit.normal[1] - 1.0).abs() < 1e-4);
    assert!((hit.point[1] - 0.0).abs() < 1e-4);

    let hit = world.raycast([0.0, 3.0], [1.0, 0.0], 20.0, true).unwrap();
    assert_eq!(hit.collider, wall);
    assert!((hit.distance - 3.5).abs() < 1e-4);

    let query = world.intersect_point(
        quasar_physics::rapier2d::math::Vector::new(0.0, -0.25),
        Default::default(),
    );
    assert_eq!(query.count(), 1);
    let shape = quasar_physics::rapier2d::geometry::SharedShape::ball(2.0);
    let hits = world
        .intersect_shape(
            quasar_physics::rapier2d::math::Pose::translation(0.0, 0.5),
            &*shape,
            Default::default(),
        )
        .count();
    assert!(hits >= 1);

    assert!(world.raycast([0.0, 5.0], [0.0, 0.0], 20.0, true).is_none());
    assert!(
        world
            .raycast([0.0, 5.0], [0.0, -1.0], f32::NAN, true)
            .is_none()
    );
}

#[test]
fn collision_only_2d_respects_collision_groups() {
    let mut world = Collision2::new();
    let groups_obstacle =
        InteractionGroups::new(Group::GROUP_1, Group::GROUP_2, Default::default());
    world.add_collider(
        Collider2Builder::cuboid(5.0, 0.5)
            .translation(quasar_physics::rapier2d::math::Vector::new(0.0, 0.0))
            .collision_groups(groups_obstacle),
    );
    world.update();
    let passing = Default::default();
    assert!(world.raycast([0.0, 5.0], [0.0, -1.0], 10.0, true).is_some());
    assert!(
        world
            .intersect_shape(
                quasar_physics::rapier2d::math::Pose::translation(0.0, 5.0),
                &*quasar_physics::rapier2d::geometry::SharedShape::ball(1.0),
                quasar_physics::rapier2d::pipeline::QueryFilter::default().groups(passing),
            )
            .count()
            <= 2
    );
    let wrong_groups = InteractionGroups::new(Group::GROUP_3, Group::GROUP_3, Default::default());
    let excluded = world
        .intersect_shape(
            quasar_physics::rapier2d::math::Pose::translation(0.0, 0.0),
            &*quasar_physics::rapier2d::geometry::SharedShape::ball(1.0),
            quasar_physics::rapier2d::pipeline::QueryFilter::default().groups(wrong_groups),
        )
        .count();
    assert_eq!(excluded, 0);
}

#[test]
fn collision_only_3d_queries_work_without_rigid_bodies() {
    let mut world = Collision3::new();
    let ground = world.add_collider(
        Collider3Builder::cuboid(10.0, 0.5, 10.0)
            .translation(quasar_physics::rapier3d::math::Vector::new(0.0, -0.5, 0.0)),
    );
    world.update();
    assert_eq!(world.bodies.len(), 0);
    let hit = world
        .raycast([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], 20.0, true)
        .unwrap();
    assert_eq!(hit.collider, ground);
    assert!((hit.distance - 5.0).abs() < 1e-4);
    assert!((hit.normal[1] - 1.0).abs() < 1e-4);

    let shape = quasar_physics::rapier3d::geometry::SharedShape::ball(2.0);
    let hits = world
        .intersect_shape(
            quasar_physics::rapier3d::math::Pose::translation(0.0, 0.5, 0.0),
            &*shape,
            Default::default(),
        )
        .count();
    assert!(hits >= 1);

    let shape = quasar_physics::rapier3d::geometry::SharedShape::ball(0.5);
    let cast = world.cast_shape(
        &quasar_physics::rapier3d::math::Pose::translation(0.0, 5.0, 0.0),
        quasar_physics::rapier3d::math::Vector::new(0.0, -1.0, 0.0),
        &*shape,
        quasar_physics::rapier3d::parry::query::ShapeCastOptions::with_max_time_of_impact(10.0),
        Default::default(),
    );
    assert!(cast.is_some());

    let projection = world.project_point(
        quasar_physics::rapier3d::math::Vector::new(0.0, 2.0, 0.0),
        10.0,
        true,
        Default::default(),
    );
    assert_eq!(projection.unwrap().0, ground);
}

#[test]
fn removing_a_collider_invalidates_its_handle_and_queries() {
    let mut world = Collision2::new();
    let handle = world.add_collider(
        Collider2Builder::cuboid(1.0, 1.0)
            .translation(quasar_physics::rapier2d::math::Vector::new(0.0, 0.0)),
    );
    world.update();
    assert!(world.colliders.get(handle).is_some());
    assert!(world.raycast([0.0, 5.0], [0.0, -1.0], 10.0, true).is_some());
    assert!(world.remove_collider(handle).is_some());
    world.update();
    assert!(world.colliders.get(handle).is_none());
    assert!(world.raycast([0.0, 5.0], [0.0, -1.0], 10.0, true).is_none());
}

#[cfg(feature = "spatial")]
#[test]
fn spatial_index_matches_collider_aabbs_and_keys_round_trip() {
    use quasar_physics::dim2::{collider_from_key, collider_key};

    let mut world = Collision2::new();
    let first = world.add_collider(
        Collider2Builder::cuboid(1.0, 1.0)
            .translation(quasar_physics::rapier2d::math::Vector::new(0.0, 0.0)),
    );
    let second = world.add_collider(
        Collider2Builder::cuboid(1.0, 1.0)
            .translation(quasar_physics::rapier2d::math::Vector::new(10.0, 0.0)),
    );
    world.update();
    let index = world.spatial_index().unwrap();
    assert_eq!(index.len(), 2);
    assert_eq!(
        index.query(quasar_spatial::Aabb::point([10.0, 0.0]).unwrap()),
        vec![collider_key(second)]
    );
    assert!(
        index
            .query(quasar_spatial::Aabb::point([5.0, 0.0]).unwrap())
            .is_empty()
    );
    assert_eq!(collider_from_key(collider_key(first)), first);
    assert_eq!(collider_from_key(collider_key(second)), second);

    let bounds = quasar_physics::dim2::collider_bounds(&world.colliders[first]).unwrap();
    assert!((bounds.min[0] + 1.0).abs() < 1e-4);
    assert!((bounds.max[1] - 1.0).abs() < 1e-4);
    let rapier_aabb = quasar_physics::dim2::aabb_to_rapier(bounds);
    let round_trip = quasar_physics::dim2::aabb_from_rapier(rapier_aabb).unwrap();
    assert_eq!(round_trip, bounds);
}

#[test]
fn group_constants_are_available_from_both_dimensions() {
    assert_eq!(Group2::GROUP_1.bits(), Group3::GROUP_1.bits());
    assert_eq!(Group::GROUP_1.bits(), Group2::GROUP_1.bits());
}

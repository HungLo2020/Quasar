#![cfg(feature = "dim3")]
use quasar_physics::{
    dim3::*,
    rapier3d::{pipeline::QueryFilter, prelude::ColliderBuilder},
};
fn floor(world: &mut CollisionWorld) {
    world.add_collider(ColliderBuilder::cuboid(20., 0.25, 20.).translation([0., -0.25, 0.].into()));
}
#[test]
fn explicit_shapes_validate_and_nested_compounds_flatten_without_panics() {
    assert!(
        Shape3::CylinderY {
            radius: f32::NAN,
            half_height: 1.
        }
        .shared()
        .is_err()
    );
    assert!(Shape3::Cuboid { half: [1., 0., 1.] }.shared().is_err());
    assert!(
        Shape3::CapsuleY {
            radius: 1.,
            half_segment: -0.1
        }
        .shared()
        .is_err()
    );
    assert!(Shape3::Compound(vec![]).shared().is_err());
    assert!(
        RigidPose3 {
            rotation: [0.; 4],
            ..RigidPose3::IDENTITY
        }
        .rapier()
        .is_err()
    );
    assert!(
        RigidPose3::translated([f32::INFINITY, 0., 0.])
            .matrix()
            .is_err()
    );
    let mut shape = Shape3::Cuboid { half: [0.5; 3] };
    for _ in 0..3 {
        shape = Shape3::Compound(vec![PlacedShape3 {
            pose: RigidPose3::translated([1., 0., 0.]),
            shape,
        }]);
    }
    let mut world = CollisionWorld::new();
    world.add_collider(
        PlacedShape3 {
            pose: RigidPose3::IDENTITY,
            shape: shape.clone(),
        }
        .collider()
        .unwrap(),
    );
    world.update();
    assert_eq!(
        world
            .raycast([0., 0., 0.], [1., 0., 0.], 10., true)
            .unwrap()
            .distance,
        2.5
    );
    for _ in 0..7 {
        shape = Shape3::Compound(vec![PlacedShape3 {
            pose: RigidPose3::IDENTITY,
            shape,
        }]);
    }
    assert!(shape.shared().is_err());
    assert!(
        Shape3::Compound(vec![
            PlacedShape3 {
                pose: RigidPose3::IDENTITY,
                shape: Shape3::Cuboid { half: [1.; 3] }
            };
            256
        ])
        .shared()
        .is_err()
    );
}
#[test]
fn bounds_fit_matches_rigid_rotated_collider_and_rejects_elliptical_round_fits() {
    let source = Bounds3::new([-2., 0., -1.], [2., 4., 1.]).unwrap();
    let pose = RigidPose3::yaw([4., 2., -3.], 0.7);
    let matrix = source.fit_matrix(pose, [1., 2., 0.5]).unwrap();
    let point = |p: [f32; 3]| {
        std::array::from_fn::<_, 3, _>(|r| {
            matrix[3][r] + (0..3).map(|c| matrix[c][r] * p[c]).sum::<f32>()
        })
    };
    let a = point(source.center());
    assert!(
        a.into_iter()
            .zip(pose.translation)
            .all(|(a, b)| (a - b).abs() < 1e-5)
    );
    let shape = PlacedShape3 {
        pose,
        shape: Shape3::Cuboid {
            half: [1., 2., 0.5],
        },
    }
    .collider()
    .unwrap();
    let bounds = shape.compute_aabb();
    for bits in 0..8 {
        let p = point(std::array::from_fn(|i| {
            if bits & (1 << i) == 0 {
                source.min[i]
            } else {
                source.max[i]
            }
        }));
        for (axis, value) in p.into_iter().enumerate() {
            assert!(value >= bounds.mins[axis] - 1e-5 && value <= bounds.maxs[axis] + 1e-5);
        }
    }
    assert!(source.collider(PrimitiveFit3::CylinderY).is_err());
    let source = Bounds3::new([-1., 0., -1.], [1., 4., 1.]).unwrap();
    assert!(source.collider(PrimitiveFit3::CapsuleY).is_ok());
    assert!(source.fit_matrix(pose, [1., -1., 1.]).is_err());
    assert!(Bounds3::new([0.; 3], [1., 0., 1.]).is_err());
}
#[test]
fn cylinder_rays_miss_empty_corners_and_hit_grazing_surface_precisely() {
    let mut world = CollisionWorld::new();
    world.add_collider(
        PlacedShape3 {
            pose: RigidPose3::translated([0., 1., 0.]),
            shape: Shape3::CylinderY {
                radius: 1.,
                half_height: 1.,
            },
        }
        .collider()
        .unwrap(),
    );
    world.update();
    assert!(
        world
            .raycast([1.01, 1., 3.], [0., 0., -1.], 6., true)
            .is_none()
    );
    let hit = world
        .raycast([0.99, 1., 3.], [0., 0., -1.], 6., true)
        .unwrap();
    assert!((hit.distance - (3. - (1.0f32 - 0.99 * 0.99).sqrt())).abs() < 0.002);
    assert!(hit.normal[0] > 0.98);
    assert_eq!(
        world
            .raycast([0., 1., 0.], [0., 0., 1.], 2., true)
            .unwrap()
            .distance,
        0.
    );
    let capsule = CapsuleController3::new(1.85, 0.32, 0.005).unwrap();
    assert!(!capsule.overlaps(&world, [1., 0.01, 1.], QueryFilter::default()));
    assert!(capsule.overlaps(&world, [0., 0.01, 1.], QueryFilter::default()));
}
#[test]
fn capsule_sweeps_stop_tunneling_slide_around_round_obstacles_and_land() {
    let mut world = CollisionWorld::new();
    floor(&mut world);
    world.add_collider(
        PlacedShape3 {
            pose: RigidPose3::translated([0., 1., 0.]),
            shape: Shape3::CylinderY {
                radius: 1.,
                half_height: 1.,
            },
        }
        .collider()
        .unwrap(),
    );
    world.update();
    let capsule = CapsuleController3::new(1.85, 0.32, 0.005).unwrap();
    let hit = capsule
        .move_feet(
            &world,
            [-10., 0.01, 0.],
            [20., 0., 0.],
            1. / 60.,
            QueryFilter::default(),
        )
        .unwrap();
    assert!(hit.feet[0] < -1.3);
    assert!(hit.collisions > 0);
    let mut p = [-2., 0.01, -1.];
    let mut last = p;
    for _ in 0..120 {
        let moved = capsule
            .move_feet(
                &world,
                p,
                [0.04, -0.02, 0.],
                1. / 60.,
                QueryFilter::default(),
            )
            .unwrap();
        p = moved.feet;
        assert!(
            !capsule.overlaps(&world, p, QueryFilter::default()),
            "overlap after curved movement at {p:?}"
        );
        assert!((p[2] - last[2]).abs() < 0.05);
        last = p;
    }
    assert!(p[0] > 1., "must make progress around cylinder: {p:?}");
    let landed = capsule
        .move_feet(
            &world,
            [4., 4., 0.],
            [0., -10., 0.],
            1. / 60.,
            QueryFilter::default(),
        )
        .unwrap();
    assert!(landed.grounded);
    assert!((landed.feet[1] - 0.005).abs() < 0.002);
    assert!(
        capsule
            .move_feet(
                &world,
                p,
                [f32::NAN, 0., 0.],
                1. / 60.,
                QueryFilter::default()
            )
            .is_err()
    );
    assert!(CapsuleController3::new(0.5, 1., 0.01).is_err());
}
#[test]
fn capsule_hit_shape_has_no_rectangular_head_corners() {
    let capsule = CapsuleController3::new(1.94, 0.34, 0.005).unwrap();
    assert!(
        capsule
            .ray_distance([0.; 3], [0.33, 1.85, 3.], [0., 0., -1.], 10.)
            .is_none()
    );
    assert!(
        capsule
            .ray_distance([0.; 3], [0.33, 1., 3.], [0., 0., -1.], 10.)
            .is_some()
    );
    assert!(
        capsule
            .ray_distance([0.; 3], [0., 1.65, 3.], [0., 0., -1.], 10.)
            .is_some()
    );
}

#[test]
fn capsule_sweeps_respect_overhangs_during_upward_movement() {
    let mut world = CollisionWorld::new();
    floor(&mut world);
    world.add_collider(ColliderBuilder::cuboid(2., 0.2, 2.).translation([0., 2.5, 0.].into()));
    world.update();
    let actor = CapsuleController3::new(1.85, 0.32, 0.005).unwrap();
    let moved = actor
        .move_feet(
            &world,
            [0., 0.01, 0.],
            [0., 10., 0.],
            1. / 60.,
            QueryFilter::default(),
        )
        .unwrap();
    assert!(moved.feet[1] < 0.45);
    assert!(moved.collisions > 0);
    assert!(!actor.overlaps(&world, moved.feet, QueryFilter::default()));
}

#[test]
fn cylinder_side_rays_match_analytic_circle_across_the_silhouette() {
    let mut world = CollisionWorld::new();
    world.add_collider(
        PlacedShape3 {
            pose: RigidPose3::translated([2., 1., -4.]),
            shape: Shape3::CylinderY {
                radius: 0.6,
                half_height: 1.,
            },
        }
        .collider()
        .unwrap(),
    );
    world.update();
    for i in 0..513 {
        let x = -0.8 + i as f32 / 512. * 1.6;
        let hit = world.raycast([2. + x, 1., -1.], [0., 0., -1.], 6., true);
        if x.abs() > 0.60001 {
            assert!(hit.is_none(), "x={x}");
        } else if x.abs() < 0.59999 {
            let expected = 3. - (0.6f32.powi(2) - x.powi(2)).sqrt();
            assert!((hit.unwrap().distance - expected).abs() < 0.002, "x={x}");
        }
    }
}

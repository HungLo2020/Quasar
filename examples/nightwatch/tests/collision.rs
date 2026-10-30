use nightwatch::{
    props::{ModelId, PLAYER_HEIGHT, PLAYER_RADIUS, Prop, SKIN},
    world::{EYE_HEIGHT, Input, World},
};
use quasar_physics::{dim3::*, rapier3d::pipeline::QueryFilter};
fn floor() -> Prop {
    Prop::cuboid(1, "floor", [0., -0.25, 0.], [15., 0.25, 15.], [0.1; 3])
}
#[test]
fn default_barrel_corners_are_free_and_rays_use_the_round_hull() {
    let w = World::new(1);
    let barrel = w.props().iter().find(|p| p.id == 200).unwrap();
    let Shape3::CylinderY { radius, .. } = barrel.collider.as_ref().unwrap() else {
        panic!("barrel must use a cylinder")
    };
    let [x, y, z] = barrel.pose.translation;
    let actor = CapsuleController3::new(PLAYER_HEIGHT, PLAYER_RADIUS, SKIN).unwrap();
    assert!(!actor.overlaps(
        w.collision(),
        [x + 0.65, 0.01, z + 0.65],
        QueryFilter::default()
    ));
    assert!(actor.overlaps(
        w.collision(),
        [x + 0.3, 0.01, z + 0.3],
        QueryFilter::default()
    ));
    assert!(
        w.raycast_world([x + radius + 0.01, y, z - 2.], [0., 0., 1.], 4.)
            .is_none()
    );
    let hit = w
        .raycast_world([x + radius - 0.02, y, z - 2.], [0., 0., 1.], 4.)
        .unwrap();
    assert_eq!(w.collision().colliders[hit.collider].user_data, 200);
    assert!(hit.distance < 2.);
}
#[test]
fn workbench_is_a_compound_with_leg_gaps_and_a_solid_top() {
    let mut w = World::new(2);
    let bench = w.props().iter().find(|p| p.id == 104).unwrap().clone();
    assert!(matches!(bench.collider, Some(Shape3::Compound(_))));
    w.set_props(vec![floor(), bench.clone()]).unwrap();
    let [x, _, z] = bench.pose.translation;
    assert!(
        w.raycast_world([x, 0.5, z - 3.], [0., 0., 1.], 6.)
            .is_none()
    );
    assert!(
        w.raycast_world([x, 1.1, z - 3.], [0., 0., 1.], 6.)
            .is_some()
    );
    let leg_x = x + 0.10695 / 0.14782624 * 1.6;
    assert!(
        w.raycast_world([leg_x, 0.5, z - 3.], [0., 0., 1.], 6.)
            .is_some()
    );
}
#[test]
fn prop_replacement_is_atomic_and_identity_does_not_select_a_model() {
    let mut w = World::new(3);
    let before = w.state_hash();
    let original = w.props().to_vec();
    let mut bad = original.clone();
    bad.push(bad[0].clone());
    assert!(w.set_props(bad).is_err());
    assert_eq!(w.state_hash(), before);
    let mut bad = original.clone();
    bad[1].collider = Some(Shape3::Cuboid { half: [0.; 3] });
    assert!(w.set_props(bad).is_err());
    assert_eq!(w.props(), original.as_slice());
    let mut reordered = original.clone();
    reordered.reverse();
    let assignments: Vec<_> = original
        .iter()
        .map(|p| (p.id, p.visual.clone(), p.pose, p.collider.clone()))
        .collect();
    w.set_props(reordered).unwrap();
    for (id, visual, pose, shape) in assignments {
        let p = w.props().iter().find(|p| p.id == id).unwrap();
        assert_eq!(p.visual, visual);
        assert_eq!(p.pose, pose);
        assert_eq!(p.collider, shape);
    }
}
#[test]
fn zombies_cannot_attack_through_a_thin_wall() {
    let mut w = World::new(4);
    let mut zombie = w.zombies[0].clone();
    zombie.position = [-0.45, -2.];
    zombie.feet_y = 0.01;
    w.zombies = vec![zombie];
    w.player = [0.45, EYE_HEIGHT + 0.01, -2.];
    w.set_props(vec![
        floor(),
        Prop::cuboid(2, "thin wall", [0., 1., -2.], [0.025, 1., 1.], [1.; 3]),
    ])
    .unwrap();
    let health = w.health;
    w.update(Input::default());
    assert_eq!(w.health, health);
}
#[test]
fn jump_lands_on_real_surfaces_and_oblique_movement_does_not_tunnel() {
    let mut w = World::new(5);
    w.zombies.clear();
    w.player = [0., EYE_HEIGHT + 0.01, 3.];
    w.set_props(vec![
        floor(),
        Prop::cuboid(2, "test crate", [0., 0.5, 0.], [1., 0.5, 1.], [1.; 3]),
    ])
    .unwrap();
    w.update(Input {
        jump: true,
        ..Default::default()
    });
    let high = w.player[1];
    for _ in 0..100 {
        w.update(Input::default());
    }
    assert!(high > EYE_HEIGHT + 0.01);
    assert!((w.player[1] - EYE_HEIGHT - SKIN).abs() < 0.003);
    for _ in 0..120 {
        w.update(Input {
            forward: 1.,
            sprint: true,
            ..Default::default()
        });
    }
    assert!(w.player[2] > 1. + PLAYER_RADIUS - 0.01);
}
#[test]
fn deterministic_replay_includes_shared_prop_geometry() {
    let (mut a, mut b) = (World::new(42), World::new(42));
    for i in 0..1800 {
        let input = Input {
            forward: if i % 180 < 60 { 1. } else { 0. },
            strafe: if i % 240 < 90 { 0.6 } else { -0.4 },
            look: [0.7, 0.],
            jump: i % 120 == 0,
            fire: i % 3 == 0,
            reload: i % 90 == 0,
            ..Default::default()
        };
        a.update(input);
        b.update(input);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {i}");
    }
    let before = a.state_hash();
    let mut props = a.props().to_vec();
    props[1].pose.translation[0] += 0.1;
    a.set_props(props).unwrap();
    assert_ne!(a.state_hash(), before);
}
#[cfg(feature = "graphics")]
#[test]
fn shipped_model_fits_and_authored_colliders_have_matching_poses_and_envelopes() {
    use quasar_assets::{AssetServer, GltfLoader};
    let mut server = AssetServer::new();
    server
        .mount_file("", concat!(env!("CARGO_MANIFEST_DIR"), "/assets"))
        .unwrap();
    server.register(GltfLoader {
        allow_animation_default_pose: true,
        ..Default::default()
    });
    use nightwatch::props::{self, PropVisual};
    for prop in props::courtyard() {
        let PropVisual::Model { asset, half } = prop.visual else {
            continue;
        };
        let model = server.load::<GltfLoader>(asset.file()).unwrap();
        let bounds = if asset == ModelId::Workbench {
            model_bounds_selected(&model, |i| i.name == "workbench").unwrap()
        } else {
            model_bounds(&model).unwrap()
        };
        let matrix = bounds.fit_matrix(prop.pose, half).unwrap();
        let transform = |p: [f32; 3]| {
            std::array::from_fn::<_, 3, _>(|row| {
                matrix[3][row] + (0..3).map(|col| matrix[col][row] * p[col]).sum::<f32>()
            })
        };
        for (a, b) in transform(bounds.center())
            .into_iter()
            .zip(prop.pose.translation)
        {
            assert!((a - b).abs() < 1e-5);
        }
        let collider = prop.placed_collider().unwrap().collider().unwrap();
        let aabb = collider.compute_aabb();
        if asset == ModelId::Barrel {
            let source = model_cylinder_y(&model, |_| true).unwrap();
            let Shape3::CylinderY {
                radius,
                half_height,
            } = source.shape
            else {
                unreachable!()
            };
            let Shape3::CylinderY {
                radius: world_radius,
                half_height: world_half_height,
            } = prop.collider.clone().unwrap()
            else {
                unreachable!()
            };
            let scale = half[1] / half_height;
            assert!((radius * scale - world_radius).abs() < 1e-5);
            assert!((half_height * scale - world_half_height).abs() < 1e-5);
        }
        if asset == ModelId::Workbench {
            assert!((bounds.min[0] + 0.14782624).abs() < 1e-6);
            assert!((bounds.max[1] - 0.23655).abs() < 1e-6);
        }
        // Imported structural vertices are enclosed; loose tools/paper are explicitly decorative.
        for instance in model
            .instances
            .iter()
            .filter(|i| asset != ModelId::Workbench || i.name == "workbench")
        {
            for v in &model.primitives[instance.primitive].vertices {
                let local: [f32; 3] = std::array::from_fn(|row| {
                    instance.transform[3][row]
                        + (0..3)
                            .map(|col| instance.transform[col][row] * v.position[col])
                            .sum::<f32>()
                });
                let p = transform(local);
                for (axis, value) in p.into_iter().enumerate() {
                    assert!(
                        value >= aabb.mins[axis] - 1e-4 && value <= aabb.maxs[axis] + 1e-4,
                        "asset={asset:?}, point={p:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn zombies_route_around_real_cylinders_and_do_not_clip_corners() {
    let mut w = World::new(10);
    let mut zombie = w.zombies[0].clone();
    zombie.position = [-3., 0.];
    zombie.feet_y = 0.01;
    w.zombies = vec![zombie];
    w.player = [3., EYE_HEIGHT + 0.01, 0.];
    let radius = 1.;
    let visual_radius = radius * 0.11825 / 0.13364123;
    w.set_props(vec![
        floor(),
        Prop::model(
            2,
            "navigation barrel",
            [0., 1., 0.],
            [visual_radius, 1., visual_radius],
            ModelId::Barrel,
            Shape3::CylinderY {
                radius,
                half_height: 1.,
            },
        ),
    ])
    .unwrap();
    let actor = CapsuleController3::new(
        nightwatch::props::ZOMBIE_HEIGHT,
        nightwatch::props::ZOMBIE_RADIUS,
        SKIN,
    )
    .unwrap();
    let mut detour = 0.0f32;
    for _ in 0..900 {
        w.health = 100.;
        w.update(Input::default());
        let z = &w.zombies[0];
        detour = detour.max(z.position[1].abs());
        assert!(!actor.overlaps(
            w.collision(),
            [z.position[0], z.feet_y, z.position[1]],
            QueryFilter::default()
        ));
    }
    let z = &w.zombies[0];
    assert!(
        detour > 1.2,
        "must detour around the curved hull, max z={detour}"
    );
    assert!(
        (z.position[0] - w.player[0]).hypot(z.position[1] - w.player[2]) < 1.1,
        "must reach player: {:?}",
        z.position
    );
}
#[test]
fn bullets_cross_former_box_corners_and_hit_a_zombie_behind_them() {
    let mut w = World::new(11);
    let mut zombie = w.zombies[0].clone();
    let d = std::f32::consts::FRAC_1_SQRT_2;
    w.player = [0.9, EYE_HEIGHT + 0.01, 1.2];
    w.yaw = std::f32::consts::FRAC_PI_4;
    zombie.position = [0.9 + d * 3., 1.2 - d * 3.];
    zombie.feet_y = 0.01;
    w.zombies = vec![zombie];
    let radius = 1.;
    let visual_radius = radius * 0.11825 / 0.13364123;
    w.set_props(vec![
        floor(),
        Prop::model(
            2,
            "corner barrel",
            [0., 1., 0.],
            [visual_radius, 1., visual_radius],
            ModelId::Barrel,
            Shape3::CylinderY {
                radius,
                half_height: 1.,
            },
        ),
    ])
    .unwrap();
    w.update(Input {
        fire: true,
        ..Default::default()
    });
    assert_eq!(w.kills, 1);
    assert_eq!(w.shots_fired, 1);
}

#[test]
fn jumping_onto_a_low_crate_lands_on_its_top_instead_of_falling_through() {
    let mut w = World::new(12);
    w.zombies.clear();
    w.player = [0., EYE_HEIGHT + 0.01, 1.8];
    w.set_props(vec![
        floor(),
        Prop::cuboid(2, "low platform", [0., 0.2, 0.], [1., 0.2, 1.], [1.; 3]),
    ])
    .unwrap();
    for tick in 0..20 {
        w.update(Input {
            forward: 1.,
            jump: tick == 0,
            ..Default::default()
        });
    }
    for _ in 0..100 {
        w.update(Input::default());
    }
    assert!(w.player[2] < 1.);
    assert!(
        (w.player[1] - EYE_HEIGHT - 0.4 - SKIN).abs() < 0.005,
        "must land on platform: {:?}",
        w.player
    );
}

#[test]
fn courtyard_perimeter_and_rotated_props_use_their_rendered_poses() {
    let mut w = World::new(15);
    w.zombies.clear();
    for _ in 0..400 {
        w.update(Input {
            strafe: 1.,
            sprint: true,
            ..Default::default()
        });
    }
    // Wall's rendered inner face is x=14.5, then capsule radius and skin.
    assert!((w.player[0] - (14.5 - PLAYER_RADIUS - SKIN)).abs() < 0.01);
    let hit = w.raycast_world([0., 1.65, 10.], [1., 0., 0.], 30.).unwrap();
    assert!((hit.distance - 14.5).abs() < 0.001);
    assert_eq!(w.collision().colliders[hit.collider].user_data, 301);
    let mut prop = Prop::cuboid(2, "rotated wall", [4., 1., -3.], [1., 1., 0.2], [1.; 3]);
    prop.pose = RigidPose3::yaw(prop.pose.translation, std::f32::consts::FRAC_PI_4);
    w.set_props(vec![floor(), prop.clone()]).unwrap();
    let hit = w.raycast_world([4., 1., 0.], [0., 0., -1.], 6.).unwrap();
    assert!((hit.distance - (3. - 0.2 * 2f32.sqrt())).abs() < 0.002);
    assert_eq!(
        w.collision().colliders[hit.collider].user_data,
        prop.id as u128
    );
    let nav = w.navigation();
    assert!(nav.grid().is_blocked(nav.cell([4., -3.])));
}

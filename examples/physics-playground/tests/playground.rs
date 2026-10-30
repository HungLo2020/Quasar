use physics_playground::render::Graphics;
use physics_playground::world::{MAX_PROPS, Playground, PlaygroundInput, PropKind, autoplay_input};
use quasar_frame::OffscreenTarget;
use quasar_spatial::{Aabb, Frustum, Plane};
use vulkanic_gal::BackendChoice;

fn stand(world: &mut Playground, ticks: u32) {
    for _ in 0..ticks {
        world.update(PlaygroundInput::default());
    }
}

#[test]
fn player_settles_on_the_ground() {
    let mut world = Playground::new(3);
    stand(&mut world, 60);
    let stats = world.stats();
    assert!(stats.grounded, "player not grounded: {stats:?}");
    assert!(
        (stats.player[1] - 0.86).abs() < 0.2,
        "player y={}",
        stats.player[1]
    );
}

#[test]
fn walking_moves_the_player_and_pushes_a_block() {
    let mut world = Playground::new(5);
    stand(&mut world, 30);
    let block_before = world.prop_transform(0).unwrap().0;
    let player_before = world.player_position();
    for _ in 0..120 {
        world.update(PlaygroundInput {
            forward: 1.0,
            ..PlaygroundInput::default()
        });
    }
    let player_after = world.player_position();
    let block_after = world.prop_transform(0).unwrap().0;
    assert!(player_after[2] < player_before[2] - 2.0);
    assert!(
        block_after[2] < block_before[2] - 0.05,
        "block was not pushed: {block_before:?} -> {block_after:?}"
    );
}

#[test]
fn jumping_raises_and_then_lands() {
    let mut world = Playground::new(7);
    stand(&mut world, 60);
    let grounded_y = world.player_position()[1];
    world.update(PlaygroundInput {
        jump: true,
        ..PlaygroundInput::default()
    });
    let mut peak = grounded_y;
    for _ in 0..40 {
        world.update(PlaygroundInput::default());
        peak = peak.max(world.player_position()[1]);
    }
    for _ in 0..120 {
        world.update(PlaygroundInput::default());
    }
    assert!(peak > grounded_y + 0.6, "peak={peak} grounded={grounded_y}");
    assert!(world.stats().grounded);
    assert!((world.player_position()[1] - grounded_y).abs() < 0.15);
}

#[test]
fn look_raycast_ignores_the_player_and_kicks_blocks() {
    let mut world = Playground::new(9);
    stand(&mut world, 30);
    let hit = world.look_hit().expect("expected a block in the crosshair");
    assert!(hit.prop.is_some());
    assert!(hit.distance > 0.2);
    let index = hit.prop.unwrap();
    let before = world.prop_transform(index).unwrap().0;
    world.update(PlaygroundInput {
        kick: true,
        ..PlaygroundInput::default()
    });
    for _ in 0..30 {
        world.update(PlaygroundInput::default());
    }
    let after = world.prop_transform(index).unwrap().0;
    assert!(
        (after[0] - before[0]).abs() + (after[2] - before[2]).abs() > 0.02,
        "kick did not move the prop: {before:?} -> {after:?}"
    );
}

#[test]
fn spawning_ball_adds_a_prop_until_the_cap() {
    let mut world = Playground::new(11);
    let before = world.stats().props;
    world.update(PlaygroundInput {
        spawn: true,
        ..PlaygroundInput::default()
    });
    assert_eq!(world.stats().props, before + 1);
    assert_eq!(world.stats().balls, 4);
    for _ in 0..MAX_PROPS + 8 {
        world.update(PlaygroundInput {
            spawn: true,
            ..PlaygroundInput::default()
        });
    }
    assert!(world.stats().props <= MAX_PROPS);
}

#[test]
fn moving_through_the_sensor_sets_the_trigger_state() {
    let mut world = Playground::new(13);
    let mut entered = false;
    let mut surface_entered_before_center = false;
    for _ in 0..220 {
        world.update(PlaygroundInput {
            forward: 1.0,
            ..PlaygroundInput::default()
        });
        if world.zone_active() {
            entered = true;
            surface_entered_before_center |= !world
                .sensor_bounds()
                .contains_point(&world.player_position().map(f64::from));
        }
    }
    assert!(
        entered,
        "player never entered the sensor zone at {:?}",
        world.player_position()
    );
    assert!(
        surface_entered_before_center,
        "trigger must use the capsule surface"
    );
}

#[test]
fn spatial_queries_cover_props_and_frustum_culling() {
    let mut world = Playground::new(17);
    stand(&mut world, 10);
    let all = Frustum::new(vec![
        Plane::new([1.0, 0.0, 0.0], 50.0).unwrap(),
        Plane::new([-1.0, 0.0, 0.0], 50.0).unwrap(),
        Plane::new([0.0, 1.0, 0.0], 50.0).unwrap(),
        Plane::new([0.0, -1.0, 0.0], 50.0).unwrap(),
        Plane::new([0.0, 0.0, 1.0], 50.0).unwrap(),
        Plane::new([0.0, 0.0, -1.0], 50.0).unwrap(),
    ])
    .unwrap();
    let region = Aabb::new([-20.0, -2.0, -20.0], [20.0, 7.0, 20.0]).unwrap();
    assert_eq!(
        world.props_in_frustum(&all, region).len(),
        world.props().len()
    );
    let none = Frustum::new(vec![Plane::new([1.0, 0.0, 0.0], -100.0).unwrap()]).unwrap();
    assert!(world.props_in_frustum(&none, region).is_empty());
    assert!(world.stats().nearby_props > 0);
}

#[test]
fn scripted_runs_are_reproducible_and_reset_restores_initial_state() {
    fn run() -> Playground {
        let mut world = Playground::new(19);
        for _ in 0..180 {
            world.update(autoplay_input(&world));
        }
        world
    }
    let first = run();
    let second = run();
    assert_eq!(first.state_hash(), second.state_hash());
    assert_eq!(first.stats().props, second.stats().props);
    let before = first.state_hash();
    let mut reset = first;
    reset.update(PlaygroundInput {
        reset: true,
        ..PlaygroundInput::default()
    });
    assert_eq!(reset.tick(), 0);
    assert_ne!(reset.state_hash(), before);
    assert_eq!(reset.state_hash(), Playground::new(19).state_hash());
}

#[test]
fn props_hold_transform_data_for_every_kind() {
    let world = Playground::new(23);
    for (index, prop) in world.props().iter().enumerate() {
        let (position, rotation) = world.prop_transform(index).unwrap();
        assert!(position.iter().all(|value| value.is_finite()));
        assert!(rotation.iter().all(|value| value.is_finite()));
        match prop.kind {
            PropKind::Ball { radius } => assert!(radius > 0.0),
            PropKind::Block { half } => assert!(half.iter().all(|value| *value > 0.0)),
        }
    }
}

#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn offscreen_scene_is_non_uniform_and_resources_balance() {
    let mut gal =
        vulkanic_gal::VulkanicGal::create(BackendChoice::Vulkan, "playground test").unwrap();
    let mut graphics = Graphics::new(&mut gal).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 480, 270).unwrap();
    let mut world = Playground::new(29);
    for _ in 0..30 {
        world.update(autoplay_input(&world));
    }
    let report = target
        .render(&mut gal, |gal, target| {
            graphics.commands(gal, target, &world)
        })
        .unwrap();
    assert_eq!(report.rgba.len(), 480 * 270 * 4);
    let colors: std::collections::BTreeSet<_> =
        report.rgba.as_chunks::<4>().0.iter().copied().collect();
    assert!(colors.len() > 8);
    target.release(&mut gal).unwrap();
    graphics.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    let metrics = gal.metrics();
    assert_eq!(metrics.resource_creates, metrics.resource_destroys);
}

#[test]
fn agents_navigate_and_animate_around_the_arena() {
    let mut world = Playground::new(31);
    let stats = world.stats();
    assert_eq!(stats.agents, 2);
    println!(
        "initial state={:?} {:?}",
        world.agent_state(0),
        world.agent_state(1)
    );
    assert!(stats.agent_path_nodes > 0);
    let start = world.agents()[0].position;
    for _ in 0..180 {
        world.update(PlaygroundInput::default());
    }
    let moved = world.agents()[0].position;
    assert!(
        (moved[0] - start[0]).abs() + (moved[1] - start[1]).abs() > 0.25,
        "agent did not move: {start:?} -> {moved:?}"
    );
    for index in 0..world.agents().len() {
        let state = world.agent_state(index).unwrap();
        assert!(matches!(state, "idle" | "move" | "alert"));
        let pulse = world.agent_pulse(index);
        assert!((0.4..1.5).contains(&pulse), "pulse={pulse}");
    }
}

#[test]
fn navigation_does_not_rebuild_while_obstacles_are_static() {
    let mut world = Playground::new(43);
    for _ in 0..180 {
        world.update(PlaygroundInput::default());
    }
    let generation = world.nav_generation();
    for _ in 0..60 {
        world.update(PlaygroundInput::default());
    }
    assert_eq!(world.nav_generation(), generation);
}

#[test]
fn navigation_debug_paths_can_be_toggled() {
    let mut world = Playground::new(37);
    assert!(!world.debug_nav());
    world.update(PlaygroundInput {
        toggle_debug: true,
        ..PlaygroundInput::default()
    });
    assert!(world.debug_nav());
    assert!(world.stats().debug_nav);
    assert!(!world.agent_path(0).unwrap().is_empty());
    world.update(PlaygroundInput {
        toggle_debug: true,
        ..PlaygroundInput::default()
    });
    assert!(!world.debug_nav());
}

#[test]
fn agent_paths_replan_when_the_navigation_generation_changes() {
    let mut world = Playground::new(41);
    let initial = world.agent_path(0).unwrap().to_vec();
    // Let the simulation move props and rebuild the obstacle grid.
    for _ in 0..45 {
        world.update(PlaygroundInput {
            spawn: true,
            ..PlaygroundInput::default()
        });
    }
    assert!(world.nav_generation() >= 3);
    let later = world.agent_path(0).unwrap().to_vec();
    assert!(!later.is_empty());
    assert_ne!(initial, later);
}

#![cfg(feature = "physics3d")]
use quasar_nav::{Graph, GridCell, SearchOptions, SearchOutcome, physics3d::CollisionGrid3};
use quasar_physics::{
    dim3::*,
    rapier3d::{pipeline::QueryFilter, prelude::ColliderBuilder},
};
#[test]
fn baked_nodes_follow_round_shapes_instead_of_inflated_boxes() {
    let mut world = CollisionWorld::new();
    world.add_collider(ColliderBuilder::cylinder(1., 1.).translation([0., 1., 0.].into()));
    world.update();
    let actor = CapsuleController3::new(1.85, 0.32, 0.005).unwrap();
    let grid = CollisionGrid3::bake(
        &world,
        &actor,
        [21, 21],
        [-2.5, -2.5],
        0.25,
        0.01,
        QueryFilter::default(),
    )
    .unwrap();
    assert!(!grid.grid().is_blocked(grid.cell([1., 1.])));
    assert!(grid.grid().is_blocked(grid.cell([0., 1.])));
    let node = grid
        .nearest_reachable(&world, &actor, [1., 0.01, 1.], QueryFilter::default())
        .unwrap();
    assert!(actor.clear_segment(
        &world,
        [1., 0.01, 1.],
        grid.point(node),
        QueryFilter::default()
    ));
    assert!(
        grid.nearest_reachable(&world, &actor, [f32::NAN, 0., 0.], QueryFilter::default())
            .is_none()
    );
    assert!(
        CollisionGrid3::bake(
            &world,
            &actor,
            [100000, 100000],
            [0., 0.],
            1.,
            0.01,
            QueryFilter::default()
        )
        .is_err()
    );
}
#[test]
fn thin_obstacles_between_free_nodes_remove_edges_and_paths_detour() {
    let mut world = CollisionWorld::new();
    world.add_collider(ColliderBuilder::cylinder(1., 0.12).translation([0.5, 1., 0.].into()));
    world.update();
    let actor = CapsuleController3::new(1.8, 0.1, 0.005).unwrap();
    let grid = CollisionGrid3::bake(
        &world,
        &actor,
        [4, 3],
        [-1., -1.],
        1.,
        0.01,
        QueryFilter::default(),
    )
    .unwrap();
    let a = GridCell::new2(1, 1);
    let b = GridCell::new2(2, 1);
    assert!(!grid.grid().is_blocked(a));
    assert!(!grid.grid().is_blocked(b));
    let mut edges = Vec::new();
    grid.neighbors(a, &mut edges);
    assert!(!edges.iter().any(|e| e.node == b));
    let SearchOutcome::Found(path) = grid.find_path(
        GridCell::new2(0, 1),
        GridCell::new2(3, 1),
        SearchOptions::default(),
    ) else {
        panic!("detour should exist")
    };
    assert!(path.nodes.iter().any(|n| n.coords[1] != 1));
    for pair in path.nodes.windows(2) {
        assert!(actor.clear_segment(
            &world,
            grid.point(pair[0]),
            grid.point(pair[1]),
            QueryFilter::default()
        ));
    }
    assert_eq!(
        grid.find_path(
            GridCell::new2(0, 1),
            GridCell::new2(3, 1),
            SearchOptions::default()
        ),
        SearchOutcome::Found(path)
    );
}

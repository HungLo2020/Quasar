use quasar_nav::{Graph, Grid, GridCell, Neighbor, SearchOptions, SearchOutcome, astar, dijkstra};
use std::sync::{Arc, atomic::AtomicBool};

struct Chain(usize);
impl Graph for Chain {
    type Node = usize;
    fn neighbors(&self, node: usize, output: &mut Vec<Neighbor<usize>>) {
        if node + 1 < self.0 {
            output.push(Neighbor::new(node + 1, 1.0));
        }
        if node > 0 {
            output.push(Neighbor::new(node - 1, 1.0));
        }
    }
}

#[test]
fn generic_astar_and_dijkstra_find_optimal_chain_paths() {
    let graph = Chain(12);
    let a = astar(
        &graph,
        0,
        11,
        |node| (11 - node) as f32,
        SearchOptions::default(),
    )
    .path()
    .unwrap();
    let d = dijkstra(&graph, 0, 11, SearchOptions::default())
        .path()
        .unwrap();
    assert_eq!(a, d);
    assert_eq!(a.nodes.len(), 12);
    assert_eq!(a.cost, 11.0);
    assert!(matches!(
        astar(
            &graph,
            0,
            11,
            |_| 0.0,
            SearchOptions {
                max_visited: 2,
                ..Default::default()
            }
        ),
        SearchOutcome::LimitReached { .. }
    ));
}

#[test]
fn cost_budgets_return_limit_reached() {
    let graph = Chain(20);
    let outcome = astar(
        &graph,
        0,
        19,
        |_| 0.0,
        SearchOptions {
            max_cost: 5.0,
            ..SearchOptions::default()
        },
    );
    assert!(matches!(outcome, SearchOutcome::LimitReached { .. }));
}

#[test]
fn cancellation_is_checked_before_searching() {
    let graph = Chain(100);
    let cancel = Arc::new(AtomicBool::new(true));
    let outcome = astar(
        &graph,
        0,
        99,
        |_| 0.0,
        SearchOptions::default().with_cancel(cancel),
    );
    assert!(matches!(outcome, SearchOutcome::Cancelled { visited: 0 }));
}

#[test]
fn grid_astar_and_dijkstra_agree_and_respect_blocked_goals() {
    let mut grid = Grid::<2>::new([12, 9]).unwrap();
    for y in 0..9 {
        if y != 4 {
            grid.set_blocked(GridCell::new2(6, y), true).unwrap();
        }
    }
    let start = GridCell::new2(0, 0);
    let goal = GridCell::new2(11, 8);
    let a = grid.find_path(start, goal, SearchOptions::default());
    let d = grid.find_path_dijkstra(start, goal, SearchOptions::default());
    let (Some(a), Some(d)) = (a.path(), d.path()) else {
        panic!("expected paths");
    };
    assert!((a.cost - d.cost).abs() < 1e-4);
    assert!(a.cost < f64::INFINITY);
    assert!(a.nodes.iter().all(|cell| !grid.is_blocked(*cell)));

    grid.set_blocked(goal, true).unwrap();
    assert!(matches!(
        grid.find_path(start, goal, SearchOptions::default()),
        SearchOutcome::NotFound { .. }
    ));
}

#[test]
fn dynamic_obstacles_invalidate_paths_and_the_version() {
    let mut grid = Grid::<2>::new([10, 10]).unwrap();
    let start = GridCell::new2(0, 0);
    let goal = GridCell::new2(9, 9);
    let version = grid.version();
    assert!(
        grid.find_path(start, goal, SearchOptions::default())
            .path()
            .is_some()
    );
    for y in 0..10 {
        grid.set_blocked(GridCell::new2(5, y), true).unwrap();
    }
    assert!(grid.version() > version);
    assert!(matches!(
        grid.find_path(start, goal, SearchOptions::default()),
        SearchOutcome::NotFound { .. }
    ));
    grid.set_blocked(GridCell::new2(5, 5), false).unwrap();
    let path = grid
        .find_path(start, goal, SearchOptions::default())
        .path()
        .unwrap();
    assert!(path.nodes.contains(&GridCell::new2(5, 5)));
}

#[test]
fn astar_uses_the_goal_heuristic_instead_of_expanding_the_whole_grid() {
    let grid = Grid::<2>::new([64, 64]).unwrap();
    let start = GridCell::new2(0, 0);
    let goal = GridCell::new2(63, 63);
    let a = grid
        .find_path(start, goal, SearchOptions::default())
        .path()
        .unwrap();
    let d = grid
        .find_path_dijkstra(start, goal, SearchOptions::default())
        .path()
        .unwrap();
    assert!(
        a.visited <= a.nodes.len() + 2,
        "A* visited {} for {} nodes",
        a.visited,
        a.nodes.len()
    );
    assert!(d.visited > a.visited);
    assert!((a.cost - d.cost).abs() < 1e-6);
}

#[test]
fn weighted_costs_prefer_the_cheap_route() {
    let mut grid = Grid::<2>::new([5, 5]).unwrap();
    // Make the central row expensive but passable; the path should go around.
    for x in 0..5 {
        grid.set_cost(GridCell::new2(x, 2), 20.0).unwrap();
    }
    let path = grid
        .find_path(
            GridCell::new2(0, 2),
            GridCell::new2(4, 2),
            SearchOptions::default(),
        )
        .path()
        .unwrap();
    assert!(path.cost < 60.0);
    assert!(
        path.nodes
            .iter()
            .all(|cell| cell.coords[1] != 2 || cell.coords[0] == 0 || cell.coords[0] == 4)
    );
}

#[test]
fn diagonal_corner_policy_is_explicit() {
    let mut grid = Grid::<2>::new([3, 3]).unwrap();
    grid.set_blocked(GridCell::new2(1, 0), true).unwrap();
    grid.set_blocked(GridCell::new2(0, 1), true).unwrap();
    grid.set_corner_cutting(true);
    assert!(
        grid.find_path(
            GridCell::new2(0, 0),
            GridCell::new2(1, 1),
            SearchOptions::default()
        )
        .path()
        .is_some()
    );
    grid.set_corner_cutting(false);
    assert!(matches!(
        grid.find_path(
            GridCell::new2(0, 0),
            GridCell::new2(1, 1),
            SearchOptions::default()
        ),
        SearchOutcome::NotFound { .. }
    ));
}

#[test]
fn smoothing_collapses_navigable_straight_runs() {
    let grid = Grid::<2>::new([9, 9]).unwrap();
    let path = grid
        .find_path(
            GridCell::new2(0, 0),
            GridCell::new2(8, 0),
            SearchOptions::default(),
        )
        .path()
        .unwrap();
    let smoothed = quasar_nav::smooth_path(&path.nodes, |a, b| grid.line_of_sight(a, b));
    assert_eq!(smoothed, vec![GridCell::new2(0, 0), GridCell::new2(8, 0)]);
    let outcome = grid.find_smoothed_path(
        GridCell::new2(0, 0),
        GridCell::new2(8, 8),
        SearchOptions::default(),
    );
    let smoothed = outcome.path().unwrap();
    assert_eq!(smoothed.nodes.len(), 2);
}

#[test]
fn three_dimensional_voxel_paths_work() {
    let mut grid = Grid::<3>::new([8, 6, 5]).unwrap();
    for x in 0..8 {
        for z in 0..5 {
            if x != 4 {
                grid.set_blocked(GridCell::new3(x, 2, z), true).unwrap();
            }
        }
    }
    let path = grid
        .find_path(
            GridCell::new3(0, 0, 0),
            GridCell::new3(7, 5, 4),
            SearchOptions::default(),
        )
        .path()
        .unwrap();
    assert_eq!(path.nodes.first(), Some(&GridCell::new3(0, 0, 0)));
    assert_eq!(path.nodes.last(), Some(&GridCell::new3(7, 5, 4)));
    assert!(path.nodes.iter().all(|cell| !grid.is_blocked(*cell)));
}

#[test]
fn repeated_multi_agent_requests_are_deterministic() {
    let mut grid = Grid::<2>::new([30, 30]).unwrap();
    for y in 5..25 {
        grid.set_blocked(GridCell::new2(15, y), true).unwrap();
    }
    let mut first = Vec::new();
    for index in 0..20 {
        let start = GridCell::new2(index % 10, index);
        let goal = GridCell::new2(25, 29 - index);
        first.push(
            grid.find_path(start, goal, SearchOptions::default())
                .path()
                .unwrap()
                .nodes,
        );
    }
    for index in 0..20 {
        let start = GridCell::new2(index % 10, index);
        let goal = GridCell::new2(25, 29 - index);
        let again = grid
            .find_path(start, goal, SearchOptions::default())
            .path()
            .unwrap()
            .nodes;
        assert_eq!(first[index as usize], again);
    }
}

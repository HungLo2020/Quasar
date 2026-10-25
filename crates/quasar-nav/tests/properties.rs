use proptest::prelude::*;
use quasar_nav::{Grid, GridCell, SearchOptions, SearchOutcome};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn astar_matches_dijkstra_cost_on_random_grids(
        blocked in proptest::collection::vec(any::<bool>(), 10 * 10),
        start_x in 0i32..10,
        start_y in 0i32..10,
        goal_x in 0i32..10,
        goal_y in 0i32..10,
    ) {
        let grid = Grid::<2>::from_blocked([10, 10], blocked).unwrap();
        let start = GridCell::new2(start_x, start_y);
        let goal = GridCell::new2(goal_x, goal_y);
        if grid.is_blocked(start) || grid.is_blocked(goal) {
            let outcome = grid.find_path(start, goal, SearchOptions::default());
            prop_assert!(outcome.path().is_none());
            return Ok(());
        }
        let options = SearchOptions { max_visited: 100_000, ..Default::default() };
        let a = grid.find_path(start, goal, options.clone());
        let d = grid.find_path_dijkstra(start, goal, options);
        match (a, d) {
            (SearchOutcome::Found(a), SearchOutcome::Found(d)) => {
                prop_assert!((a.cost - d.cost).abs() < 1e-3);
                prop_assert_eq!(a.nodes.first(), Some(&start));
                prop_assert_eq!(a.nodes.last(), Some(&goal));
                for cell in &a.nodes {
                    prop_assert!(!grid.is_blocked(*cell));
                }
            }
            (SearchOutcome::NotFound { .. }, SearchOutcome::NotFound { .. }) => {}
            other => prop_assert!(false, "path mismatch: {:?}", other),
        }
    }

    #[test]
    fn blocking_never_creates_an_invalid_path(
        blocked in proptest::collection::vec(any::<bool>(), 8 * 8),
        update_x in 0i32..8,
        update_y in 0i32..8,
        block in any::<bool>(),
    ) {
        let mut grid = Grid::<2>::from_blocked([8, 8], blocked).unwrap();
        grid.set_blocked(GridCell::new2(update_x, update_y), block).unwrap();
        let outcome = grid.find_path(
            GridCell::new2(0, 0),
            GridCell::new2(7, 7),
            SearchOptions::default(),
        );
        if let SearchOutcome::Found(path) = outcome {
            for window in path.nodes.windows(2) {
                prop_assert!(window[0].coords != window[1].coords);
                prop_assert!(!grid.is_blocked(window[1]));
            }
        }
    }
}

#[test]
fn large_map_stress_remains_bounded_and_deterministic() {
    let mut grid = Grid::<2>::new([96, 96]).unwrap();
    let mut state = 0x1234_5678u64;
    for y in 0..96 {
        for x in 0..96 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if state % 100 < 20 {
                grid.set_blocked(GridCell::new2(x, y), true).unwrap();
            }
        }
    }
    let mut first = Vec::new();
    for index in 0..20 {
        let start = GridCell::new2(index % 10, index);
        let goal = GridCell::new2(90, 95 - index);
        let outcome = grid.find_path(
            start,
            goal,
            SearchOptions {
                max_visited: 500_000,
                ..Default::default()
            },
        );
        first.push(match outcome {
            SearchOutcome::Found(path) => Some((path.cost, path.nodes)),
            SearchOutcome::NotFound { .. } => None,
            other => panic!("unexpected {other:?}"),
        });
    }
    for index in 0..20 {
        let start = GridCell::new2(index % 10, index);
        let goal = GridCell::new2(90, 95 - index);
        let outcome = grid.find_path(
            start,
            goal,
            SearchOptions {
                max_visited: 500_000,
                ..Default::default()
            },
        );
        let again = match outcome {
            SearchOutcome::Found(path) => Some((path.cost, path.nodes)),
            SearchOutcome::NotFound { .. } => None,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(first[index as usize], again);
    }
}

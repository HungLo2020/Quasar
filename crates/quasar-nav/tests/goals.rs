//! Moving goals: in-place, incremental `set_goals` must match a reference
//! binary-heap Dijkstra exactly (distances and the chosen next cell), so
//! games that replay seeded runs see identical agent routes.
use proptest::prelude::*;
use quasar_nav::{DistanceField, FieldOptions, Grid, GridCell};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(PartialEq)]
struct Open(f32, usize);
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .0
            .total_cmp(&self.0)
            .then_with(|| other.1.cmp(&self.1))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Textbook reverse Dijkstra with a binary heap ordered by (cost, index):
/// the behaviour Quasar's fields have always had.
fn reference<const N: usize>(
    grid: &Grid<N>,
    goals: &[GridCell<N>],
) -> (Vec<f32>, Vec<Option<usize>>) {
    let len = grid.len();
    let mut into: Vec<Vec<(usize, f32)>> = vec![Vec::new(); len];
    let mut scratch = Vec::new();
    for v in 0..len {
        let cell = grid.cell(v).unwrap();
        if grid.is_blocked(cell) {
            continue;
        }
        grid.neighbors_into(cell, &mut scratch);
        for n in &scratch {
            into[grid.index(n.node).unwrap()].push((v, n.cost));
        }
    }
    let mut distance = vec![f32::INFINITY; len];
    let mut parent = vec![None; len];
    let mut heap = BinaryHeap::new();
    for goal in goals {
        if let Ok(i) = grid.index(*goal)
            && !grid.is_blocked(*goal)
            && distance[i] > 0.0
        {
            distance[i] = 0.0;
            heap.push(Open(0.0, i));
        }
    }
    while let Some(Open(cost, u)) = heap.pop() {
        if cost > distance[u] {
            continue;
        }
        for &(v, step) in &into[u] {
            let candidate = cost + step;
            if candidate < distance[v] {
                distance[v] = candidate;
                parent[v] = Some(u);
                heap.push(Open(candidate, v));
            }
        }
    }
    (distance, parent)
}

fn assert_exact<const N: usize>(grid: &Grid<N>, field: &DistanceField<N>, goals: &[GridCell<N>]) {
    let (distance, parent) = reference(grid, goals);
    assert_eq!(field.distances(), &distance[..], "distances");
    for (index, expected) in parent.iter().enumerate() {
        let cell = grid.cell(index).unwrap();
        let next = field.next_cell(grid, cell).map(|c| grid.index(c).unwrap());
        assert_eq!(next, *expected, "next cell from {cell:?}");
    }
}

fn cave(width: u32, height: u32) -> Grid<3> {
    let blocked = (0..width * height)
        .map(|i| {
            let (x, z) = (i % width, i / width);
            (x * 7 + z * 13) % 11 == 0 || x == 0 || z == 0 || x == width - 1 || z == height - 1
        })
        .collect();
    Grid::<3>::from_blocked([width, 1, height], blocked).unwrap()
}

#[test]
fn a_followed_goal_matches_a_fresh_search_every_step() {
    let grid = cave(40, 30);
    let mut goal = GridCell::new([5, 0, 5]);
    let mut field = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    assert_exact(&grid, &field, &[goal]);
    for step in 0..60 {
        goal.coords[0] = 3 + step % 34;
        goal.coords[2] = 4 + (step / 3) % 22;
        field.set_goals(&grid, &[goal]).unwrap();
        assert_exact(&grid, &field, &[goal]);
    }
}

#[test]
fn adding_and_removing_goals_repairs_incrementally_and_exactly() {
    let mut grid = cave(30, 30);
    grid.set_cost(GridCell::new([10, 0, 10]), 3.0).unwrap();
    let a = GridCell::new([3, 0, 3]);
    let b = GridCell::new([25, 0, 26]);
    let c = GridCell::new([14, 0, 4]);
    let mut field = grid.distance_field(&[a], &FieldOptions::default()).unwrap();
    for goals in [
        vec![a, b],                         // add
        vec![a, b, c],                      // add
        vec![a, c],                         // remove one
        vec![c],                            // remove another
        vec![c, a],                         // re-add
        vec![],                             // nothing
        vec![b],                            // from empty
        vec![GridCell::new([0, 0, 0])],     // blocked goal
        vec![b, GridCell::new([99, 0, 0])], // out of bounds ignored
    ] {
        field.set_goals(&grid, &goals).unwrap();
        assert_eq!(field.goals(), &goals[..]);
        assert_exact(&grid, &field, &goals);
    }
}

#[test]
fn set_goals_rebuilds_a_stale_field() {
    let mut grid = cave(20, 20);
    let goal = GridCell::new([5, 0, 5]);
    let mut field = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    for x in 2..18 {
        grid.set_blocked(GridCell::new([x, 0, 9]), true).unwrap();
    }
    assert!(field.is_stale(&grid));
    let moved = GridCell::new([6, 0, 5]);
    field.set_goals(&grid, &[moved]).unwrap();
    assert!(!field.is_stale(&grid));
    assert_exact(&grid, &field, &[moved]);
}

#[test]
fn zero_cost_cells_and_bounded_fields_stay_exact() {
    // Zero-cost cells take the binary-heap path; bounded fields stop early.
    let mut grid = Grid::<2>::new([16, 12]).unwrap();
    for x in 3..9 {
        grid.set_cost(GridCell::new2(x, 4), 0.0).unwrap();
    }
    let goals = [GridCell::new2(1, 1)];
    let field = grid
        .distance_field(&goals, &FieldOptions::default())
        .unwrap();
    assert_exact(&grid, &field, &goals);
    let bounded = grid
        .distance_field(&goals, &FieldOptions::with_max_cost(6.0))
        .unwrap();
    let (distance, _) = reference(&grid, &goals);
    for (got, want) in bounded.distances().iter().zip(&distance) {
        if *want <= 6.0 {
            assert_eq!(got, want);
        } else {
            assert!(got.is_infinite());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_goal_changes_match_the_reference(
        blocked in proptest::collection::vec(any::<bool>(), 144),
        costs in proptest::collection::vec(0u8..4, 144),
        goal_sets in proptest::collection::vec(proptest::collection::vec(0usize..144, 0..4), 1..8),
        diagonal in any::<bool>(),
        corner_cutting in any::<bool>(),
    ) {
        let blocked: Vec<bool> = blocked.iter().enumerate().map(|(i, b)| *b && i % 3 == 0).collect();
        let mut grid = Grid::<2>::from_blocked([12, 12], blocked).unwrap();
        grid.set_diagonal(diagonal);
        grid.set_corner_cutting(corner_cutting);
        for (i, c) in costs.iter().enumerate() {
            let cost = [1.0, 1.0, 2.5, 0.5][*c as usize];
            grid.set_cost(grid.cell(i).unwrap(), cost).unwrap();
        }
        let mut field = grid.distance_field(&[], &FieldOptions::default()).unwrap();
        for set in goal_sets {
            let goals: Vec<GridCell<2>> = set.iter().map(|i| grid.cell(*i).unwrap()).collect();
            field.set_goals(&grid, &goals).unwrap();
            let (distance, parent) = reference(&grid, &goals);
            prop_assert_eq!(field.distances(), &distance[..]);
            for (index, expected) in parent.iter().enumerate() {
                let cell = grid.cell(index).unwrap();
                let next = field.next_cell(&grid, cell).map(|c| grid.index(c).unwrap());
                prop_assert_eq!(next, *expected);
            }
        }
    }
}

mod steering {
    use quasar_nav::{FieldOptions, Grid, GridCell, Steer, Steering};

    fn walled() -> Grid<2> {
        // A wall with a gap at the bottom between the agent and the goal.
        let mut grid = Grid::<2>::new([12, 10]).unwrap();
        for y in 0..8 {
            grid.set_blocked(GridCell::new2(6, y), true).unwrap();
        }
        grid
    }

    #[test]
    fn visible_goals_are_approached_directly() {
        let grid = Grid::<2>::new([10, 10]).unwrap();
        let goal = GridCell::new2(8, 8);
        let field = grid
            .distance_field(&[goal], &FieldOptions::default())
            .unwrap();
        let steering = Steering::default();
        assert_eq!(
            steering.steer_on_grid(&field, &grid, GridCell::new2(1, 1), goal),
            Steer::Goal
        );
        assert_eq!(
            steering.steer_on_grid(&field, &grid, goal, goal),
            Steer::Arrived
        );
    }

    #[test]
    fn hidden_goals_follow_the_field_and_cut_visible_corners() {
        let grid = walled();
        let goal = GridCell::new2(10, 1);
        let field = grid
            .distance_field(&[goal], &FieldOptions::default())
            .unwrap();
        let start = GridCell::new2(2, 1);
        let next = field.next_cell(&grid, start).unwrap();
        let none = Steering::new(1);
        assert_eq!(
            none.steer_on_grid(&field, &grid, start, goal),
            Steer::Toward(next)
        );
        let ahead = Steering::new(6).steer_on_grid(&field, &grid, start, goal);
        let Steer::Toward(target) = ahead else {
            panic!("{ahead:?}");
        };
        assert!(grid.line_of_sight(start, target));
        assert!(
            field.distance(target).unwrap() < field.distance(next).unwrap(),
            "lookahead skips ahead along the route"
        );
        // Custom visibility: nothing visible means the next cell.
        let blind = Steering::new(6).steer(&field, &grid, start, || false, |_| false);
        assert_eq!(blind, Steer::Toward(next));
    }

    #[test]
    fn unreachable_agents_are_stuck() {
        let mut grid = walled();
        grid.set_blocked(GridCell::new2(6, 8), true).unwrap();
        grid.set_blocked(GridCell::new2(6, 9), true).unwrap();
        let goal = GridCell::new2(10, 1);
        let field = grid
            .distance_field(&[goal], &FieldOptions::default())
            .unwrap();
        let steer = Steering::default().steer_on_grid(&field, &grid, GridCell::new2(1, 1), goal);
        assert_eq!(steer, Steer::Stuck);
        let outside =
            Steering::default().steer(&field, &grid, GridCell::new2(-3, 0), || false, |_| true);
        assert_eq!(outside, Steer::Stuck);
    }
}

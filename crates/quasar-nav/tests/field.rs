//! Distance/flow fields: agreement with point-to-point search, steering,
//! bounds and incremental repair.
use proptest::prelude::*;
use quasar_nav::{FieldOptions, Grid, GridCell, NavError, SearchOptions};

fn assert_fields_match<const N: usize>(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (index, (x, y)) in a.iter().zip(b).enumerate() {
        if x.is_infinite() || y.is_infinite() {
            assert_eq!(x, y, "cell {index}");
        } else {
            assert!((x - y).abs() < 1e-3, "cell {index}: {x} vs {y}");
        }
    }
}

#[test]
fn field_costs_equal_dijkstra_path_costs() {
    let mut grid = Grid::<2>::new([12, 9]).unwrap();
    for y in 0..7 {
        grid.set_blocked(GridCell::new2(5, y), true).unwrap();
    }
    grid.set_cost(GridCell::new2(8, 4), 4.0).unwrap();
    let goal = GridCell::new2(10, 1);
    let field = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    for index in 0..grid.len() {
        let cell = grid.cell(index).unwrap();
        let expected = grid
            .find_path_dijkstra(cell, goal, SearchOptions::default())
            .path()
            .map(|path| path.cost as f32);
        match (field.distance(cell), expected) {
            (Some(actual), Some(expected)) => {
                assert!(
                    (actual - expected).abs() < 1e-3,
                    "{cell}: {actual} vs {expected}"
                )
            }
            (None, None) => assert!(grid.is_blocked(cell)),
            other => panic!("{cell}: {other:?}"),
        }
    }
}

#[test]
fn agents_follow_the_gradient_to_the_nearest_goal() {
    let mut grid = Grid::<2>::new([16, 16]).unwrap();
    for x in 2..14 {
        grid.set_blocked(GridCell::new2(x, 8), true).unwrap();
    }
    let goals = [GridCell::new2(0, 0), GridCell::new2(15, 15)];
    let field = grid
        .distance_field(&goals, &FieldOptions::default())
        .unwrap();
    assert_eq!(field.distance(goals[0]), Some(0.0));
    assert_eq!(field.next_cell(&grid, goals[0]), None);
    assert_eq!(field.gradient(&grid, goals[1]), None);
    for start in [
        GridCell::new2(7, 6),
        GridCell::new2(7, 10),
        GridCell::new2(15, 0),
    ] {
        let path = field.path_from(&grid, start);
        assert_eq!(path.first(), Some(&start));
        assert!(goals.contains(path.last().unwrap()), "{path:?}");
        // Distances fall strictly along the route and steps are adjacent.
        for pair in path.windows(2) {
            assert!(field.distance(pair[1]).unwrap() < field.distance(pair[0]).unwrap());
            assert!(!grid.is_blocked(pair[1]));
            let dx = (pair[1].coords[0] - pair[0].coords[0]).abs();
            let dy = (pair[1].coords[1] - pair[0].coords[1]).abs();
            assert!(dx <= 1 && dy <= 1 && dx + dy > 0);
        }
        let gradient = field.gradient(&grid, start).unwrap();
        let length = (gradient[0] * gradient[0] + gradient[1] * gradient[1]).sqrt();
        assert!((length - 1.0).abs() < 1e-5);
    }
    // Above the wall, the top-left goal is nearer; below it, the bottom-right.
    assert_eq!(
        field.path_from(&grid, GridCell::new2(3, 3)).last(),
        Some(&goals[0])
    );
    assert_eq!(
        field.path_from(&grid, GridCell::new2(12, 12)).last(),
        Some(&goals[1])
    );
}

#[test]
fn unreachable_blocked_and_bounded_cells_have_no_distance() {
    let mut grid = Grid::<2>::new([10, 3]).unwrap();
    for y in 0..3 {
        grid.set_blocked(GridCell::new2(5, y), true).unwrap();
    }
    let field = grid
        .distance_field(&[GridCell::new2(0, 1)], &FieldOptions::default())
        .unwrap();
    assert_eq!(field.distance(GridCell::new2(8, 1)), None);
    assert_eq!(field.distance(GridCell::new2(5, 1)), None);
    assert_eq!(field.distance(GridCell::new2(-1, 0)), None);
    assert!(field.path_from(&grid, GridCell::new2(8, 1)).is_empty());

    let bounded = grid
        .distance_field(&[GridCell::new2(0, 1)], &FieldOptions::with_max_cost(2.5))
        .unwrap();
    assert_eq!(bounded.distance(GridCell::new2(2, 1)), Some(2.0));
    assert_eq!(bounded.distance(GridCell::new2(3, 1)), None);

    let blocked_goal = grid
        .distance_field(&[GridCell::new2(5, 0)], &FieldOptions::default())
        .unwrap();
    assert!(blocked_goal.distances().iter().all(|d| d.is_infinite()));
    assert_eq!(
        grid.distance_field(&[], &FieldOptions::with_max_cost(f32::NAN))
            .unwrap_err(),
        NavError::InvalidCost
    );
}

#[test]
fn incremental_repair_matches_rebuild_for_mining_and_walls() {
    let mut grid = Grid::<2>::new([20, 20]).unwrap();
    for y in 0..18 {
        grid.set_blocked(GridCell::new2(10, y), true).unwrap();
    }
    let goals = [GridCell::new2(18, 2)];
    let mut field = grid
        .distance_field(&goals, &FieldOptions::default())
        .unwrap();
    assert!(!field.is_stale(&grid));
    // Mine a hole through the wall: distances on the left must shrink.
    let before = field.distance(GridCell::new2(2, 2)).unwrap();
    grid.set_blocked(GridCell::new2(10, 2), false).unwrap();
    assert!(field.is_stale(&grid));
    let settled = field
        .update_region(&grid, &[GridCell::new2(10, 2)])
        .unwrap();
    assert!(settled > 0);
    assert!(!field.is_stale(&grid));
    assert!(field.distance(GridCell::new2(2, 2)).unwrap() < before);
    let rebuilt = grid
        .distance_field(&goals, &FieldOptions::default())
        .unwrap();
    assert_fields_match::<2>(field.distances(), rebuilt.distances());

    // Close the gap again: routes through it must lengthen.
    grid.set_blocked(GridCell::new2(10, 2), true).unwrap();
    field
        .update_region(&grid, &[GridCell::new2(10, 2)])
        .unwrap();
    let rebuilt = grid
        .distance_field(&goals, &FieldOptions::default())
        .unwrap();
    assert_fields_match::<2>(field.distances(), rebuilt.distances());
    assert!((field.distance(GridCell::new2(2, 2)).unwrap() - before).abs() < 1e-3);

    // A local edit far from the goal touches far fewer cells than a rebuild.
    grid.set_cost(GridCell::new2(0, 0), 3.0).unwrap();
    let settled = field.update_region(&grid, &[GridCell::new2(0, 0)]).unwrap();
    assert!(settled < grid.len() / 2, "{settled}");
    let rebuilt = grid
        .distance_field(&goals, &FieldOptions::default())
        .unwrap();
    assert_fields_match::<2>(field.distances(), rebuilt.distances());

    let other = Grid::<2>::new([5, 5]).unwrap();
    assert_eq!(
        field.update_region(&other, &[]).unwrap_err(),
        NavError::CellCountMismatch
    );
    assert!(
        field
            .update_region(&grid, &[GridCell::new2(99, 0)])
            .is_err()
    );
}

#[test]
fn three_dimensional_fields_work_with_corner_rules() {
    let mut grid = Grid::<3>::new([6, 6, 4]).unwrap();
    for x in 0..6 {
        for y in 0..5 {
            grid.set_blocked(GridCell::new3(x, y, 1), true).unwrap();
        }
    }
    let goal = GridCell::new3(0, 0, 3);
    let mut field = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    let start = GridCell::new3(0, 0, 0);
    let path = field.path_from(&grid, start);
    assert_eq!(path.last(), Some(&goal));
    assert!(
        path.iter().any(|cell| cell.coords[1] == 5),
        "must use the ramp"
    );
    let expected = grid
        .find_path_dijkstra(start, goal, SearchOptions::default())
        .path()
        .unwrap()
        .cost as f32;
    assert!((field.distance(start).unwrap() - expected).abs() < 1e-3);
    grid.set_blocked(GridCell::new3(2, 5, 1), true).unwrap();
    field
        .update_region(&grid, &[GridCell::new3(2, 5, 1)])
        .unwrap();
    let rebuilt = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    assert_fields_match::<3>(field.distances(), rebuilt.distances());
}

#[test]
fn set_goals_rebuilds() {
    let grid = Grid::<2>::new([8, 8]).unwrap();
    let mut field = grid
        .distance_field(&[GridCell::new2(0, 0)], &FieldOptions::default())
        .unwrap();
    field.set_goals(&grid, &[GridCell::new2(7, 7)]).unwrap();
    assert_eq!(field.goals(), &[GridCell::new2(7, 7)]);
    assert_eq!(field.distance(GridCell::new2(7, 7)), Some(0.0));
    assert!(field.distance(GridCell::new2(0, 0)).unwrap() > 9.0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_edits_repair_to_the_rebuilt_field(
        blocked in proptest::collection::vec(any::<bool>(), 144),
        edits in proptest::collection::vec((0usize..144, 0u8..4), 1..12),
        diagonal in any::<bool>(),
        corner_cutting in any::<bool>(),
        goal in 0usize..144,
    ) {
        let blocked: Vec<bool> = blocked.iter().enumerate().map(|(i, b)| *b && i % 3 == 0).collect();
        let mut grid = Grid::<2>::from_blocked([12, 12], blocked).unwrap();
        grid.set_diagonal(diagonal);
        grid.set_corner_cutting(corner_cutting);
        let goals = [grid.cell(goal).unwrap(), grid.cell(143 - goal).unwrap()];
        let mut field = grid.distance_field(&goals, &FieldOptions::default()).unwrap();
        let mut dirty = Vec::new();
        for (index, kind) in edits {
            let cell = grid.cell(index).unwrap();
            match kind {
                0 => { grid.set_blocked(cell, true).unwrap(); }
                1 => { grid.set_blocked(cell, false).unwrap(); }
                2 => grid.set_cost(cell, 3.5).unwrap(),
                _ => grid.set_cost(cell, 0.25).unwrap(),
            }
            dirty.push(cell);
            // Repair after some edits individually and others in batches.
            if index % 2 == 0 {
                field.update_region(&grid, &dirty).unwrap();
                dirty.clear();
            }
        }
        field.update_region(&grid, &dirty).unwrap();
        let rebuilt = grid.distance_field(&goals, &FieldOptions::default()).unwrap();
        for (x, y) in field.distances().iter().zip(rebuilt.distances()) {
            if x.is_infinite() || y.is_infinite() {
                prop_assert_eq!(x, y);
            } else {
                prop_assert!((x - y).abs() < 1e-3, "{} vs {}", x, y);
            }
        }
    }
}

#[cfg(feature = "voxel")]
#[test]
fn voxel_walkable_grids_repair_flow_fields_after_mining() {
    use quasar_math::IVec3 as V;
    use quasar_nav::voxel::*;
    use quasar_voxel::{Cube16, EditBatch, EditOptions, Schema, StorageKind, Volume};
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    let mut volume = Volume::<Cube16>::new(schema);
    let mut batch = EditBatch::new();
    batch.fill(block, V::new(0, 0, 0), V::new(15, 0, 15), 1); // floor
    batch.fill(block, V::new(8, 1, 0), V::new(8, 2, 15), 1); // wall
    volume.apply(&batch, EditOptions::default());
    let bounds = VoxelNavBounds {
        min: V::new(0, 1, 0),
        max: V::new(15, 2, 15),
    };
    let solid = |v: u16| v != 0;
    let mut grid = walkable_grid(&volume, block, bounds, 2, solid).unwrap();
    let goal = bounds.cell(V::new(14, 1, 7)).unwrap();
    let mut field = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    let start = bounds.cell(V::new(1, 1, 7)).unwrap();
    assert_eq!(field.distance(start), None, "the wall separates the halves");
    // Mine a doorway through the wall.
    let mut dig = EditBatch::new();
    dig.fill(block, V::new(8, 1, 7), V::new(8, 2, 7), 0);
    let result = volume.apply(&dig, EditOptions::default());
    let dirty = sync_changes(&mut grid, &volume, block, bounds, 2, &result.changes, solid).unwrap();
    assert!(!dirty.is_empty());
    field.update_region(&grid, &dirty).unwrap();
    let rebuilt = grid
        .distance_field(&[goal], &FieldOptions::default())
        .unwrap();
    assert_fields_match::<3>(field.distances(), rebuilt.distances());
    assert!(
        field.distance(start).is_some(),
        "reachable through the doorway"
    );
    let path = field.path_from(&grid, start);
    assert!(path.iter().any(|c| bounds.voxel(*c) == V::new(8, 1, 7)));
    let air = occupancy_grid(
        &volume,
        block,
        VoxelNavBounds {
            min: V::ZERO,
            max: V::new(15, 3, 15),
        },
        solid,
    )
    .unwrap();
    assert!(air.is_blocked(GridCell::new3(0, 0, 0)) && !air.is_blocked(GridCell::new3(0, 3, 0)));
}

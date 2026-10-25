use proptest::prelude::*;
use quasar_spatial::{Aabb, Frustum, GridConfig, Plane, Ray, SpatialError, SpatialGrid};
use std::collections::BTreeSet;

fn brute_query<const N: usize>(items: &[(u64, Aabb<N>)], region: &Aabb<N>) -> Vec<u64> {
    let mut ids: Vec<u64> = items
        .iter()
        .filter(|(_, bounds)| bounds.intersects(region))
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

fn brute_radius<const N: usize>(
    items: &[(u64, Aabb<N>)],
    center: [f64; N],
    radius: f64,
) -> Vec<u64> {
    let mut ids: Vec<u64> = items
        .iter()
        .filter(|(_, bounds)| bounds.distance_squared_to_point(&center) <= radius * radius)
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

fn random_box<const N: usize>(range: f64) -> impl Strategy<Value = Aabb<N>> {
    (
        proptest::collection::vec(-range..range, N),
        proptest::collection::vec(0.0..range / 4.0, N),
    )
        .prop_map(move |(min, size)| {
            let mut max = [0.0; N];
            for axis in 0..N {
                max[axis] = min[axis] + size[axis];
            }
            Aabb::new(min.try_into().unwrap(), max).unwrap()
        })
}

fn random_box_items<const N: usize>(
    count: usize,
    range: f64,
) -> impl Strategy<Value = Vec<(u64, Aabb<N>)>> {
    proptest::collection::vec(random_box::<N>(range), count).prop_map(|boxes| {
        boxes
            .into_iter()
            .enumerate()
            .map(|(index, bounds)| (index as u64, bounds))
            .collect()
    })
}

fn grid_for<const N: usize>(items: &[(u64, Aabb<N>)], cell_size: f64) -> SpatialGrid<N, u64> {
    let mut grid = SpatialGrid::new(GridConfig::new(cell_size, 64, 10_000).unwrap()).unwrap();
    for (id, bounds) in items {
        grid.insert(*id, *bounds);
    }
    grid
}

#[test]
fn aabb_validation_and_geometry() {
    assert_eq!(
        Aabb::new([0.0, 0.0], [f64::NAN, 1.0]).unwrap_err(),
        SpatialError::NonFinite
    );
    assert_eq!(
        Aabb::new([1.0, 0.0], [0.0, 1.0]).unwrap_err(),
        SpatialError::InvalidBounds
    );
    let a = Aabb::new([0.0, 0.0], [2.0, 4.0]).unwrap();
    let b = Aabb::new([1.0, 3.0], [5.0, 6.0]).unwrap();
    assert_eq!(a.center(), [1.0, 2.0]);
    assert_eq!(a.extents(), [2.0, 4.0]);
    assert_eq!(a.half_extents(), [1.0, 2.0]);
    assert_eq!(a.volume(), 8.0);
    assert!(a.intersects(&b));
    assert_eq!(
        a.intersection(&b).unwrap(),
        Aabb::new([1.0, 3.0], [2.0, 4.0]).unwrap()
    );
    assert_eq!(a.union(&b), Aabb::new([0.0, 0.0], [5.0, 6.0]).unwrap());
    assert!(!a.contains_point(&[2.1, 0.0]));
    assert!(a.contains_point(&[2.0, 4.0]));
    assert_eq!(a.distance_squared_to_point(&[5.0, 8.0]), 25.0);
    assert_eq!(
        a.distance_squared_to_aabb(&Aabb::new([4.0, 7.0], [5.0, 8.0]).unwrap()),
        13.0
    );
    assert!(a.expanded(-1.0).is_err());
    assert!(Aabb::point([1.0, 2.0]).unwrap().is_point());
}

#[test]
fn ray_edges_are_inclusive_and_finite() {
    let ray = Ray::new([0.0, 0.0], [2.0, 0.0]).unwrap();
    assert_eq!(ray.direction, [1.0, 0.0]);
    let box_ = Aabb::new([2.0, -1.0], [3.0, 1.0]).unwrap();
    let (distance, normal) = ray.intersect_aabb(&box_, 10.0).unwrap();
    assert_eq!(distance, 2.0);
    assert_eq!(normal, [-1.0, 0.0]);
    assert!(ray.intersect_aabb(&box_, 1.9).is_none());
    let inside = Ray::new([2.5, 0.0], [1.0, 0.0]).unwrap();
    assert_eq!(
        inside.intersect_aabb(&box_, 10.0).unwrap(),
        (0.0, [0.0, 0.0])
    );
    let parallel = Ray::new([0.0, 3.0], [1.0, 0.0]).unwrap();
    assert!(parallel.intersect_aabb(&box_, 10.0).is_none());
    assert!(Ray::new([0.0, 0.0], [0.0, 0.0]).is_err());
    assert!(ray.intersect_aabb(&box_, f64::INFINITY).is_none());
}

#[test]
fn plane_and_frustum_boundaries() {
    let plane = Plane::new([0.0, 2.0], -4.0).unwrap();
    assert_eq!(plane.normal(), [0.0, 1.0]);
    assert_eq!(plane.signed_distance(&[0.0, 2.0]), 0.0);
    assert!(plane.intersects_aabb(&Aabb::new([0.0, 1.0], [1.0, 3.0]).unwrap()));
    assert!(!plane.intersects_aabb(&Aabb::new([0.0, 0.0], [1.0, 1.9]).unwrap()));
    let frustum = Frustum::new(vec![
        Plane::from_point_normal([0.0, 0.0], [1.0, 0.0]).unwrap(),
        Plane::from_point_normal([10.0, 0.0], [-1.0, 0.0]).unwrap(),
    ])
    .unwrap();
    assert!(frustum.contains_point(&[5.0, 5.0]));
    assert!(!frustum.contains_point(&[11.0, 5.0]));
    assert!(Frustum::<2>::new(Vec::new()).is_err());
}

#[test]
fn duplicate_insert_replaces_and_update_requires_an_entry() {
    let mut grid = SpatialGrid::<2, u64>::new(GridConfig::default()).unwrap();
    let first = Aabb::new([0.0, 0.0], [1.0, 1.0]).unwrap();
    let second = Aabb::new([10.0, 10.0], [11.0, 11.0]).unwrap();
    assert_eq!(grid.insert(1, first), None);
    assert_eq!(grid.insert(1, second), Some(first));
    assert!(grid.query(first).is_empty());
    assert_eq!(grid.query(second), vec![1]);
    assert_eq!(grid.update(2, second), Err(SpatialError::UnknownId));
    assert_eq!(grid.update(1, first), Ok(second));
    assert_eq!(grid.remove(1), Some(first));
    assert_eq!(grid.remove(1), None);
    assert!(grid.is_empty());
}

#[test]
fn oversized_items_remain_queryable_with_bounded_cells() {
    let config = GridConfig::new(1.0, 4, 10).unwrap();
    let mut grid = SpatialGrid::<2, u64>::new(config).unwrap();
    grid.insert(1, Aabb::new([-1e9, -1e9], [1e9, 1e9]).unwrap());
    grid.insert(2, Aabb::new([0.0, 0.0], [0.5, 0.5]).unwrap());
    assert_eq!(grid.large_items(), 1);
    assert_eq!(grid.query(Aabb::point([0.25, 0.25]).unwrap()), vec![1, 2]);
    assert_eq!(grid.query(Aabb::point([999.0, 999.0]).unwrap()), vec![1]);
    assert!(grid.memory().approximate_bytes > 0);
}

#[test]
fn negative_and_large_world_coordinates_are_supported() {
    let mut grid = SpatialGrid::<2, u64>::new(GridConfig::new(8.0, 64, 10_000).unwrap()).unwrap();
    grid.insert(
        1,
        Aabb::new([-1_000_000.0, -1_000_000.0], [-999_990.0, -999_990.0]).unwrap(),
    );
    assert_eq!(grid.query_point([-999_995.0, -999_995.0]), vec![1]);
    let ray = Ray::new([-1_000_100.0, -999_995.0], [1.0, 0.0]).unwrap();
    assert_eq!(grid.first_ray(&ray, 1_000.0).unwrap().id, 1);
}

#[test]
fn ray_query_matches_brute_force_with_negative_directions() {
    let items = vec![
        (1, Aabb::new([-5.0, -1.0], [-4.0, 1.0]).unwrap()),
        (2, Aabb::new([4.0, -2.0], [5.0, 2.0]).unwrap()),
        (3, Aabb::new([0.0, 4.0], [1.0, 5.0]).unwrap()),
    ];
    for grid_cell in [0.5, 2.0, 100.0] {
        let grid = grid_for(&items, grid_cell);
        let ray = Ray::new([10.0, 0.0], [-1.0, 0.0]).unwrap();
        let hits = grid.cast_ray(&ray, 30.0);
        let mut expected: Vec<(f64, u64)> = items
            .iter()
            .filter_map(|(id, bounds)| {
                ray.intersect_aabb(bounds, 30.0)
                    .map(|(distance, _)| (distance, *id))
            })
            .collect();
        expected.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        assert_eq!(
            hits.iter()
                .map(|hit| (hit.distance, hit.id))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(grid.first_ray(&ray, 30.0).unwrap().id, 2);
    }
}

#[test]
fn empty_and_degenerate_queries_are_explicit() {
    let grid = SpatialGrid::<3, u64>::new(GridConfig::default()).unwrap();
    assert!(grid.query_point([0.0, 0.0, 0.0]).is_empty());
    assert!(grid.nearest([0.0, 0.0, 0.0], 10.0).is_none());
    assert!(
        grid.cast_ray(&Ray::new([0.0; 3], [1.0, 0.0, 0.0]).unwrap(), 10.0)
            .is_empty()
    );
    assert!(grid.query_radius([0.0; 3], f64::NAN).is_empty());
    assert!(grid.query_radius([0.0; 3], -1.0).is_empty());
    assert!(grid.bounds().is_none());
}

#[test]
fn rebuild_changes_cell_policy_without_losing_items() {
    let mut grid = SpatialGrid::<2, u64>::new(GridConfig::new(1.0, 20_000, 1000).unwrap()).unwrap();
    let bounds = Aabb::new([0.0, 0.0], [100.0, 100.0]).unwrap();
    grid.insert(1, bounds);
    assert_eq!(grid.occupied_cells(), 101 * 101);
    grid.rebuild(GridConfig::new(16.0, 4, 1000).unwrap())
        .unwrap();
    assert_eq!(grid.large_items(), 1);
    assert_eq!(grid.query(bounds), vec![1]);
    assert_eq!(grid.get(1), Some(bounds));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn query_matches_brute_force_for_moving_items_2d(
        items in random_box_items::<2>(40, 200.0),
        queries in proptest::collection::vec(random_box::<2>(250.0), 30),
        updates in proptest::collection::vec((0usize..40, random_box::<2>(200.0)), 30),
        cell_size in 1.0f64..64.0,
    ) {
        let mut items = items;
        let mut grid = grid_for(&items, cell_size);
        for (index, bounds) in updates {
            items[index].1 = bounds;
            grid.update(index as u64, bounds).unwrap();
        }
        for query in queries {
            prop_assert_eq!(grid.query(query), brute_query(&items, &query));
        }
        prop_assert_eq!(grid.len(), items.len());
    }

    #[test]
    fn query_matches_brute_force_for_removals_3d(
        items in random_box_items::<3>(50, 100.0),
        remove_flags in proptest::collection::vec(any::<bool>(), 50),
        queries in proptest::collection::vec(random_box::<3>(150.0), 20),
        cell_size in 1.0f64..32.0,
    ) {
        let mut items = items;
        let mut grid = grid_for(&items, cell_size);
        for (index, remove) in remove_flags.iter().enumerate() {
            if *remove {
                items[index].1 = Aabb::new([f64::MAX, f64::MAX, f64::MAX], [f64::MAX, f64::MAX, f64::MAX]).unwrap();
                grid.remove(index as u64);
            }
        }
        items.retain(|(_, bounds)| bounds.min != [f64::MAX; 3]);
        for query in queries {
            prop_assert_eq!(grid.query(query), brute_query(&items, &query));
        }
    }

    #[test]
    fn radius_and_nearest_match_brute_force_3d(
        items in random_box_items::<3>(60, 100.0),
        centers in proptest::collection::vec(
            proptest::collection::vec(-120.0f64..120.0, 3), 20
        ),
        radius in 0.0f64..50.0,
        cell_size in 1.0f64..32.0,
    ) {
        let grid = grid_for(&items, cell_size);
        for center in centers {
            let center: [f64; 3] = center.try_into().unwrap();
            prop_assert_eq!(
                grid.query_radius(center, radius),
                brute_radius(&items, center, radius)
            );
            let mut expected = None;
            for (id, bounds) in &items {
                let distance = bounds.distance_squared_to_point(&center);
                if distance <= radius * radius {
                    let candidate = (distance, *id);
                    if expected.is_none_or(|current: (f64, u64)| {
                        candidate.0.total_cmp(&current.0).is_lt()
                            || (candidate.0 == current.0 && candidate.1 < current.1)
                    }) {
                        expected = Some(candidate);
                    }
                }
            }
            match (grid.nearest(center, radius), expected) {
                (Some(found), Some((distance, id))) => {
                    prop_assert_eq!(found.id, id);
                    prop_assert!((found.distance * found.distance - distance).abs() < 1e-9);
                }
                (None, None) => {}
                other => prop_assert!(false, "nearest mismatch: {:?}", other),
            }
        }
    }

    #[test]
    fn rays_match_brute_force_2d(
        items in random_box_items::<2>(50, 100.0),
        origins in proptest::collection::vec(
            proptest::collection::vec(-150.0f64..150.0, 2), 25
        ),
        directions in proptest::collection::vec(
            proptest::collection::vec(-1.0f64..1.0, 2), 25
        ),
        max_distance in 1.0f64..300.0,
        cell_size in 1.0f64..32.0,
    ) {
        let grid = grid_for(&items, cell_size);
        for (origin, direction) in origins.into_iter().zip(directions) {
            let origin: [f64; 2] = origin.try_into().unwrap();
            let direction: [f64; 2] = direction.try_into().unwrap();
            let Ok(ray) = Ray::new(origin, direction) else { continue };
            let mut expected: Vec<(f64, u64)> = items.iter().filter_map(|(id, bounds)| {
                ray.intersect_aabb(bounds, max_distance).map(|(distance, _)| (distance, *id))
            }).collect();
            expected.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let actual: Vec<(f64, u64)> = grid.cast_ray(&ray, max_distance)
                .into_iter().map(|hit| (hit.distance, hit.id)).collect();
            prop_assert_eq!(actual, expected);
        }
    }

    #[test]
    fn frustum_query_matches_brute_force_3d(
        items in random_box_items::<3>(40, 100.0),
        region in random_box::<3>(150.0),
        cell_size in 1.0f64..32.0,
    ) {
        let grid = grid_for(&items, cell_size);
        let frustum = Frustum::new(vec![
            Plane::from_point_normal([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap(),
            Plane::from_point_normal([120.0, 0.0, 0.0], [-1.0, 0.0, 0.0]).unwrap(),
            Plane::from_point_normal([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap(),
            Plane::from_point_normal([120.0, 0.0, 0.0], [0.0, -1.0, 0.0]).unwrap(),
        ]).unwrap();
        let mut expected: Vec<u64> = items.iter()
            .filter(|(_, bounds)| bounds.intersects(&region) && frustum.intersects_aabb(bounds))
            .map(|(id, _)| *id)
            .collect();
        expected.sort_unstable();
        prop_assert_eq!(grid.query_frustum(&frustum, region), expected);
    }
}

#[test]
fn stress_many_items_remain_consistent_with_brute_force() {
    let mut items = Vec::new();
    let mut grid = SpatialGrid::<3, u64>::new(GridConfig::new(8.0, 512, 100_000).unwrap()).unwrap();
    let mut state = 0x1234_5678_9abc_def0u64;
    for id in 0..5_000u64 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = ((state >> 33) % 20_000) as f64 / 10.0 - 1_000.0;
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let y = ((state >> 33) % 20_000) as f64 / 10.0 - 1_000.0;
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let z = ((state >> 33) % 20_000) as f64 / 10.0 - 1_000.0;
        let bounds = Aabb::from_center_half_extents([x, y, z], [2.0, 1.0, 3.0]).unwrap();
        grid.insert(id, bounds);
        items.push((id, bounds));
    }
    for query_index in 0..200u64 {
        let x = query_index as f64 * 3.0 - 300.0;
        let region = Aabb::from_center_half_extents([x, x * 0.5, -x], [40.0, 25.0, 60.0]).unwrap();
        assert_eq!(grid.query(region), brute_query(&items, &region));
    }
    let occupied = grid.occupied_cells();
    let references = grid.cell_references();
    let memory = grid.memory();
    assert_eq!(memory.occupied_cells, occupied);
    assert!(references >= grid.len());
    assert!(memory.approximate_bytes > 0);
}

#[test]
fn output_buffers_are_reused_and_replaced() {
    let mut grid = SpatialGrid::<2, u64>::new(GridConfig::default()).unwrap();
    grid.insert(1, Aabb::new([0.0, 0.0], [1.0, 1.0]).unwrap());
    let mut ids = vec![99, 98];
    grid.query_into(Aabb::new([0.0, 0.0], [1.0, 1.0]).unwrap(), &mut ids);
    assert_eq!(ids, vec![1]);
    let mut hits = Vec::new();
    grid.cast_ray_into(&Ray::new([-1.0, 0.5], [1.0, 0.0]).unwrap(), 5.0, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(
        BTreeSet::from_iter(hits.iter().map(|hit| hit.id)),
        BTreeSet::from([1])
    );
}

#[test]
fn objects_cross_cell_boundaries_and_leave_no_stale_references() {
    let config = GridConfig::new(10.0, 64, 10_000).unwrap();
    let mut grid = SpatialGrid::<2, u64>::new(config).unwrap();
    let first = Aabb::new([8.0, 8.0], [12.0, 12.0]).unwrap();
    let second = Aabb::new([-2.0, -2.0], [2.0, 2.0]).unwrap();
    grid.insert(9, first);
    assert_eq!(grid.occupied_cells(), 4);
    assert_eq!(grid.query_point([9.5, 9.5]), vec![9]);
    grid.update(9, second).unwrap();
    assert_eq!(grid.occupied_cells(), 4);
    assert!(grid.query_point([9.5, 9.5]).is_empty());
    assert_eq!(grid.query_point([-1.5, -1.5]), vec![9]);
    assert_eq!(grid.cell_references(), 4);

    grid.rebuild(GridConfig::new(1.0, 2, 10_000).unwrap())
        .unwrap();
    assert_eq!(grid.large_items(), 1);
    grid.rebuild(GridConfig::new(10.0, 64, 10_000).unwrap())
        .unwrap();
    assert_eq!(grid.large_items(), 0);
    assert_eq!(grid.remove(9), Some(second));
    assert_eq!(grid.occupied_cells(), 0);
    assert_eq!(grid.cell_references(), 0);
    assert!(grid.memory().entries == 0);
}

#[test]
fn repeated_insert_remove_churn_is_reclaimed() {
    let mut grid = SpatialGrid::<3, u64>::new(GridConfig::new(2.0, 128, 10_000).unwrap()).unwrap();
    for round in 0..2_000u64 {
        let id = round % 17;
        let bounds = Aabb::from_center_half_extents(
            [round as f64 % 50.0, (round as f64 * 0.7) % 50.0, 0.0],
            [0.5, 0.5, 0.5],
        )
        .unwrap();
        grid.insert(id, bounds);
        if round.is_multiple_of(3) {
            grid.remove(id);
        }
    }
    for id in 0..17 {
        grid.remove(id);
    }
    assert!(grid.is_empty());
    assert_eq!(grid.occupied_cells(), 0);
    assert_eq!(grid.cell_references(), 0);
    assert_eq!(grid.memory().entries, 0);
}

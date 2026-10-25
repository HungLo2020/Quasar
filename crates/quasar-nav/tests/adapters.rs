#![cfg(any(
    feature = "spatial",
    feature = "tilemap",
    feature = "jobs",
    feature = "navmesh"
))]

#[cfg(feature = "tilemap")]
use quasar_nav::SearchOptions;
use quasar_nav::{Grid, GridCell};

#[cfg(feature = "spatial")]
#[test]
fn spatial_aabbs_block_cells_without_duplicating_physics_queries() {
    let mut grid = Grid::<2>::new([10, 10]).unwrap();
    let bounds = quasar_spatial::Aabb::new([2.2, 3.2], [4.8, 5.7]).unwrap();
    assert_eq!(grid.set_aabb_blocked(bounds, true).unwrap(), 9);
    for y in 3..6 {
        for x in 2..5 {
            assert!(grid.is_blocked(GridCell::new2(x, y)));
        }
    }
    assert_eq!(grid.set_aabb_blocked(bounds, false).unwrap(), 9);
    assert!(!grid.is_blocked(GridCell::new2(3, 4)));
}

#[cfg(feature = "tilemap")]
#[test]
fn tilemap_adapter_consumes_metadata_without_a_renderer() {
    use quasar_tilemap::{Chunk, Layer, LayerContent, Map, Runtime, Tileset};
    let cells = vec![0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0];
    let map = Map {
        quasar_version: 1,
        tile_width: 16,
        tile_height: 16,
        width: 4,
        height: 3,
        infinite: false,
        background: [0; 4],
        tilesets: vec![Tileset {
            first_gid: 1,
            name: "tiles".into(),
            image: "tiles.png".into(),
            image_width: 16,
            image_height: 16,
            tile_width: 16,
            tile_height: 16,
            columns: 1,
            tile_count: 1,
            margin: 0,
            spacing: 0,
            tiles: Default::default(),
            properties: vec![],
            metadata: Default::default(),
        }],
        layers: vec![Layer {
            id: 1,
            name: "nav".into(),
            visible: true,
            opacity: 1.0,
            offset: [0.0, 0.0],
            parallax: [1.0, 1.0],
            tint: [255; 4],
            content: LayerContent::Tiles {
                chunks: vec![Chunk {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 3,
                    cells,
                }],
            },
            properties: vec![],
            metadata: Default::default(),
        }],
        properties: vec![],
        metadata: Default::default(),
    };
    let runtime = Runtime::new(map).unwrap();
    let grid = quasar_nav::grid_from_tilemap(&runtime, 1, |gid| gid == 1).unwrap();
    assert_eq!(grid.dimensions(), [4, 3]);
    assert!(grid.is_blocked(GridCell::new2(1, 1)));
    assert!(grid.is_blocked(GridCell::new2(2, 1)));
    assert!(!grid.is_blocked(GridCell::new2(0, 0)));
    let path = grid
        .find_path(
            GridCell::new2(0, 0),
            GridCell::new2(3, 2),
            SearchOptions::default(),
        )
        .path()
        .unwrap();
    assert_eq!(path.nodes.last(), Some(&GridCell::new2(3, 2)));
}

#[cfg(feature = "jobs")]
#[test]
fn grid_paths_can_run_on_a_manual_job_pool() {
    use quasar_jobs::JobPool;
    let grid = Grid::<2>::new([16, 16]).unwrap();
    let pool = JobPool::manual();
    let handle = quasar_nav::spawn_grid_path(
        &pool,
        grid,
        GridCell::new2(0, 0),
        GridCell::new2(15, 15),
        SearchOptions::default(),
    )
    .unwrap();
    pool.run_all();
    let path = handle.wait().unwrap().path().unwrap();
    assert_eq!(path.nodes.len(), 16);
    pool.shutdown().unwrap();
}

#[cfg(feature = "navmesh")]
#[test]
fn navmesh_adapter_returns_world_points() {
    let mesh = quasar_nav::navmesh_support::mesh_from_triangles(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 1.0],
        ],
        vec![[0, 1, 4], [4, 3, 0], [1, 2, 5], [5, 4, 1]],
    )
    .unwrap();
    let path = quasar_nav::navmesh_support::find_path(
        &mesh,
        [0.0, 1.0, 0.0],
        [1.5, 0.25, 0.5],
        quasar_nav::navmesh_support::navmesh::NavQuery::Accuracy,
        quasar_nav::navmesh_support::navmesh::NavPathMode::MidPoints,
    )
    .unwrap();
    assert!(!path.is_empty());
    assert!(path.iter().flatten().all(|value| value.is_finite()));
}

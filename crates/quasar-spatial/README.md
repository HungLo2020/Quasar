# quasar-spatial

Renderer-independent spatial indexing and geometric queries for 2D and 3D
worlds. It owns no ECS, entity IDs, scene graph, renderer or window.

```toml
quasar-spatial = { path = "crates/quasar-spatial" }
```

```rust
use quasar_spatial::{Aabb, GridConfig, Ray, SpatialGrid};

let mut grid = SpatialGrid::<2, u32>::new(GridConfig::new(8.0, 256, 100_000)?)?;
grid.insert(7, Aabb::new([0.0, 0.0], [2.0, 2.0])?);
grid.insert(8, Aabb::new([20.0, 0.0], [22.0, 2.0])?);

assert_eq!(grid.query(Aabb::point([1.0, 1.0])?), vec![7]);
assert_eq!(grid.query_radius([3.0, 1.0], 3.0), vec![7]);
assert_eq!(grid.nearest([3.0, 1.0], 10.0)?.id, 7);
let ray = Ray::new([-5.0, 1.0], [1.0, 0.0])?;
assert_eq!(grid.first_ray(&ray, 20.0)?.id, 7);
# Ok::<(), quasar_spatial::SpatialError>(())
```

## APIs

- `Aabb<N>`, `Ray<N>`, `Plane<N>`, `Frustum<N>` with finite/inclusive
  validation and 2D/3D aliases.
- `SpatialGrid<N, T>`: deterministic spatial hash for any `T: Copy + Ord`.
  Insert/replace/update/remove, point/AABB/radius/frustum queries, nearest
  candidates and generalized-DDA ray queries.
- Ordered iteration and result ordering by key; tie-breaks use distance then
  key.
- Bounded-work configuration: oversized items move to a linearly tested list
  and huge query regions fall back to a linear scan.
- `GridMemory` reports entries, occupied cells, references and approximate
  retained bytes.

## Determinism, precision and bounds

- `f64` coordinates; negative and large-world positions are supported. Cell
  indices are checked `i64` conversions. Games needing camera-relative worlds
  should apply that policy before insertion.
- Results never expose `HashMap` iteration order.
- An AABB that would cover more than `max_cells_per_item` cells is stored in
  the large-item list, so insertion cannot exhaust memory.
- Ray queries use a generalized DDA over grid cells, not a scan of the whole
  index. Degenerate, non-finite and zero-direction inputs are rejected or
  return no results.

## Measured baseline

`cargo run --release -p quasar-spatial --example spatial_benchmark` on the
development host (AMD/Intel CPU, rustc 1.94, release profile), 20,000 2D boxes,
cell size 8:

| Operation | Samples | Mean | p50 | p99 | Max |
|---|---:|---:|---:|---:|---:|
| Build | 20,000 inserts | 13.7 ms total | — | — | — |
| AABB query | 2,000 | 17.5 µs | 8.9 µs | 15.5 µs | 27.8 µs |
| First ray | 2,000 | 8.2 µs | 8.2 µs | 14.3 µs | 24.1 µs |
| Brute-force AABB | 200 | 73.1 µs | — | — | — |
| Moving update | 5,000 | 1.3 µs | 1.1 µs | 2.4 µs | — |

Approximate retained memory after build: 4.16 MB (`GridMemory`), 56k occupied
cells, 63k references. These are single-host observations, not portable
guarantees. The example prints the raw values so other machines can reproduce
them.

## Verification

```sh
cargo test -p quasar-spatial
cargo clippy -p quasar-spatial --all-targets -- -D warnings
cargo run --release -p quasar-spatial --example spatial_benchmark
```

Tests compare randomized inserts, moving updates, removals, AABB/radius/nearest
and ray/frustum queries against brute force in 2D and 3D, including negative
coordinates, degenerate boxes, oversized items, deterministic ordering and a
5,000-item stress case.

## Limitations

- This is a spatial hash plus brute-force fallbacks, not a hierarchical BVH.
  Dense scenes with highly non-uniform object sizes may benefit more from a
  BVH; Rapier already supplies one internally for physics queries.
- Frustum tests are conservative (plane/AABB).
- All queries and mutations are single-threaded. Parallel snapshot queries are
  left to callers (the grid is `Send + Sync` when `T` is).
- No serialization feature is provided; callers that persist indexes should
  store their own item bounds and rebuild.

See [spatial architecture](../../docs/spatial.md).

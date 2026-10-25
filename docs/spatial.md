# Spatial indexing and geometric queries

`quasar-spatial` addresses Quasar issue **#37**. It is a dependency-free,
headless library for culling, collision candidate selection, AI proximity,
network interest management and any other system that needs fast geometric
queries without owning a scene, ECS or renderer.

## Architecture

```text
Aabb / Ray / Plane / Frustum (f64, 2D and 3D)
              │
              ▼
SpatialGrid<N, T: Copy + Ord>
  ├─ deterministic BTreeMap<T, Entry>
  ├─ spatial hash: HashMap<[i64; N], Vec<T>>
  └─ bounded large-item list + linear query fallback
```

- **Keys are caller-defined.** No entity ID, ECS handle or renderer type is
  assumed. `T` must be ordered so results can be stable.
- **Coordinates are `f64`.** Negative and large-world coordinates work. Cell
  indices are explicitly range-checked before conversion to `i64`.
- **Deterministic results.** Public query results are sorted by key, or by
  distance then key for nearest/ray results. Internal hash iteration never
  leaks into output.
- **Bounded memory and work.** An item covering more than
  `max_cells_per_item` cells is kept in a linear large-item list. A query region
  covering more than `max_query_cells` cells falls back to a linear entry scan.
  Correctness is preserved; only the cost model changes.
- **Ray traversal is a generalized DDA** over the grid, not a full-index scan.
  Zero direction components are handled by the slab test and traversal.
- **Frustums are half-space lists.** AABB tests are conservative: a false
  negative is not possible, but boxes just outside a corner may be reported.
  Callers provide a bounded region so the index does not need an infinite
  frustum AABB.

## Public API summary

| Type | Purpose |
|---|---|
| `Aabb<N>` | Inclusive bounds, intersection/union, distance, expansion, validation |
| `Ray<N>` | Normalized ray, slab AABB intersection with entry normal |
| `Plane<N>` | Unit-normal half-space, conservative AABB test |
| `Frustum<N>` | Non-empty half-space list, point/AABB containment |
| `GridConfig` | Cell size, per-item cell cap, query-cell cap |
| `SpatialGrid<N, T>` | Insert/update/remove, ordered iteration, queries |
| `GridMemory` | Entries, cells, references, large items, approximate bytes |

Queries: `query`, `query_into`, `query_point`, `query_radius`,
`query_radius_into`, `query_frustum`, `query_frustum_into`, `nearest`,
`cast_ray`, `cast_ray_into`, `first_ray`.

## Static and moving objects

There is intentionally no separate static/dynamic API: both are ordinary
inserts. A static layout is inserted once; moving objects use `update`, which
removes the old cell references before inserting the new ones. `insert` replaces
an existing key and returns the previous bounds. `update` returns
`UnknownId` when the key was not present and leaves the index unchanged.

## Large-world and precision policy

The grid uses absolute world coordinates in `f64`. At ordinary game scales this
gives more precision than an `f32` world. Extremely large coordinates still
lose absolute precision; games with origin shifting or camera-relative worlds
should shift coordinates before indexing. The grid does **not** silently wrap,
normalize or rebase coordinates.

The cell size is fixed for the lifetime of a grid. `rebuild(config)` reclassifies
every item under a new configuration and is the intended way to change cell
size.

## Performance

Measured with `cargo run --release -p quasar-spatial --example spatial_benchmark`
on the development host: 20,000 boxes, cell size 8, 2,000 queries, 2,000 rays and
5,000 moving updates.

| Operation | Mean | p50 | p99 | Max |
|---|---:|---:|---:|---:|
| AABB query | 17.5 µs | 8.9 µs | 15.5 µs | 27.8 µs |
| First ray | 8.2 µs | 8.2 µs | 14.3 µs | 24.1 µs |
| Brute-force query | 73.1 µs | — | — | — |
| Moving update | 1.3 µs | 1.1 µs | 2.4 µs | — |

The build took 13.7 ms and retained approximately 4.16 MB by `GridMemory`
accounting. Results depend on object size distribution, query size, cache state
and hardware. The example is a reproducible local probe, not a cross-machine
benchmark suite. Memory accounting excludes allocator metadata and hash-map
overhead, so treat it as a lower-bound estimate.

## Integration points

- **Rapier / `quasar-physics`:** with its optional `spatial` feature,
  `quasar-physics` converts collider AABBs into this crate's `Aabb` and can
  build a grid keyed by collider handles. Rapier's BBVH remains authoritative
  for physics queries; the spatial grid is for caller-defined layers.
- **Simulation:** indexing is independent of tick timing. Games can rebuild or
  update after their own fixed step; results are deterministic for a given
  sequence of mutations.
- **Rendering:** `Frustum` accepts half-spaces extracted from any view
  projection. No matrix or renderer dependency is imposed.
- **Networking / AI:** caller keys can be entity, player or cell IDs.

## Acceptance criteria for #37

- [x] Reusable AABB/shape queries and a measured spatial-hash baseline.
- [x] Static/moving insert, remove and update.
- [x] Region, radius and frustum searches and closest candidates.
- [x] Deterministic/stable result iteration.
- [x] 2D/3D support and explicit large-world precision policy.
- [x] No ECS, entity-ID, renderer or world-origin assumptions.
- [x] Optional `quasar-physics` integration without a reverse dependency.
- [x] Property-based brute-force oracle tests for randomized edge cases and
  moving objects.
- [x] Build/update/query/ray/memory benchmark example.
- [x] Headless API tests and ordinary Cargo usage.
- [x] Independent crate with no mandatory runner, editor, Qt, ECS or GPU.

## Limitations and future work

- A spatial hash is a good default for homogeneous object sizes. A hierarchical
  BVH may be preferable for highly unbalanced scenes. The issue permits adding
  one based on workload evidence; no second implementation was added without
  evidence that this baseline is insufficient.
- Queries are single-threaded. Read-only concurrent access is possible through
  shared references, but there is no striped or lock-free parallel query API.
- AABB filtering is conservative for frustums and non-axis-aligned shapes.
- The large-item list is linear. Many huge objects degrade to brute force.

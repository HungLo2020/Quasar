# quasar-nav

Reusable A*/Dijkstra pathfinding over caller-owned graphs, 2D grids, 3D voxel
grids, optional tilemap metadata and an optional `navmesh` adapter. It owns no
agents, steering, ECS, renderer or global world.

```toml
quasar-nav = { path = "crates/quasar-nav" }
# Optional layers
# features = ["jobs", "spatial", "tilemap", "navmesh"]
```

## Core APIs

- `Graph` + `Neighbor`: generic caller-owned weighted graph.
- `astar`, `dijkstra`: deterministic open-set tie-breaking, cost/visited limits
  and optional atomic cancellation.
- `Grid<N>` / `GridCell<N>`: N-dimensional grid with per-cell costs, diagonal and
  corner-cutting policy, dynamic blocking, version invalidation and ray-like
  line-of-sight.
- `find_path`, `find_path_dijkstra`, `find_smoothed_path`: grid searches.
- `smooth_path`: generic greedy string-pulling with caller line-of-sight.
- `spawn_grid_path` (`jobs`): optional background search on `quasar-jobs`.
- `grid_from_tilemap` (`tilemap`): consume tile GIDs from `quasar-tilemap`
  without enabling its renderer.
- `set_aabb_blocked` (`spatial`): convert `quasar-spatial` AABBs into blocked
  cells.
- `navmesh_support` (`navmesh`): thin path/query wrapper over the `navmesh`
  crate.

```rust
use quasar_nav::{Grid, GridCell, SearchOptions};
let mut grid = Grid::<2>::new([16, 16])?;
grid.set_blocked(GridCell::new2(8, 7), true)?;
let path = grid.find_path(
    GridCell::new2(0, 0),
    GridCell::new2(15, 15),
    SearchOptions::default(),
).path().unwrap();
assert_eq!(path.nodes.first(), Some(&GridCell::new2(0, 0)));
# Ok::<(), quasar_nav::NavError>(())
```

## Search behavior

- A* uses octile distance for 2D grids and Chebyshev-max for N>2; arbitrary
  graphs supply their own heuristic.
- Ties are broken by node order then insertion sequence, so results are stable.
- Negative/non-finite edge costs and invalid heuristics are ignored/converted to
  zero; costs must be finite and non-negative.
- A blocked start/goal returns `NotFound` without searching.
- `Path::visited` reports closed nodes; `SearchOptions { max_visited, max_cost,
  cancel }` bounds adverse searches.
- Dynamic changes increment `Grid::version`; agents can replan only when the
  version changes.

## Tilemap, spatial and physics integration

- `grid_from_tilemap(runtime, layer, predicate)` reads `quasar-tilemap` tiles and
  never pulls in its renderer feature.
- `set_aabb_blocked` marks cells covered by `quasar-spatial::Aabb<2>`.
- Physics Playground converts Rapier/Quasar-Physics collider AABBs into
  `quasar-spatial` bounds, blocks grid cells, replans agents and shows debug
  paths. `quasar-nav` does not depend on Rapier or duplicate its broad phase.
- Steering and agent bodies remain game-owned; Physics Playground uses
  kinematic Rapier bodies for its visual agents.

## Optional navmesh

The optional `navmesh` feature wraps the mature `navmesh` crate for triangle
mesh path queries. Quasar does not implement navmesh generation; use a Recast/
external authoring pipeline and pass vertices/triangles through
`mesh_from_triangles`.

## Measured baseline

`cargo run --release -p quasar-nav --example nav_benchmark` on the development
host, 256×256 grids, 20 paths per reachable case:

| Case | Found | Mean | p50 | p99 | Visited nodes |
|---|---:|---:|---:|---:|---:|
| Easy (open) | 20/20 | 0.36 ms | 0.35 ms | 0.48 ms | 252 |
| Typical (12% blocked) | 20/20 | 10.7 ms | 10.2 ms | 12.8 ms | 13,673 |
| Difficult (28% blocked) | 20/20 | 13.0 ms | 12.4 ms | 15.9 ms | 22,167 |
| Unreachable barrier | 0/5 | 24.7 ms | 24.7 ms | 25.0 ms | 32,768 |
| Dijkstra (typical) | 1/1 | 36.5 ms | — | — | 57,722 |

The heuristic bug that previously explored the entire easy grid is fixed: an
open 256×256 path now closes only the 252 nodes on the path. The A* result and
cost match Dijkstra on the same grid while exploring fewer nodes. The random
maps are adverse; single-host observations only.

Allocation probe (same example, counting global allocator; one easy and one
typical search):

| Case | Allocations | Total allocated | Peak live | Dense grid storage |
|---|---:|---:|---:|---:|
| Easy | 47 | 218 KB | 122 KB | 328 KB |
| Typical | 62 | 2.66 MB | 1.61 MB | 328 KB |

The search reuses a caller-owned neighbor buffer; allocations are primarily
open-set/heap growth and score/parent maps. The typical result is not
allocation-dominated at this scale; the dominant cost is graph exploration.

## Verification

```sh
cargo test -p quasar-nav
cargo test -p quasar-nav --all-features
cargo clippy -p quasar-nav --all-targets --all-features -- -D warnings
```

Tests cover generic graph optimality, unreachable goals, dynamic blocking,
weighted costs, corner policy, smoothing, 3D voxel paths, cancellation,
visited limits, random-grid A*/Dijkstra equivalence, multi-agent determinism,
tilemap metadata, spatial AABBs, jobs and navmesh adapters.

## Limitations

- Grids are dense arrays capped at 16M cells; very large worlds need streaming
  chunks or hierarchical graphs (not yet provided).
- No built-in agent steering, local avoidance, flow fields, hierarchical
  pathfinding or navmesh generation.
- Line-of-sight is conservative cell sampling, not exact polygon visibility.
- Path smoothing is greedy and preserves the path endpoints.
- A limited cost makes the search return `LimitReached` when an edge would
  exceed the budget and the goal was not reached.

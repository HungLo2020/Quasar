# Navigation and pathfinding

`quasar-nav` addresses Quasar issue **#34**. It provides general-purpose path
and navigation queries, not an agent AI framework, ECS or steering system.

## Architecture

```text
Graph trait ── astar / dijkstra ── SearchOutcome<Path>
                  │
                  └─ Grid<N, GridCell<N>> ── A* heuristic, costs, dynamic blocking
                        ├─ tilemap adapter (optional, headless)
                        ├─ spatial AABB adapter (optional)
                        ├─ jobs adapter (optional)
                        └─ navmesh adapter (optional, mature `navmesh` crate)
```

Games own graphs or `Grid<N>`, agents, ways to convert world positions to cell
coordinates, and steering. `quasar-nav` returns paths and reason codes.

## Search semantics

- Generic `Graph::neighbors` supplies `Neighbor { node, cost }`.
- A* accepts a heuristic closure; Dijkstra is A* with a zero heuristic.
- 2D grids use an admissible octile heuristic; 3D/multi-dimensional grids use
  Chebyshev-max, which is admissible for the grid's step costs.
- The open set breaks ties by `f64` total, node order then insertion sequence,
  giving deterministic paths for the same graph, options and traversal order.
- Negative/non-finite costs and invalid heuristics are ignored or replaced by
  zero; constructors validate grid dimensions/costs.
- `Path::visited` reports closed nodes; `SearchOptions` bounds visited nodes and
  total cost, and accepts an `Arc<AtomicBool>` cancellation flag. `SearchOutcome` distinguishes `Found`,
  `NotFound`, `Cancelled` and `LimitReached`.

## Grids

`Grid<2>` and `Grid<3>` store blocked flags and per-cell multiplier costs.
They support:

- 4/8-neighbor 2D or 6/26-neighbor 3D movement;
- optional corner cutting (without it, diagonal moves require both adjacent
  axis cells to be free);
- dynamic `set_blocked`/`set_cost` with a monotonically increasing `version`;
- `set_blocked_many` and spatial AABB blocking;
- line-of-sight and greedy string-pulled smoothing.

`Grid` dimensions are validated with a 16M-cell cap. A cell search can be
bounded or cancelled; a blocked start or goal returns `NotFound` immediately.

## Optional adapters

### Tilemap

`grid_from_tilemap(runtime, layer, predicate)` reads tile GIDs through
`quasar-tilemap`'s headless runtime. The dependency is default-feature-free, so
no renderer/GPU/window is pulled in. Game-specific tile semantics stay in the
caller's predicate.

### Spatial and physics

`Grid::<2>::set_aabb_blocked` marks cells covered by a `quasar-spatial` AABB.
Physics Playground computes Rapier collider AABBs, converts them to planar
spatial AABBs, and updates the nav grid every 15 simulation ticks. This keeps
Rapier's broad phase authoritative and uses the grid only for path planning.

### Jobs

`spawn_grid_path(pool, grid, start, goal, options)` submits a deterministic
search to a caller-owned `quasar-jobs` pool. The optional cancellation flag in
`SearchOptions` remains usable inside the job.

### Navmesh

The `navmesh` feature wraps the established `navmesh` crate. Quasar does not
implement Recast or navmesh generation. `mesh_from_triangles`, `find_path`, and
`find_path_custom` adapt arrays and world points; use an external generated mesh
for production geometry.

## Measured baseline

`cargo run --release -p quasar-nav --example nav_benchmark` on the development
host, 256×256 grids:

| Case | Found | Mean | p50 | p99 | Mean visited |
|---|---:|---:|---:|---:|---:|
| Easy (open) | 20/20 | 0.36 ms | 0.35 ms | 0.48 ms | 252 |
| Typical (12% blocked) | 20/20 | 10.7 ms | 10.2 ms | 12.8 ms | 13,673 |
| Difficult (28% blocked) | 20/20 | 13.0 ms | 12.4 ms | 15.9 ms | 22,167 |
| Unreachable barrier | 0/5 | 24.7 ms | 24.7 ms | 25.0 ms | 32,768 |
| Dijkstra (typical) | 1/1 | 36.5 ms | — | — | 57,722 |

The open-grid case closes only the path nodes; the earlier heuristic bug that
expanded the whole grid was fixed. A* matches Dijkstra's cost while visiting
fewer nodes. The random maps are adverse; single-host observations only.

Allocation probe (same example, counting global allocator; one easy and one
typical search):

| Case | Allocations | Total allocated | Peak live | Dense grid storage |
|---|---:|---:|---:|---:|
| Easy | 47 | 218 KB | 122 KB | 328 KB |
| Typical | 62 | 2.66 MB | 1.61 MB | 328 KB |

The search reuses a caller-owned neighbor buffer; allocations are primarily
open-set/heap growth and score/parent maps. The typical result is not
allocation-dominated at this scale; the dominant cost is graph exploration.

## Physics Playground integration

- Two AI agents have kinematic Rapier bodies and animated idle/move/alert state
  machines.
- A 32×32 navigation grid is rebuilt from static arena boxes and dynamic prop
  AABBs only when prop poses change, at most once every five ticks; version
  changes trigger replanning.
- Paths are smoothed, agents follow cell centers at a fixed speed, and `N`
  toggles debug path cubes.
- Headless tests assert movement, nav generation changes, debug toggling and
  deterministic replay hash behavior.

## Acceptance criteria for #34

- [x] Generic A*/Dijkstra over caller-owned graphs and 2D/3D grids.
- [x] Custom costs, heuristics, bounded searches and cancellation.
- [x] Dynamic blockers and invalidation version.
- [x] Tilemap metadata adapter without a renderer.
- [x] Optional navmesh support through `navmesh`, without implementing
  generation.
- [x] Path smoothing and multi-agent fixtures.
- [x] Benchmarks for large/adverse grids, headless and dependency-light.
- [x] Physics/animation demo integration and visual debug paths.
- [x] No SDL/winit/ECS/GPU requirements in the crate.
- [x] No modifications to other game repositories.

## Limitations

- Dense grids, not hierarchical/streaming navigation.
- No steering, local avoidance, flow fields or dynamic replanning policy; the
  game decides when to replan.
- Line-of-sight is conservative cell sampling; smoothed paths are not guaranteed
  to be globally optimal, only collision-free under that predicate.
- Navmesh generation is out of scope and delegated to `navmesh`/external tools.
- Cost/visited limits return `LimitReached` rather than partial paths.

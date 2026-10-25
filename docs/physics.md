# Physics and collision integration

`quasar-physics` addresses Quasar issue **#33**. It is a thin, optional adapter
over **Rapier 0.36** (`rapier2d` / `rapier3d`), not a custom collision or
rigid-body implementation.

## Why Rapier

Rapier already provides a mature broad phase, narrow phase, impulse solver,
joints, CCD, character controllers, query pipelines, filtering and event
channels. Reimplementing any of those would duplicate a well-tested engine and
risk incompatible behavior. Quasar's layer therefore focuses on:

1. caller-owned world construction;
2. a bounded fixed-step policy;
3. collision-only worlds;
4. event collection through Rapier's own handler;
5. ergonomic array-based raycasts;
6. optional `quasar-spatial` AABB/index conversion;
7. preserving direct access to every Rapier API.

No renderer, window, asset, ECS, editor or application runner is a dependency.

## Features and default build

| Feature | Default | Effect |
|---|---|---|
| `dim2` | yes | Compile Rapier 2D integration |
| `dim3` | no | Compile Rapier 3D integration (opt in) |
| `dynamics` | yes | Compile the Quasar fixed-step wrappers and event stepping |
| `events` | yes | Collect Rapier collision/contact-force events |
| `spatial` | no | `quasar-spatial` AABB conversion and collider index construction |
| `enhanced-determinism` | no | Forward Rapier's cross-run determinism feature |

The default build compiles only Rapier 2D (`rapier2d`/`parry2d`); `rapier3d` is
opt-in. Enable `dim3` to add 3D, and disable `dim2` explicitly for 3D-only
builds so both engines are not compiled. Collision-only use does not require
`dynamics`, `events` or `spatial`.

```sh
cargo tree -p quasar-physics -e normal                     # rapier2d only
cargo tree -p quasar-physics -e normal --no-default-features --features dim2
cargo tree -p quasar-physics -e normal --no-default-features --features dim3
```

The default change is a workspace semver-minor behavior change: existing
`quasar-physics` users keep 2D dynamics and events; 3D users must opt in.

## Crate structure

```text
common:
  StepConfig / WorldConfig<D> / StepReport / PhysicsEvents / PhysicsError
dim2:
  CollisionWorld ── Rapier PhysicsWorld (colliders, broad/narrow phase)
  PhysicsWorld   ── Rapier PhysicsWorld + fixed stepping + event queue
  rapier2d       ── re-exported for advanced use
dim3:
  same structure with rapier3d
```

Both wrappers implement `Deref`/`DerefMut` to Rapier's `PhysicsWorld`. That is
the primary escape hatch: Quasar adds policy and convenience without hiding
Rapier's sets, handles, builders, query pipelines, joints or character
controllers.

### Collision-only path

`CollisionWorld::new()` creates colliders and a query pipeline. `update()` calls
Rapier's `detect_collisions` (broad/narrow phase) without integrating bodies.
`add_collider`, `remove_collider`, ray/shape/point queries, sensors and events
work without the `dynamics` feature. A `CollisionWorld` still uses Rapier's
container type internally, but no rigid-body stepping is performed by the
Quasar API.

### Dynamics path

`PhysicsWorld::new(WorldConfig)` validates gravity and step configuration, sets
`IntegrationParameters::dt`, and exposes:

- `step_fixed()` — exactly one fixed step;
- `advance(frame_delta)` — clamped accumulation with `max_substeps`;
- `drain_events()` — collision/contact-force events;
- `raycast` / `raycast_filtered` — arrays in, `RayHit` out;
- `spatial_index()` — optional `quasar-spatial` snapshot.

The caller owns the world, inserts bodies/colliders with Rapier builders and can
use any Rapier joint, CCD, sensor or character-controller API.

## Stepping and determinism

`StepConfig` defaults to 1/60 s, a 0.25 s maximum frame delta and 8 substeps.
Fixed steps above 0.25 s are rejected. `advance()` reports
`StepReport { steps, simulated_seconds, saturated }`; when the substep cap is
reached, leftover time is discarded to avoid a spiral of death. `saturated` makes
that explicit.

For replay, one game tick should call exactly one `step_fixed()`. Rapier is
deterministic for a fixed build/feature set/platform/step sequence, not across
arbitrary platforms or feature combinations. `enhanced-determinism` forwards
Rapier's stronger floating-point policy at a performance cost. `quasar-sim`
remains responsible for input journals and state hashes; this crate does not
own game snapshots.

## Collision layers, sensors and events

Quasar re-exports Rapier's `Group`, `InteractionGroups`, `InteractionTestMode`,
`QueryFilterFlags`, `ActiveEvents`, `CollisionEvent` and `ContactForceEvent`.

- Collision groups are set on colliders with Rapier's `.collision_groups(...)`.
- Query filtering uses `QueryFilter` (exclude self, sensors, groups, predicate).
- Sensors use `.sensor(true)` and `.active_events(ActiveEvents::COLLISION_EVENTS)`.
- `events` uses Rapier's `ChannelEventCollector` and drains into
  `PhysicsEvents<CollisionEvent, ContactForceEvent>`.

Events are opt-in per collider in Rapier; touching is only reported when at
least one collider has the relevant active-event flag.

## `quasar-spatial` integration

With the `spatial` feature:

```text
Rapier ColliderSet
  → collider_bounds() / spatial_index()
  → quasar_spatial::SpatialGrid<N, u64>
  → region/radius/frustum/nearest/ray queries
```

Keys are `(generation << 32) | index` from Rapier handles. This index is a
snapshot for non-physics systems; Rapier's broad phase remains the physics
authority. There is no reverse dependency: `quasar-spatial` does not depend on
physics.

## Integration recipes

### Tile-based 2D game

Use a `CollisionWorld` or `PhysicsWorld` with static cuboid colliders for tiles
and a dynamic/kinematic body for the player. Queries can use collision groups for
player/enemy/trigger layers. Tilemap-specific collision remains game-owned;
`quasar-physics` does not force a tile representation.

### Voxel world

Insert chunk or voxel colliders as static Rapier colliders; use
`ColliderBuilder::voxels`/trimesh or compound shapes for detailed terrain. Keep
voxel simulation and meshing in the game; use Rapier queries for movement,
raycasts and trigger volumes. The `CollisionWorld` path avoids a dynamics scene
when only queries are needed.

### 3D FPS character

Use Rapier's `KinematicCharacterController` through `Deref`, with a capsule
collider and a filtered query pipeline. Rapier supplies slope/step/snap behavior
and optional collision impulses. The 3D playground demonstrates walking,
jumping, pushing dynamic boxes and raycast interaction.

## Reference demos

- `examples/falling-sand`: cellular sand with static Rapier obstacles, one
  dynamic ball and `quasar-spatial` obstacle/ball queries.
- `examples/physics-playground`: kinematic first-person player, dynamic blocks
  and bouncing balls, collision filtering, a sensor trigger, raycast kicks,
  frustum culling and Rapier + `quasar-spatial` integration.

```sh
cargo run -p falling-sand
cargo run -p falling-sand -- --headless --ticks 300 --autoplay
cargo run -p physics-playground
cargo run -p physics-playground -- --headless --ticks 300 --autoplay
```

## Measured cost

`cargo run --release -p quasar-physics --example physics_benchmark`, 501
bodies/colliders, 600 steps:

| Workload | Mean step | Total |
|---|---:|---:|
| 2D | 89.5 µs | 53.7 ms |
| 3D | 154.8 µs | 92.9 ms |

2,000 array raycasts averaged 0.23 µs (2D) and 0.15 µs (3D). This is a single
host/scene and excludes rendering. Rapier does not expose portable heap totals;
use an allocator profiler for memory measurements. `quasar-spatial` separately
reports approximate retained bytes.

## Acceptance criteria for #33

- [x] Reusable 2D/3D shapes and transforms through Rapier/Parry.
- [x] Overlap, ray and shape queries and collision-only worlds.
- [x] Optional trigger/contact events.
- [x] Collision-only path without renderer/audio/window/GPU or the Quasar
  dynamics wrapper.
- [x] Bounded fixed-step dynamics with explicit determinism caveats.
- [x] Collision filtering/layers and measured scaling.
- [x] Game-defined collision layers, terrain adapters and independent worlds.
- [x] Stable `quasar-spatial` and `quasar-sim` integration points without
  circular dependencies.
- [x] Headless collision/raycast/contact fixtures in 2D and 3D, including
  degenerate/non-finite inputs.
- [x] Documentation for tile, voxel and FPS integration without changing those
  games.
- [x] Demos runnable with ordinary `cargo run`, headless reproduction and native
  Vulkan rendering.

## Limitations

- Rapier's high-level API is evolving; the workspace pins `0.36.x`. Quasar
  re-exports Rapier to keep advanced users on the same version.
- `advance()` is not a replay API.
- Contact-force event content is Rapier's; Quasar collects and exposes it but
  does not reinterpret impact semantics.
- The demos use simple shapes and first-person character control; they are
  reference integrations, not production game frameworks.
- No ECS, scene graph, serialization format or editor owns physics state.

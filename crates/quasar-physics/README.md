# quasar-physics

Thin, optional Quasar integration for Rapier's 2D and 3D collision and
rigid-body engines. Quasar does **not** implement a solver, broad phase,
narrow phase or character controller.

```toml
# Default: Rapier 2D, fixed-step dynamics and events.
quasar-physics = { path = "crates/quasar-physics" }

# Add 3D alongside 2D.
quasar-physics = { path = "crates/quasar-physics", features = ["dim3"] }

# Collision-only, 2D only: no Quasar dynamics wrapper is compiled.
quasar-physics = { path = "crates/quasar-physics", default-features = false, features = ["dim2"] }
```

## Two levels of use

### Collision-only

`dim2::CollisionWorld` and `dim3::CollisionWorld` own a Rapier collider set and
broad/narrow phase. Add standalone colliders, call `update()`, then use Rapier
queries through `Deref` or the array-based `raycast` convenience:

```rust
use quasar_physics::dim2::{CollisionWorld, Pose};
use quasar_physics::rapier2d::prelude::*;

let mut world = CollisionWorld::new();
world.add_collider(ColliderBuilder::cuboid(10.0, 0.5));
world.update();
let hit = world.raycast([0.0, 5.0], [0.0, -1.0], 20.0, true);
assert!(hit.is_some());
# let _ = Pose::translation(0.0, 0.0);
```

No rigid bodies are integrated unless the caller explicitly reaches the raw
Rapier world through `Deref`/`raw_mut()`.

### Explicit 3D shapes and characters

With `dim3`, `Shape3`, `PlacedShape3` and `RigidPose3` validate cuboids, cylinders,
capsules and flattened compounds. `CapsuleController3` wraps Rapier capsule
sweeps/sliding, feet coordinates, clearance queries and ray hits without dynamics.
`Bounds3` shares a render-neutral fitting transform. Optional `model-assets`
consumes decoded model data; it never implicitly generates mesh colliders.
See [3D collision architecture](../../docs/collision-3d.md) and Nightwatch for reuse.

### Fixed-step dynamics

`dim2::PhysicsWorld` and `dim3::PhysicsWorld` wrap Rapier's own `PhysicsWorld`
with:

- validated gravity and a **bounded fixed timestep** (`StepConfig`);
- explicit `step_fixed()` for deterministic simulation ticks;
- optional `advance(frame_delta)` with a substep cap and saturation report;
- collision/contact-force event collection (`events` feature);
- array-based raycast convenience with player/self filtering;
- optional `quasar-spatial` collider-index construction (`spatial` feature).

All Rapier state and methods remain available:

```rust
use quasar_physics::dim3::{PhysicsWorld, WorldConfig};
use quasar_physics::rapier3d::prelude::*;

let mut world = PhysicsWorld::new(WorldConfig::default())?;
world.insert(
    RigidBodyBuilder::fixed(),
    ColliderBuilder::cuboid(10.0, 0.5, 10.0),
);
world.step_fixed();
assert_eq!(world.bodies.len(), 1);
# Ok::<(), quasar_physics::PhysicsError>(())
```

`Deref`/`DerefMut`, `raw()`, `raw_mut()`, and the re-exported `rapier2d` /
`rapier3d` crates give direct access to joints, CCD, character controllers,
query pipelines and every advanced Rapier feature.

## Features

| Feature | Default | Effect |
|---|---|---|
| `dim2` | yes | Compile Rapier 2D integration |
| `dim3` | no | Compile Rapier 3D integration (opt in) |
| `dynamics` | yes | Compile the Quasar fixed-step wrappers and event stepping |
| `events` | yes | Collect Rapier collision/contact-force events via `std::sync::mpsc` |
| `model-assets` | no | CPU model bounds, selected structural instances and explicit containing-cylinder fitting (enables dim3) |
| `spatial` | no | `quasar-spatial` AABB conversion and collider index construction |
| `enhanced-determinism` | no | Forward Rapier's cross-run determinism feature |

The default build compiles only Rapier 2D. Enable `dim3` to add Rapier 3D;
disable `dim2` explicitly for 3D-only builds to avoid compiling both engines.
Collision-only use does not require `dynamics`, `events` or `spatial`.

## Determinism and stepping

`step_fixed()` always uses `StepConfig::fixed_dt`; drive one call per game tick
for replayable simulations. `advance()` is provided for wall-clock frames but
accumulates time, clamps a large frame delta and can discard leftover time at
`max_substeps` (`StepReport::saturated`). Rapier determinism is guaranteed only
for the same build, feature set, platform and step sequence; cross-platform
bitwise equality is not claimed. Use `quasar-sim` for journals and quantized
state hashes.

Fixed steps are limited to `0.25 s` to bound solver work. Invalid gravity,
timesteps and frame deltas return `PhysicsError` instead of panicking.

## Collision filtering and events

Use Rapier `InteractionGroups`/`Group`; they are re-exported from each dimension.
Sensors are ordinary Rapier colliders with `sensor(true)` and
`active_events(ActiveEvents::COLLISION_EVENTS)`. With `events` enabled,
`PhysicsWorld::drain_events()` returns `PhysicsEvents { collisions,
contact_forces }`. `CollisionWorld` also collects events during `update()`.

## Spatial integration

With `features = ["spatial"]`:

- `dimN::collider_bounds(&collider)` returns a `quasar_spatial::Aabb<N>`.
- `dimN::spatial_index(&collider_set)` builds a deterministic grid keyed by a
  stable compact collider key; `spatial_index_with_config` sets cell limits.
- `collider_key` / `collider_from_key` convert handles.
- `aabb_from_rapier` / `aabb_to_rapier` convert AABBs.

Rapier's BBVH remains authoritative for physics queries. The optional index is
for caller-defined culling, interest management or non-Rapier systems;
`quasar-spatial` never depends on `quasar-physics`.

## Measured baseline

`cargo run --release -p quasar-physics --example physics_benchmark` on the
development host, 501 bodies/colliders, 600 fixed steps:

| Dimension | Step mean | Total |
|---|---:|---:|
| 2D | 89.5 µs | 53.7 ms |
| 3D | 154.8 µs | 92.9 ms |

2,000 array raycasts averaged 0.23 µs (2D) and 0.15 µs (3D). These numbers
measure one machine and one scene; they are not cross-platform guarantees.
Rapier does not expose portable heap totals, so memory should be measured with
an allocator-specific profiler.

## Verification

```sh
cargo test -p quasar-physics
cargo test -p quasar-physics --no-default-features --features dim2
cargo test -p quasar-physics --no-default-features --features dim3
cargo test -p quasar-physics --no-default-features --features dim2,dynamics
cargo test -p quasar-physics --no-default-features --features dim2,dim3,dynamics,spatial
cargo clippy -p quasar-physics --all-targets --all-features -- -D warnings
```

Tests cover collision-only queries, moving/removing colliders, filtering, sensor
and contact-force events, fixed-step bounds, gravity/restitution/friction,
determinism, resource removal and randomized drop/ray invariants in both
dimensions.

## Limitations

- Quasar does not wrap every Rapier builder or controller. Advanced use should
  call Rapier directly through the re-exports/Deref instead of waiting for a
  Quasar wrapper.
- `advance()` is frame-rate dependent by design; replay paths should use
  `step_fixed()`.
- `CollisionWorld` still uses Rapier's `PhysicsWorld` storage internally for
  compatibility, but its public API exposes collider-only updates.
- All worlds and handles are caller-owned; there are no global registries.
- Advanced joints/controller policies remain available directly through Rapier;
  the reusable capsule helper intentionally keeps a small fixed policy.

# Explicit 3D collision and Nightwatch

Quasar's collision authority is Rapier through `quasar-physics`. The renderer,
asset service, ECS and scene graph do not own collision state. Games choose
shapes, units, placements and query filters. Nightwatch demonstrates a
collision-only FPS: no dynamic rigid-body integration is needed.

## Reusing the library

```toml
quasar-physics = { version = "0.2.19", default-features = false, features = ["dim3"] }
# Optional CPU model-data fitting, without a decoder or renderer:
# features = ["dim3", "model-assets"]
quasar-nav = { version = "0.2.19", features = ["physics3d"] } # optional
```

`dim3::{Shape3, PlacedShape3, RigidPose3, Bounds3, CapsuleController3}` are
independent of Nightwatch, graphics, audio, ECS and application runners. The
optional `model-assets` feature consumes already decoded
`quasar_assets::model::Model3D`; it does not load files, upload meshes or
register colliders. Ordinary navigation grids do not depend on physics.

```rust,ignore
use quasar_physics::dim3::*;
use quasar_physics::rapier3d::pipeline::QueryFilter;
let mut world = CollisionWorld::new();
let barrel = PlacedShape3 {
    pose: RigidPose3::translated([2., 1., 0.]),
    shape: Shape3::CylinderY { half_height: 1., radius: 0.6 },
};
world.add_collider(barrel.collider()?);
world.update(); // refresh broad/narrow phase after collider changes
let character = CapsuleController3::new(1.85, 0.32, 0.005)?;
let moved = character.move_feet(
    &world, [0., 0.01, 0.], [0.05, 0., 0.], 1. / 60., QueryFilter::default(),
)?;
let hit = world.raycast([0., 1., 0.], [1., 0., 0.], 10., true);
```

Use Rapier's re-exported builders, collision groups, query filters and raw
world APIs for additional shapes, sensors or dynamics. These helpers implement
validation and coordinate conversion; sweeps, contact refinement, sliding and
raycasts all use Rapier/Parry.

## Choosing a collider

| Geometry | Explicit choice | Policy |
|---|---|---|
| Round upright barrel | `CylinderY` | One continuous curved surface, no rectangular corners |
| Crate, wall, panel, post | `Cuboid` | Oriented local box, bevels approximated |
| Table or other hollow prop | `Compound` | Separate top/legs preserve gaps |
| Character | `CapsuleY` / `CapsuleController3` | Rounded feet/head and sliding body |
| Decoration | No collider | Explicit author decision |

`Shape3::Compound` contains local `PlacedShape3` children. Nested compounds
are flattened before constructing Rapier geometry, since Rapier does not
accept nested composite shapes. Validation caps descriptors at 256 and nesting
at eight levels, rejects empty compounds, non-finite inputs and invalid sizes.
Shape errors expose `physics.invalid_shape` for diagnostics.

There is no automatic triangle-mesh generation. Detailed static terrain may
use an explicitly authored Rapier mesh collider; a small prop usually benefits
from a primitive or compound instead. `Bounds3::collider(PrimitiveFit3)` is an
explicit bounds approximation, not a promise that the result encloses every
mesh point. In particular a circular footprint fitted to a square AABB cannot
enclose its corners. `model_cylinder_y` instead finds a **containing** radius
from actual default-pose, instance-transformed vertices. Callers explicitly
select structural instances to exclude loose decorations.

## Coordinates and scale

Quasar/Nightwatch use Y-up, world units treated as meters. `RigidPose3` contains
translation and a unit quaternion in **xyzw** order. Its `matrix()` is
column-major and can be passed to rendering. Collider primitives carry their
actual local dimensions: cuboid half extents, cylinder half height and radius,
capsule half segment length and radius. Total capsule height is
`2 * (half_segment + radius)`.

Collider poses contain no scale or shear. Bake scale into the dimensions;
nonuniform X/Z scale does not produce a circular cylinder or capsule. The
bounds convenience fit rejects unequal X/Z round fits. `Bounds3::fit_matrix`
centers source bounds, scales them into explicitly requested local half
extents, then applies the same rigid pose as the collider. `model_bounds` and
`model_bounds_selected` include imported instance transforms, matching the
renderer default-pose bound calculation. They reject missing/invalid geometry
and non-affine transforms. Very large worlds still need a floating origin or
other precision policy appropriate for f32 Rapier/render coordinates.

`CapsuleController3` accepts **feet** positions; Rapier's centered capsule is
translated up by half its height. It performs a bounded zero-motion Rapier
penetration correction followed by the desired sweep/slide. Deeply embedded
teleports may need multiple updates; valid spawn placement remains game-owned.
Its default policy disables automatic steps and ground snapping. Gravity,
velocity, jump eligibility and camera eye height remain game rules. Filters
apply to overlaps, movement and clearance; exclude sensors explicitly when
using them as nonblocking triggers. The reusable wrapper does not create a
character collider in the world, avoiding self hits and mandatory dynamics.

## Nightwatch ownership and consistency

`examples/nightwatch/src/props.rs` is the authoritative courtyard definition.
Each `Prop` has a stable ID, name, rigid pose, named visual asset/bounds and an
explicit optional collider. `ModelId` chooses an asset by name, independently
of vector order or obstacle dimensions. Reordering props cannot turn a crate
into a barrel. The shipped courtyard has 63 props and 63 static colliders,
including the floor, perimeter walls, gate, panels, posts and lamps that were
previously mostly decorative with respect to collision.

`World` owns a prepared immutable prop set, its Rapier world and baked
navigation graph. Rendering reads `World::props()`. Movement, world bullet
raycasts and navigation query that same Rapier geometry. Collider `user_data`
stores the prop ID, not a model index. `World::set_props` stages validation,
colliders and navigation before replacing anything; failure leaves the active
world/hash untouched and successful replacement clears old NPC paths.
`collision()` and `navigation()` expose read-only views.

The barrel pair's actual old **rendered** uniform dimensions are preserved,
while each barrel now has its own containing cylinder. The workbench fits its
structural `workbench` node, excluding loose paper/tools from the fitting
anchor; its compound top and four legs use that same local reference. Floor
paint and checker shading are cosmetic overlays over one flat floor collider.
Every solid procedural cuboid renders from its actual collider pose/size.
Emissive light positions/colors also come from those same props.

Player and zombie capsule sizes live beside prop definitions. Movement uses
swept capsules, bullets use world-shape raycasts and the same zombie capsule.
A headshot is the hit-height rule within that capsule; animated limbs are not
separate hitboxes. Melee requires capsule clearance to the player, preventing
attacks through thin walls. Zombie render roots follow their collision feet.

F5 still stages GPU/model generations transactionally. Collision definitions
remain explicit game data, so successful image/material reloads do not change
simulation. Changing a model's structure requires updating its authored
collision profile and alignment tests too; reloading an arbitrary changed mesh
does not silently replace colliders. This preserves simulation-only operation.
Nightwatch adopts `dim3` even with `--no-default-features`; headless means no
window/GPU/audio, while the game still needs its chosen collision engine.
No renderer, asset crate or ordinary navigation user gains mandatory physics.

## Navigation and determinism

The optional `quasar_nav::physics3d::CollisionGrid3` bakes a flat XZ graph from
an existing updated collision world and caller-supplied capsule/filter.
Nodes use signed-contact-refined overlaps; **every edge** uses a swept capsule,
so a thin obstacle between free node centers still blocks the edge. The cache
is capped at 262,144 cells. It owns neither physics nor agents. Geometry or
agent-clearance changes require rebaking; no hidden world mutation is watched.

Nightwatch uses a 60×60 grid with 0.5 m spacing and a slightly larger navigation
capsule than its movement capsule. Actual-position start attachment and greedy
waypoint shortcuts also sweep the capsule. Steering and bounded A* remain in
the game; ordinary `Grid` APIs and their cell-based smoothing are unchanged.
This is a flat, fixed-height navigation adapter, not multi-floor navigation,
climbing, dynamic avoidance or a navmesh generator. Player jumps can land on
props; zombies do not plan jumps or climbs. Characters do not collide with
one another and may overlap when converging on the player.

Fixed 60 Hz inputs and deterministic tie-breaking are preserved. State hashes
include prop descriptors and zombie feet. Identical seeds/input sequences
repeat on the same build/platform/features; hashes intentionally differ from
the old box simulation. Cross-platform or cross-version bitwise replay is not
promised. Nightwatch does not force Rapier's `enhanced-determinism` feature on
other applications through Cargo feature unification; callers can opt in and
test their chosen feature set.

## Remaining approximation limits

The Kenney barrel is faceted with an approximately rounded-square cross
section. Its containing **cylinder** encloses the vertices, so the largest
local flat-to-circle discrepancy is about 6–7 cm at the shipped scales. This
small conservative proxy is distinct from the removed large group rectangle;
strict facet-accurate hits would require an explicitly authored convex hull.
Crate bevels and tapered workbench legs retain cuboid approximations; loose
bench tools/paper and painted floor stripes are intentionally nonblocking.
No blanket mesh generation, skeletal collision, ragdolls or rigid-body response
has been added to the FPS.

## Validation and performance

See [collision validation results](collision-3d-validation.md) for measured
costs, test coverage, native exercises and reproduction commands.

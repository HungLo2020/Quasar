# Voxels

Quasar's voxel stack is four layered crates plus optional adapters. Each
layer works alone and depends only on the layers below it. The design and its
rationale are in [`voxel-spec.md`](voxel-spec.md); this page shows how to use it.

| Crate | Owns | Graphics? |
|---|---|---|
| `quasar-voxel` | chunks, channels, volumes, grids, scenes, block catalogs, edits, queries, propagation, connectivity, codecs, conformance harness | no |
| `quasar-voxel-mesh` | culled-model, greedy and Surface Nets extraction, GPU formats + WGSL decoders, translucency sorters, remesh queue | no |
| `quasar-voxel-view` | connectivity masks and visibility traversal, streaming planner, LOD selection, GPU range allocation, indirect draw lists | no |
| `quasar-render-voxel` | wgpu pipelines, paged geometry arenas, GPU culling, materials, shadows, overlays | yes |
| adapters | `quasar-physics` feature `voxel`, `quasar-nav` feature `voxel`, `quasar-assets` feature `voxel` | no |

Games with their own renderer (or their own GPU abstraction) stop at
`quasar-voxel-view`: the draw lists and range allocator use no wgpu types, and
`quasar_voxel_mesh::WGSL_FORMATS` decodes the packed formats in any WGSL
pipeline.

## Data

```rust
use quasar_voxel::*;
use quasar_math::IVec3;

let mut schema = Schema::new();
let block = schema.channel("block", StorageKind::Paletted, AIR);      // u16 IDs
let light = schema.derived_channel("light", StorageKind::Nibble, 0u8); // not part of edit deltas
let mut world = Volume::<Cube16>::new(schema);
world.set(block, IVec3::new(3, 4, 5), 1);
assert_eq!(world.get(block, IVec3::new(3, 4, 5)), 1);
```

- **Layouts.** `Cube<LOG2, Order>` fixes the chunk edge (8 to 64) and the
  index order (`Xzy`, `Xyz`, `Yzx`) at compile time. `Cube16` and `Cube32`
  are first-class.
- **Channels.** A channel can be uniform, dense, paletted, nibble or sparse.
  Uniform chunks cost no per-voxel memory. `channel_with_policy` takes a
  `PalettePolicy` (`FastPalette` or `ThresholdPalette` with `BitPacking`)
  when a game must match an external palette format bit for bit.
- **Sources.** `Volume::with_source` attaches a deterministic `VoxelSource`.
  `ensure_chunk` generates missing chunks, and with the `jobs` feature
  `ensure_chunks_parallel` does the same on the job pool. `edited_chunks` is
  the delta to persist or replicate. `evict_generated` drops chunks that can
  be regenerated.
- **Snapshots.** `Volume::snapshot(channel, coord, pad, &mut snap)` copies a
  chunk plus a border of neighbour voxels into a reusable flat buffer. Every
  extractor reads snapshots.

## Edits

```rust
let mut batch = EditBatch::new();
batch.fill(block, IVec3::new(0, 0, 0), IVec3::new(7, 0, 7), 1);
batch.brush(Brush::new(BrushShape::Sphere { center, radius: 4.0 }, BrushOp::Subtract, density));
let result = world.apply(&batch, EditOptions::default());
// result.dirty: chunks to remesh, including neighbours whose snapshots changed.
// result.changes: ordered (cell, old, new) events for gameplay, nav and physics.
```

Batches, change lists and chunks have deterministic binary encodings in
`quasar_voxel::codec`, which suits replication. Density is `i8` scaled by
`DENSITY_SCALE`, and positive values are solid.

## Blocks and grids

- `BlockCatalog` holds `BlockDef`s: per-face materials and shapes, a layer
  (opaque, cutout, translucent or custom), emission, multi-cell footprints
  and mount points. `place` and `remove` keep footprints consistent across
  the block, part and orientation channels (`BlockChannels`). `connected`
  checks mount-point connectivity.
- `Orientation` covers the 24 axis rotations.
- `Grid` is a volume with an `f64` `GridTransform` and a cell size.
  `Grid::split_off` moves one connected component into a new grid (cutting a
  ship in half). `split_check` or `components` find the components.
- `Scene` holds many grids. World-space raycasts and box queries are
  transformed into each grid. Rendering rebases grid transforms to the camera
  in `f64` (`GridTransform::camera_relative`).

## Queries and propagation

- `Volume::raycast` runs a DDA with a `TieBreak` policy and a hit callback
  for sub-cell shapes. `raycast_density` refines hits to the surface.
  `overlaps` and `sweep_aabb` do AABB collision against solid cells.
- `Propagator` floods light-like values through a `u8` channel. The game's
  `PropagationRules` decide cost and emission, reading the volume through a
  `Cursor` so repeated lookups stay in one chunk. A `QueueDiscipline` decides
  visit order. `add` seeds new light, and seeds equal to the current level
  re-spread into newly loaded neighbours. `update` recomputes the region
  after opacity or emission edits and matches a full rebuild. `sky_seeds`
  seeds sky light down columns.

## Meshing

| Extractor | Use | Policies |
|---|---|---|
| `CulledModelExtractor` | per-block model quads; the parity-capable path | `ModelSource`, `FacePolicy`, `ShadeSource`, `LayerRouter`, `IterationOrder` |
| `GreedyExtractor` | full-cube terrain, fewest quads | `GreedyMaterials`, `GreedyOptions` (AO, light) |
| `SurfaceNetsExtractor` | density volumes, material blending, LOD skirts | `SmoothOptions` |

Extractors write to a `MeshSink`. `MeshBuffers` collects quads and smooth
triangles per layer, and `CountingSink` only counts them. They then pack to
GPU formats: `PackedQuad` (12 B per axis quad), `ModelVertex` (20 B per
vertex) and `PackedSmoothVertex` (16 B per vertex). `PlaneSorter` and
`CentroidSorter` order translucent quads for a camera. With the `jobs`
feature, `RemeshQueue` meshes dirty chunks on the job pool and drops results
for stale revisions.

**Parity.** To reproduce a reference mesher, implement its rules as policies
for `CulledModelExtractor`. Then record both outputs with
`quasar_voxel::conformance::Recording` and `quasar_voxel_mesh::record_mesh`,
and diff them. `Recording::compare` reports the first differing byte.

## View

- `chunk_connectivity` computes a 15-bit `Connectivity` mask per chunk. A
  `VisibilityGraph` traverses the masks for cave-style occlusion culling with
  an injectable `TraversalPolicy`. The default `FrustumPolicy` combines a
  `ChunkFrustum` with a range check.
- `StreamingPlanner` turns viewers and budgets into ordered generate, mesh
  and evict work.
- `LodSelector` picks octree nodes for density volumes, with two-sided
  hysteresis and seam faces for skirts. `sample_lod` and
  `downsample_majority` build coarse data.
- `RangeAllocator` and `PagedArena` allocate GPU geometry ranges and delay
  frees until a submission completes. `DrawList` builds indirect commands in
  wgpu's layout.

## Rendering

`quasar_render_voxel::VoxelRenderer` draws on a caller-owned device inside
the caller's encoder:

1. `set_grid` sets each grid's transform.
2. `upload` sends meshed chunks, and `remove` drops them. Freed ranges are
   reused once the GPU finishes the frames that used them. In a live loop,
   call `submitted(gpu, prepared.submission)` after submitting and
   `retire_completed(gpu)` each frame. Headless tools use `wait_and_retire`.
3. `prepare` runs once per frame, with the camera, shading and options.
   Frustum culling runs on the GPU into multi-draw-indirect where the adapter
   supports `INDIRECT_FIRST_INSTANCE`, and falls back to the CPU otherwise.
4. The pass draws opaque, cutout, translucent (sorted, back to front) and
   overlays in one command buffer. `set_visible_chunks` restricts a grid to a
   visibility traversal's result. Record lists and GPU cull input are cached
   until geometry or filters change, so a static scene costs one plane upload
   and one dispatch per frame.
5. `prepare_depth` renders voxel shadow casters into a `quasar-render-3d`
   `ShadowTarget` layer.

Lighting reuses `quasar_render_3d::LIGHTING_WGSL`, so voxels and meshes are
lit identically. `OverlayBox` draws selection outlines and ghost previews.

## Adapters

- **Physics** (`quasar-physics`, feature `voxel`): `VoxelColliders` builds
  merged-box compounds for cubic grids and trimeshes for density volumes, as
  static colliders or as dynamic rigid bodies per grid. `apply_dirty`
  rebuilds only the edited chunks.
- **Navigation** (`quasar-nav`, feature `voxel`): `walkable_grid` and
  `occupancy_grid` extract pathfinding grids. `sync_changes` updates them
  from edit events.
- **Assets** (`quasar-assets`, feature `voxel`): `CatalogLoader` (JSON block
  catalogs) and `ChunkLoader` (codec chunk files).

## Examples and benchmarks

- `examples/voxel-blocks`: a block world with MattMC-like defaults.
- `examples/voxel-drift`: a Space Engineers-style asteroid and ship.
- `cargo run --release -p quasar-voxel --example voxel_benchmark -- --check`
- `cargo run --release -p quasar-voxel-mesh --example mesh_benchmark -- --check`
- `cargo run --release -p quasar-render-voxel --example render_benchmark -- --check`
  (needs a GPU)

The benchmarks print JSON. `--check` fails when a target from
`voxel-spec.md` §11 is missed, after retries and normalisation for machine
load (`quasar_profile::micro`; see §11). The examples print settle, tick and prepare
timings and can write them as JSON (`--benchmark-file`).

## Partial-height blocks

`BlockDef::with_height(h)`, `with_bounds(min, max)` and `BlockDef::partial`
make a single-cell block occupy a box inside its cell, such as slabs,
cut-away walls a top-down camera sees over, and pedestals. Face shapes are
derived from the box:

- A face covering the whole cell side is `Full`.
- A partial face on the cell boundary gets a shape id shared by equal
  rectangles, so two equal walls cull each other, including when rotated.
- An inset face never hides anything and is never hidden.

The default `CatalogModels` meshes the box (rotated with the block), and
`BlockDef::bounds` gives collision code the height. voxel-blocks' slab is a
`with_height(0.5)` block.

## Voxels with meshes

See [3D effects](effects-3d.md#voxels-and-meshes-on-one-scene) for
`HybridFrame`, which orders voxel and render-3d passes on one `SceneTarget`
and reports voxel submissions.

## Bevelled blocks

`BevelExtractor` (in `quasar-voxel-mesh`) meshes full-cube blocks with
chamfered edges and chipped corners that follow the neighbourhood: only
edges and corners whose adjoining faces are all open are cut, each lattice
point's depth comes from a seeded hash and the largest bevel scale of the
blocks around it (so neighbours' cuts meet), and corners stay sharp where a
cut would expose a neighbour's face. The result is closed and outward
facing (tests check every edge is shared). A per-block policy returns
`None` to leave a block to another extractor, `Some(0.0)` for a flat cube
or `Some(scale)` for a bevel; faces take block and sky light from the cell
in front. Voxel Blocks' `--sculpted` style bevels stone and ore this way.

## Dense read views

`Volume::dense_view(channel, min, max)` copies one channel of a box into a
flat array: reads are O(1) with no chunk hashing or palette unpacking, which
suits small fixed maps queried thousands of times a tick (collision, line
of sight, nearest-ore scans). `DenseView::refresh` re-copies only chunks
whose revision changed, and `set` mirrors a write the game also made.

## Emissive masks

`MaterialDef::with_emissive_mask(colour, layer)` makes a texture-array
layer scale a material's emission per texel (with triplanar mapping on
smooth surfaces), so ore flecks or lava cracks glow while the rest of the
block does not. Voxel Blocks' crystal ore uses it.

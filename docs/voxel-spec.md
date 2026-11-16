# Voxel machinery specification

Status: **implemented.** This document specifies Quasar's reusable voxel
stack and remains the reference for its design. The usage guide is
[`voxel.md`](voxel.md). Scope decisions are listed at the end (§14).

## 1. Goals and targets

Quasar's voxel stack must serve three target game styles without forking:

| Target | Shape of the problem |
|---|---|
| **MattMC** (priority) | Minecraft-style block world: 16³ sections in tall columns, a block-state palette, per-block baked model quads, ambient occlusion, block + sky light, translucent layers, section visibility culling, distant LOD. Must be able to reach **exact parity with its Frozen Java reference** using Quasar machinery plus MattMC-supplied policies. |
| **Planetfall** | Top-down survival: chunked multi-layer planets with mining, a free-form buildable ship, picking and ghost placement previews, hordes that path over voxel terrain. |
| **Space Engineers-style** | Two kinds of volume in one scene: smooth, deformable **voxel maps** (density + material, huge and mostly procedural, LOD with seams) and **block grids** (oriented cubic blocks, multi-cell blocks, slopes, many grids moving as rigid bodies, splitting when cut). |

Non-negotiables:

1. **Performance is critical.** Hot paths are monomorphised, allocation-free
   in the steady state, data-oriented and measured by benchmarks from the
   first commit (§11).
2. **Modular.** Every layer is usable alone; dependencies point downwards;
   no layer requires a runner, ECS, editor or the built-in renderer.
3. **Mechanism in Quasar, policy injectable.** Anything whose observable
   result a game may need to reproduce exactly (orders, rules, packings) is
   either documented-deterministic or supplied by the caller (§3).
4. **SE-style volumes are first-class from the start**, not an extension:
   channels, grids with transforms, oriented multi-cell blocks, procedural
   sources with edit deltas and smooth extraction are part of the core model.
5. **Deterministic per platform.** The same inputs and policies produce the
   same outputs (bytes, orders, counts) on one platform and build profile.
   Cross-platform bit identity is not required.
6. Quasar does not depend on, copy from or modify the target games. They
   adopt Quasar later through its public APIs.

## 2. Key decisions

### 2.1 Chunk sizes

- Chunks are **cubes with a power-of-two edge**, chosen per volume at compile
  time: `Chunk<const LOG2: u32>`, with `LOG2` in **3..=6** (8, 16, 32, 64).
  Hot paths (indexing, snapshot copy, occupancy masks, extraction) are
  monomorphised per size; there is no runtime branching on size inside loops.
- **First-class sizes are 16 and 32.** Every extractor, test and benchmark runs
  at both. 8 and 64 are supported for storage and queries; extractors may
  reject 64 (the binary mesher's padded 66-cell column exceeds 64 bits).
- Defaults per target:

| Use | Edge | Why |
|---|---|---|
| MattMC-style worlds | 16 | Matches section granularity, so dirty/remesh/visibility units line up with the parity-defined unit. |
| Planetfall terrain and ships | 32 | Fewer chunks and draws; 34-cell padded columns fit `u64` masks. |
| SE voxel maps | 32 cells per LOD node | Fixed cell count per node at every LOD (node voxel size doubles per level). |
| SE block grids | 16 | Grids are often small; 16 keeps sparse grids cheap and splitting local. |

- Tall worlds are columns of cubic chunks (vertical extent is a world bound,
  not a chunk shape), so non-cubic chunks are never needed.

### 2.2 Coordinates

- Voxel coordinates are **grid-local `i32`** per axis (`IVec3`); chunk
  coordinates are voxel coordinates shifted by `LOG2`. Each grid (§4.2) has its
  own origin, so ±2³¹ voxels per grid is enough for planets and worlds.
- Grid transforms are **`f64`** (position `DVec3` + rotation `DQuat`) so scenes
  can span solar-system distances. Rendering is **camera-relative**: grid
  matrices are rebased to the camera in `f64` and converted to `f32` per frame.
- Conventions follow `quasar-math`: right-handed, Y up, `[0, 1]` depth.

### 2.3 Rendering home

A separate **`quasar-render-voxel`** crate owns wgpu pipelines and buffers.
It builds on public pieces of `quasar-render-3d` (lights, shadow bindings,
scene target, transparent conventions, post chain) rather than living inside
it, keeping `quasar-render-3d` free of voxel code and letting games with their
own renderers (MattMC) skip it entirely. A small prerequisite change exposes
render-3d's lighting/shadow WGSL functions and binding layouts as reusable
modules (§8.4).

## 3. Parity model ("mechanism vs policy")

MattMC needs Quasar to reproduce vanilla's **observable outputs**, not to run
vanilla's algorithms. Every subsystem therefore separates:

- **Mechanism** (Quasar): data structures, traversal, scheduling, memory,
  parallelism and GPU residency.
- **Policy** (trait, default supplied by Quasar, replaceable by the game):
  every rule or order that decides an output.

| Subsystem | Parity-sensitive outputs | Policy trait(s) |
|---|---|---|
| Paletted storage | palette kinds, resize thresholds, first-use growth order, bits per entry, word packing (spanning or not), serialised layout | `PalettePolicy`, `BitPacking` |
| Face culling | whether each face is emitted | `FacePolicy` |
| Model extraction | quads per block, cull faces, emission order, layer routing | `ModelSource`, `LayerRouter`, `IterationOrder` |
| Shading in meshes | per-vertex AO and light | `ShadeSource` |
| Translucency | quad order for a camera | `TranslucentSorter` |
| Visibility | which chunks are visited and in what order | `ConnectivityPolicy`, `TraversalPolicy` |
| Light-like propagation | resulting levels; observable update order | `PropagationRules`, `QueueDiscipline` |
| Raycast | which cell/face is hit, tie-breaking, sub-cell shapes | `CellHitTest`, `TieBreak` |

Rules that apply everywhere:

- No output may depend on `HashMap`/`HashSet` iteration order or thread timing.
  Parallel stages merge results in a defined order.
- Every policy-driven stage is **replayable**: the conformance harness (§10)
  records inputs/outputs so a game can diff against its own reference data.
- Quasar's defaults may be faster or different (for example greedy meshing).
  Exact-parity configurations select the policy-driven paths.

## 4. Core data model (`quasar-voxel`)

Dependency-free except `quasar-math` (optional `serde`, `jobs` features).

### 4.1 Channels

A volume stores **channels**: independent per-voxel attributes, each with its
own storage strategy. Struct-of-arrays keeps hot loops (occupancy, density)
dense and lets games pay only for what they use.

| Channel examples | Typical storage |
|---|---|
| block/material state (`u16`/`u32` IDs) | paletted |
| density (`i8` signed distance, quantised) | dense or uniform |
| block light, sky light (4-bit) | nibble-packed dense, uniform when constant |
| orientation (5 bits, 24 rotations) | paletted or dense `u8` |
| sparse metadata / block entities | sparse map keyed by local index |

A volume's channels are declared once in a runtime `Schema` that hands out
typed `ChannelId<T>` handles. Accessors are monomorphised on the value type
and chunk layout. Channel lookup is an index into a small array, so there is
no per-access dispatch on storage kind inside extractor loops: extractors read
decoded snapshots. A runtime schema keeps games free to compose channels
(fluids, metadata, game-specific state) without new core types. Derived
channels (light) are excluded from edit deltas.

### 4.2 Storage strategies (per channel, per chunk)

`trait ChannelStorage<T>` with implementations:

- `Uniform<T>`: one value for the whole chunk (zero per-voxel memory).
- `Dense<T>`: flat array in the chunk's index order.
- `Paletted<T, P: PalettePolicy, B: BitPacking>`: palette plus packed indices.
  Policies decide palette kinds (single, linear, hashed, global), thresholds,
  growth order and packing (`u64` words, spanning or non-spanning). Quasar's
  default policy optimises speed; a vanilla-capable configuration must be
  expressible with no Quasar changes.
- `Nibble`: 4-bit dense values.
- `Sparse<T>`: sorted local index → value for rare data.

Storage may **promote/demote** automatically (uniform ↔ paletted ↔ dense)
under the policy's rules; promotion order is part of the policy.

Index order within a chunk is a const policy (`IndexOrder`: `XZY`, `YZX`, ...)
so a game can match an external layout for zero-copy import/export.

### 4.3 Chunks, volumes and grids

- `Chunk<C, LOG2>`: one chunk of channel set `C`, plus a **revision** counter
  and occupancy summary (all-empty, all-solid, mixed) for fast skipping.
- `Volume<C, LOG2>`: chunk map keyed by `IVec3` chunk coordinate, using an
  ordered, cache-friendly map (sorted pages of chunks per region) so
  iteration is deterministic. Absent chunks read from the volume's
  **source** (§4.5).
- `Grid`: a volume plus `GridTransform` (`f64`), `cell_size` (metres per voxel:
  MattMC 1, Planetfall 2, SE large 2.5, SE small 0.5), `GridKind` (static or
  dynamic) and a stable `GridId`.
- `Scene`: many grids indexed by world bounds through `quasar-spatial`.
  World-space queries (rays, boxes, spheres) find candidate grids, transform
  into grid space and run grid-local queries. MattMC is one static grid at
  the identity transform.

### 4.4 Block catalog (block grids and block worlds)

`BlockCatalog` holds definitions referenced by a state channel:

- **Footprint:** set of occupied cells relative to an anchor (multi-cell
  blocks). Non-anchor cells store a reference to the anchor.
- **Orientations:** the 24 axis-aligned rotations, applied to footprint,
  faces and models.
- **Per-face shapes:** full, none or a shape ID compared by `FacePolicy`
  (slopes, half blocks, panels cull correctly).
- **Mount points:** faces where neighbours may attach (placement validity and
  structural connectivity).
- **Render data:** model ID, material IDs and layer for the extractors.

Placement and removal go through the catalog so footprints stay consistent.

### 4.5 Sources and edit deltas

Large worlds (planets, asteroids, infinite terrain) are mostly procedural.
`trait VoxelSource` fills a chunk's channels for a coordinate (and LOD level).
A volume is **source + delta**: generated chunks are cached under a budget and
may be dropped; edited chunks persist as deltas (full chunk or sparse diff).
Generators run on `quasar-jobs` and must be deterministic for a seed (using
`quasar_core::Rng`).

### 4.6 Snapshots

Extractors and propagation read a **padded snapshot**: the chunk plus a
border of width `B` from its neighbours (`B` = 1 for culled/greedy meshing,
1–2 for smooth extraction and AO). Snapshots are copied into reusable,
cache-aligned scratch buffers, with fast paths for uniform neighbours and
whole-row copies. Snapshot extraction order is deterministic.

### 4.7 Edits, revisions and dirtiness

- `EditBatch`: set cells, place/remove catalog blocks, apply density brushes
  (sphere/box/capsule, add/subtract/smooth/paint), fill regions.
- Applying a batch bumps chunk revisions, records **dirty chunks plus the
  neighbours whose padded snapshots touch the edited cells**, and emits
  ordered change events (cell, old, new) for gameplay, nav and physics.
- Work started for an old revision is cancelled or discarded automatically.

### 4.8 Queries

- **Raycast:** DDA (Amanatides–Woo) through chunks then cells, skipping
  uniform-empty chunks, with a `CellHitTest` callback for sub-cell shapes and
  a `TieBreak` policy for edge/corner cases. Returns cell, face, distance and
  hit point. For density channels, hits refine to the zero crossing.
- **Overlap and sweep:** AABB/sphere overlap and swept-AABB queries against
  solid cells or catalog shapes.
- **Region iteration:** ordered visits of cells or chunks in a box or sphere.

### 4.9 Propagation engine

A generic flood-fill engine for light-like values (block light, sky light,
fluids, heat, oxygen): bucketed level queues, increase and decrease phases,
cross-chunk propagation through snapshots, `PropagationRules` (attenuation per
cell and direction, sources, sky seeding) and `QueueDiscipline` (bucket and
within-bucket order). Results are deterministic for a given discipline.

### 4.10 Connectivity and splitting

Connected components over solid/catalog cells respecting mount points, run
incrementally from removal sites. When a dynamic grid's cells split into
several components, the edit result reports the split so the game can create
new grids (ships cut in half). Splitting itself is a core operation that
moves chunk data to a new `Grid` with the same transform.

## 5. Extraction (`quasar-voxel-mesh`)

Renderer-neutral CPU meshing. Depends on `quasar-voxel`.

### 5.1 Shared shape

```text
trait Extractor<C, const LOG2: u32> {
    type Config;                       // policies and tuning
    fn extract(&mut self, snapshot: &Snapshot<C, LOG2>, config: &Self::Config,
               sink: &mut dyn MeshSink) -> ExtractStats;
}
```

`MeshSink` receives quads or triangles tagged with a **layer** (opaque,
cutout, translucent, game-defined) in deterministic order. Quasar ships sinks
that write its GPU formats (§7.2); games may write their own vertex formats.
Extractors own reusable scratch memory and never allocate per call once
warmed up.

### 5.2 Extractors

| Extractor | For | Notes |
|---|---|---|
| `CulledModelExtractor` | MattMC-style blocks, any catalog block | Per-cell model quads from `ModelSource`; faces culled by `FacePolicy`; shading from `ShadeSource`; order from `IterationOrder`; layer from `LayerRouter`. The parity-capable path. |
| `GreedyExtractor` | full-cube materials (terrain, armour blocks) | Binary greedy meshing over bitmask columns (`u32` for 16, `u64` for 32), merging coplanar faces with equal material/shade keys. Fastest path; output is not per-block. |
| `SurfaceNetsExtractor` | density channels | One vertex per surface cell, material blending, normals from density gradient. Fast and robust. |
| `SeamSkirts` | LOD seams between density nodes | Skirt geometry along a node's surface boundary on faces that border a different LOD, extruded into the solid side so cracks between levels are never visible. Needs no lookup tables and works for any extractor. |

Extractors can be combined per chunk (for example greedy for full cubes plus
culled-model for non-cube blocks in the same chunk), writing to one sink.

### 5.3 Shading defaults

Quasar's default `ShadeSource` provides 4-level vertex AO with the
anisotropy-correct quad flip, and optional light channels sampled per vertex.
A vanilla-exact smooth-light implementation is a game policy, not a Quasar
default.

### 5.4 Translucency

Translucent layers keep per-quad centroids and normals. `TranslucentSorter`
policies: none, centroid distance (default), and a topology-aware sorter that
only re-sorts when the camera crosses a quad plane group. Sorting runs off the
render thread and produces an index permutation.

## 6. Visibility, streaming and LOD (`quasar-voxel-view`)

Renderer-neutral planning: what should exist, be meshed and be drawn.

- **Connectivity graph:** per chunk, a 15-bit face-to-face connectivity mask
  from a flood fill of non-opaque cells (`ConnectivityPolicy` decides opacity).
  `TraversalPolicy` controls queue order, direction order and frustum test, so
  a game can reproduce a reference culler's visit set and order exactly.
- **Frustum and distance culling** with `quasar-spatial` frusta, camera-relative.
- **Streaming planner:** desired chunk set from viewers (position, radius,
  vertical limits, per grid), priorities by distance and view direction, and
  budgets for generation, meshing, upload and eviction per frame. Produces
  ordered work for `quasar-jobs`. Games with their own loading rules (tickets)
  can drive the planner's inputs directly.
- **LOD:** a clipmap/octree selector for density maps (fixed 32³ cells per
  node, voxel size doubling per level, hysteresis to avoid popping) and a
  downsample pyramid for cubic volumes (majority material for coarse cells).
  LOD data comes from a `LodSource` so games can supply their own far-terrain
  representation.
- **Residency bookkeeping (portable GPU layer):** a range allocator for large
  vertex/index pages (best-fit over size-ordered free ranges with coalescing,
  O(log n) per operation, spread over fixed-size pages), frees that
  are released only after a caller-supplied submission completes, per-chunk
  draw records and **indirect draw command arrays** in wgpu's layout. No wgpu
  types: callers with their own GPU abstraction (MattMC's GAL handles) perform
  the copies and draws themselves.

## 7. GPU data formats

### 7.1 Principles

Vertex pulling from storage buffers (as `quasar-render-3d` already does), one
shared quad index buffer, compact fixed-point formats decoded in shaders, and
chunk-relative positions so `f32` precision is never an issue.

### 7.2 Formats

| Format | Size | Use |
|---|---|---|
| `PackedQuad` | 12 bytes per quad | Axis-aligned quads (greedy and full-cube faces): position in chunk, face, width/height, material layer, 4×2-bit AO, light. Expanded to 4 vertices in the vertex shader. |
| `ModelVertex` | 20 bytes per vertex | Arbitrary model quads: fixed-point position relative to the chunk (with margin for overhang), octahedral normal, UV, tint, AO/light, material. |
| `SmoothVertex` | 16 bytes per vertex | Density meshes: fixed-point position, octahedral normal, two blended materials and weight. |

Exact bit layouts are fixed during the spike (§12, phase 1) and then frozen.

## 8. Rendering (`quasar-render-voxel`)

wgpu implementation on a caller-owned device and frame. Optional crate.

### 8.1 Responsibilities

- GPU pages driven by the portable residency layer; uploads through staging
  buffers in the caller's encoder; completion-tracked frees.
- Pipelines for the three formats, opaque/cutout/translucent layers, shadow
  casters and picking/outline overlays.
- **Multi-draw indirect** where the adapter supports it, with a per-draw loop
  fallback. **GPU culling** (compute frustum + optional hierarchical-Z
  occlusion) writes indirect commands when indirect-count is available; CPU
  culling otherwise. Feature detection uses capabilities, never backend names.
- Materials through texture arrays (cubic) and triplanar mapping (smooth),
  sampled in linear space.

### 8.2 Composition with existing renderers

- Draws into a `SceneTarget` (HDR) or any caller target with depth, composes
  with `quasar-render-3d` passes in the same frame, and uses render-3d's light
  lists and shadow maps (voxel chunks are shadow casters and receivers).
- Ghost previews and glass go through the sorted transparent path.
- The post chain encodes once, as for other 3D work.

### 8.3 External renderers

Games with their own renderer use layers 1–3 plus the portable residency
layer and their own pipelines. WGSL decoding functions for the formats are
published as source modules so external renderers can share them.

### 8.4 Prerequisite in `quasar-render-3d`

Expose lighting/shadow WGSL functions and their binding layouts as reusable
modules (no behaviour change), so voxel pipelines light identically to
render-3d meshes without duplicating shader code.

## 9. Adapters (optional features in existing crates)

- `quasar-physics` feature `voxel`: colliders per grid (compound boxes for
  cubic cells and catalog shapes, triangle meshes for density), incremental
  updates from change events, dynamic grids as Rapier rigid bodies.
- `quasar-nav` feature `voxel`: `Grid<3>` or walkable-surface extraction from
  occupancy, updated through `DistanceField::update_region` on edits.
- `quasar-assets`: optional loader for catalog definitions and chunk saves.

## 10. Conformance harness

- Record/replay files capture a stage's inputs (snapshot, policies, camera)
  and outputs (quads/indices with order, visit lists, propagated levels).
- Quasar's own tests use synthetic goldens that prove each policy hook can
  force an exact, specified result (for example "emit faces in this custom
  order", "visit chunks in this queue order", "pack palette indices without
  spanning words").
- Games point the same harness at their own reference data from their repos.

## 11. Performance

Targets apply to a release build on the development machine, single thread,
with full `Quad` output through a `MeshSink`. The initial guesses (greedy
100 µs, culled 150 µs, Surface Nets 200 µs) were refined by measurement into
per-scenario targets. Uniform chunks exit after one scan (about 2 µs).
Throughput scales with cores through `RemeshQueue`.

| Measure | Target | Measured |
|---|---|---|
| Greedy extraction, 32³ rolling terrain with AO (typical) | < 150 µs per chunk | 125 µs |
| Greedy extraction, 32³ rough terrain with glass and caves (worst case) | < 500 µs per chunk | 395 µs |
| Culled-model extraction, 16³ rough mixed blocks with AO | < 300 µs per chunk | 232 µs |
| Surface Nets, 32³ node, 7.5k triangles | < 450 µs per node | 353 µs |
| Snapshot, 16³ with 1-voxel border | < 50 µs | 12 µs |
| Edit to visible remesh (edited chunk + neighbours) | next frame | `voxel-blocks`: relight and remesh requests in the edit tick, uploaded the next frame |
| Uniform chunk overhead | ≤ 32 B header + 32 B per channel (64 B for one channel) | 32 + 32 per channel |
| CPU draw submission, 32-chunk radius cubic world | < 1 ms with indirect draws | 0.6–0.8 ms for 21k chunks (`render_benchmark`, median); 0.83 ms in `voxel-blocks` at radius 32 with connectivity culling |
| Steady-state allocations in extract/upload/draw | zero | Extraction and upload: none after warm-up. Draw preparation: a fixed handful of small per-frame buffers (uniform staging, layer lists), none per chunk or record |

Benchmarks ship as headless examples with JSON output (like existing Quasar
benchmarks) and guard regressions in `tools/full_checks.py`.

## 12. Phases

LOD seams use skirts (`SeamSkirts`) rather than Transvoxel transition
cells: skirts are extractor-independent, cheap, need no third-party tables and
reliably hide cracks at the cost of a little hidden geometry.

Each phase ends with tests, synthetic parity goldens for any new policy, and
benchmark results against §11.

1. **Spike:** channel and storage traits, `Chunk<LOG2>` indexing at 16/32,
   palette and packing policies, snapshot copy, greedy vs culled-model
   prototypes, vertex format bit layouts, benchmark and harness skeletons.
2. **Core:** volumes, grids and scene, catalog with orientations and
   footprints, sources and deltas, edits and change events, raycast/overlap
   queries, connectivity and splitting.
3. **Extraction:** culled-model, greedy, Surface Nets, seam skirts,
   translucency sorters, jobs integration.
4. **View layer:** connectivity graph, streaming planner, LOD selection,
   residency allocator and draw records, propagation engine.
5. **Rendering:** `quasar-render-voxel` plus the render-3d WGSL prerequisite.
6. **Adapters:** physics colliders and dynamic grids, nav extraction.
7. **Examples completed** (§13) and documentation.

## 13. Example games (first consumers)

Both are original, deliberately simple games with procedural placeholder
art (no external textures or assets). Each has a native window mode and a
headless mode with deterministic replay, screenshots and a benchmark, like
the existing Quasar examples.

### 13.1 `examples/voxel-blocks` (MattMC-style defaults)

- Infinite procedural terrain in 16³ chunks, columns with a fixed height.
- A handful of original block types: solid, grass-like top/side, glass
  (translucent), leaves (cutout), a light-emitting block and a half block.
- Culled-model extraction with AO, block + sky light via the propagation
  engine, translucent sorting, connectivity-graph culling, streaming by view
  distance.
- First-person movement, a crosshair raycast, **place and remove** blocks,
  a simple hotbar.

### 13.2 `examples/voxel-drift` (Space Engineers-style)

- A space scene with a smooth procedural asteroid (density + material,
  Surface Nets, LOD with seam stitching) and a small ship built as a dynamic
  block grid (2.5 m cells, oriented blocks, a multi-cell block, slopes).
- **Remove and add** on the asteroid with spherical brushes; **place and
  remove** oriented blocks on the ship.
- Removing the blocks that connect two halves of the ship splits it into two
  grids that drift apart.
- Free-flight camera; physics colliders for the asteroid and ship grids.

## 14. Resolved scope

- **Examples:** `voxel-blocks` and `voxel-drift`; keyboard and mouse, plus
  controllers through `quasar-input` where it is free to add.
- **Physics:** the `quasar-physics` voxel adapter (colliders per grid,
  incremental updates, optional rigid bodies for dynamic grids) is in scope.
  The examples need collision and simple drift, not a full simulation.
- **Persistence:** the examples keep edits in memory. A chunk delta codec is
  still provided because multiplayer games replicate edits.
- **Multiplayer:** the target games are multiplayer, so edit batches, change
  events and chunk deltas are deterministic, ordered and serialisable for
  replication. There is no networking code or multiplayer example.
- **GPU baseline:** compute shaders, storage buffers and threading are
  required; multi-draw-indirect and indirect-count are used when available.
- **Fluids (designed for, not built):** a fluid is a channel (`kind`,
  `level`, flags) stored like any other; flow is the propagation engine
  driven by scheduled updates with `PropagationRules` for spread and decay;
  extractors already route per-layer output, and a future fluid extractor
  receives per-corner heights through the same `MeshSink`. Nothing in the
  core assumes cells are either solid or empty.

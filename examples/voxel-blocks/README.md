# Voxel Blocks

A small block world built on Quasar's voxel stack with MattMC-style defaults:
16³ chunks in 80-block columns, per-block models through the culled-model
extractor, AO, block and sky light, and cutout and translucent layers. Block
textures are procedural and the game is original. Edits stay in memory.

```sh
cargo run --release -p voxel-blocks
cargo run --release -p voxel-blocks -- --headless --autoplay --ticks 600 --screenshot /tmp/blocks.png
cargo run --release -p voxel-blocks -- --headless --ticks 1 --radius 32 --screenshot /tmp/far.png --benchmark-file /tmp/blocks.json
cargo run --release -p voxel-blocks -- --benchmark --benchmark-frames 240 --benchmark-file /tmp/blocks-frames.json
```

## Controls

| Input | Action |
|---|---|
| `W/A/S/D`, mouse | Walk, look |
| `Space` | Jump (fly up while flying) |
| `Left Shift` | Fly down |
| `F` | Toggle flying |
| Left mouse | Break the outlined block |
| Right mouse | Place the selected block |
| `1`-`9`, wheel | Select a hotbar block |
| `F11` / `Escape` | Fullscreen / quit |

Blocks: stone, dirt, grass, planks, a half slab (custom model with shaped
faces), glass (translucent, sorted), leaves (cutout), a lamp (light level
14) and logs.

## What it uses

- `quasar-voxel`: a five-channel `Schema` (paletted blocks and orientation,
  sparse multi-cell parts, nibble block and sky light). It also provides a
  deterministic `VoxelSource`, `BlockCatalog` placement and removal through
  `EditBatch`, DDA raycasts, and `Propagator` light with game
  `PropagationRules`. Sky light falls without loss, leaves dim it, and lamps
  emit. Edits relight incrementally with `Propagator::update`.
- `quasar-voxel-view`: a `StreamingPlanner` loads whole columns around the
  player (4 per tick, ahead of the view first). Per-chunk `Connectivity`
  masks feed a `VisibilityGraph` traversal, which skips chunks hidden behind
  terrain ("cave culling") through `VoxelRenderer::set_visible_chunks`.
- `quasar-voxel-mesh`: `CulledModelExtractor` jobs on a `quasar-jobs` pool,
  tracked by a `RemeshQueue` so stale results are dropped.
- `quasar-render-voxel`: packed quads and model vertices in paged arenas, GPU
  culling with multi-draw indirect, fog, an outline overlay and a 2D HUD.

Newly loaded columns seed sky light only on the faces of their all-air
chunks, which are filled uniformly, and reseed from loaded neighbours. A test
checks the result against a full recompute.

## Headless and benchmarks

`--headless` runs fixed ticks without a window. Meshing waits for its jobs
each tick, so results are deterministic regardless of thread count. The run
prints counts, a world hash, the settle time (generate, light and mesh the
view radius) and tick timings. `--screenshot` renders offscreen and reports
the voxel renderer's CPU prepare time. `--benchmark-file` writes the same
numbers as JSON. `--no-occlusion` disables connectivity culling for
comparison.

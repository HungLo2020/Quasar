# Tilemap and indexed 2D implementation report

All four improvements are implemented. The architecture, ownership contracts,
read-only game audit and reproduction commands are in
[retro-rendering.md](retro-rendering.md).

## Delivered changes

| Improvement | Implementation |
| --- | --- |
| Preloaded tilesets | `LoadedTilesets::from_textures` validates/reuses existing identities and deduplicates normalized image paths. CPU assets share `Image`'s Arc pixels; caller-owned GAL views use the existing renderer registration path. Optional `PngLoader` integrates mounted/cached images. The convenience PNG loader and preloaded constructor share atlas validation. |
| Indexed palettes | `IndexedImage`, `Palette`, `PaletteBinding`, checked color cycling, existing `AnimationClip` clocks and dynamic palette textures. Shader/reference lookup covers all 256 indices, explicit transparent indices, bank swapping and mixed RGBA drawing. |
| Pixel masks and priority | Configurable eight-bit compare/replace rules, alpha/index coverage, mask-only draws and priority helpers. GAL stencil and CPU byte-plane implementations distinguish color, background occupancy and sprite occupancy. Hidden high-priority sprites block later sprites through covered pixels. |
| Live edits | Validated, transactional `Runtime::set_tiles`/`set_tile`, ordered incremental row/span insertion, unchanged cell buffers, bounded mutation plans, counters and `into_map` export. `Map::set_tiles` removes whole-map cloning while validating the public document once per batch. |

Tilequest uses asset-service preloading, and its new `retro` example demonstrates
batched live edits, palette banks, background masks and sprite priority with
optional GPU/reference parity. The composition example tests real preloaded GAL
atlases and masked overlays over a loaded 3D world. The full-check tool has a
reproducible `retro-2d` section.

## Validation results

Validation was performed locally on Linux, Intel Core i7-10750H and NVIDIA
GeForce RTX 2070 (driver 580.178.04). No external game repository was modified.

- Workspace tests passed: **603 passed**, 52 opt-in tests ignored by the ordinary
  workspace command. Relevant opt-in GPU tests were then executed explicitly.
- Workspace Cargo check with **all targets and all features** passed.
- Changed packages/examples passed all-target/all-feature Clippy with warnings
  denied. Rustdoc with warnings denied, formatting and whitespace checks passed.
- The focused section passed **24 renderer tests, 10 composition tests and 6
  Tilequest tests on each of Vulkan and wgpu**. These counts include each suite's
  CPU tests as well as its native GPU tests; GPU checks were not skipped.
- Three Quasar UI shared-GAL/text GPU tests and four Pong tests (including its GPU
  scene regression) passed.
- Headless tests passed with renderer defaults disabled, with optional assets,
  tilemap defaults disabled, tilemap `render-2d` without PNG/GPU, tilemap PNG, and
  Tilequest desktop disabled. Renderer configurations with GPU-only, Vulkan,
  wgpu and OpenGL features compiled.
- Tests cover all index values, bank cycling/animation, tint/alpha, sprite
  transforms, clip masks, transparent background/sprite indices, hidden-sprite
  claims, caller color/depth target composition, pending generations, reverse
  submission and resource retirement. Opaque pixel checks are exact; blended
  CPU/GPU channels differ by at most one byte.
- Native Tilequest presented **40 frames**, completed all four window exercises,
  and retired **101/101 resources**. Its final ordinary draw list had **748 quads,
  five batches, 59,840 geometry-upload bytes and zero texture-upload bytes**.
- The existing tilemap verification script passed PNG previews, conversion,
  bundling, bad-input checks, relocated packaging and a deterministic
  **6,000-tick winning replay**. Both CPU-only and GPU retro examples passed.

The OpenGL runtime attempt reached the renderer's pre-existing explicit-binding
GLSL restriction. It remains a compiled configuration with unsupported runtime
initialization, rather than a tested rendering backend for these APIs.

## Performance

[Recorded measurements](../examples/tilequest/benchmarks/retro-2d-cpu.json) include
workload definitions and raw timings. These are local measurements with normal
clock/load variation, not guarantees for another machine.

A single-cell edit in an existing chunk now performs no cell-buffer clone,
whole-map validation, animation rebuild or index rebuild. Release-mode means:

| Stored cells | Indexed live edit | Legacy validate/clone/rebuild path |
| ---: | ---: | ---: |
| 10,000 | 0.184 µs | 250.188 µs |
| 1,000,000 | 0.217 µs | 23,530.550 µs |
| 4,194,304 | 0.114 µs | 89,805.925 µs |

The benchmark averages 1,000 live edits and ten legacy edits per case, includes
validation and mutation work, and prevents the edited map from being optimized
away. It compares with the old validate/clone/validate/runtime-build algorithm;
the comparison transfers the resulting document without adding an extra clone.
New sparse cells require an incremental row/span insertion; existing edits
report **zero inserted/indexed spans**. Tests also verify stable buffer addresses.

A separate 3,000-iteration culling comparison used an isolated source archive of
baseline commit `292dd79`, then the working tree, at a fixed 480x320 viewport:

| Stored cells | Before p50 / p95 | After p50 / p95 | Maximum visited cells |
| ---: | ---: | ---: | ---: |
| 10,000 | 37.348 / 61.936 µs | 32.768 / 40.236 µs | 620 |
| 1,000,000 | 29.808 / 36.665 µs | 31.862 / 37.583 µs | 620 |

Culling remains bounded by the viewport. The million-cell case showed a 6.9%
median increase and 2.5% p95 increase in this run; the smaller case improved.
The measurements do not establish a culling speedup, and richer draw metadata
and ordered row indexes have some overhead. Load/index times for the million-cell
comparison were 74.444 ms before and 74.544 ms after.

GPU regression checks establish resource/work bounds:

- Preloaded GAL atlases cause **zero pixel uploads** through the tilemap/2D path.
- Changing a 256-entry dynamic palette uploads **1,024 bytes**, versus 131,072
  bytes for recoloring the test's 256x128 RGBA image; the index texture stays put.
- Reusing a palette/atlas causes **zero texture-upload bytes** after warm-up.
- Warm masked preparations allocate **zero additional GAL resources** on the
  same target. Prepared generations retain their correct pixels in reverse
  submission order, and explicit release balances resource counters.
- Ordinary GPU geometry remains **80 bytes per quad**, with compatible batching
  and no fallback texture upload. Priority helpers cost two quads per sprite or
  background tile; they are optional.

These GPU checks measure bytes, allocations, batches and pixels. They do not
claim a GPU frame-time improvement.

## Remaining limitations

- Indexed storage currently encodes indices in RGBA8 (four bytes/index), with
  256x1 palette textures. This needs no extra GAL texture format; it is not a
  compressed-index storage format.
- There are eight mask bits, reset per canvas/prepare. Share one canvas to share
  masks; persistent mask state across separate passes is not provided.
- Priority ordering and game-specific OAM/scanline rules remain caller-owned.
  MicroMages/Jackal rules were checked using their current source plus synthetic
  conformance scenes, without porting or changing either game.
- Structural map/layer/tileset edits still rebuild a Runtime. Empty cells retain
  chunks, and new sparse cells remain subject to the existing 65,536 chunks per
  layer and 4,194,304 stored-cell limits. Automatic compaction is not included.
- OpenGL rendering remains blocked by the existing shader-convention mismatch.

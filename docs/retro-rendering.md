# Indexed 2D rendering, pixel priority and live tilemaps

Native graphics APIs now use unmodified wgpu through VulkanicGAL. See
[native migration](native-wgpu-migration.md) for native views/command buffers, WGSL, Drop ownership,
backend selection and replacement of old slot/receipt APIs. Historical benchmark
figures describe their recorded release.

These APIs use the existing `Canvas`, immutable `Textures`, GAL resource
registration, `DynamicTexture`, `AnimationClip`, asset server and tilemap runtime.
They require no ECS, window, editor, application runner or game-specific asset
format. CPU drawing/data operations work with renderer/tilemap defaults disabled.

## API audit and ownership

Before this change, `LoadedTilesets::load` always opened PNG files even when the
application already owned the pixels or GPU image. `Image` already shared its
pixel allocation through `Arc`; `Textures::declare_external` and
`RenderState2D::register_texture` already described and registered caller-owned
GAL images. The new `LoadedTilesets::from_textures` connects these APIs rather
than introducing another texture owner/uploader.

```rust,ignore
// Existing image service handle, e.g. server.load::<PngLoader>("atlas.png")?.
let texture = textures.insert((*image_asset).clone()); // Arc clone, no pixel copy
let tiles = LoadedTilesets::from_textures(&map, &textures, |_| Ok(texture))?;
```

The resolver receives the normalized tileset image path and runs once per unique
path. It may return an identity already used elsewhere in the application. The
constructor validates the map, image dimensions and atlas declarations without
file access or changes to the texture store. Multiple tilesets may share one
identity. Atlas metadata has no texture ownership: removing the identity makes
future canvases invalid. Dropping `LoadedTilesets` does not remove caller images.

For GAL images, call `RenderState2D::register_texture(gal, textures, source_view)`
and pass the returned identity to `from_textures`. The existing registration
rules apply: sampled 2D RGBA8/BGRA8, one mip/layer, same GAL; the renderer owns its
view/samplers/bindings while the source texture remains caller-owned. Order
producer writes before shader reads. No GPU texture or pixel upload is duplicated.
The CPU reference renderer explicitly rejects GPU-only images without CPU pixels.

`quasar-render-2d/assets` provides the optional `PngLoader` and an `Asset`
implementation for `Image`. This reuses the bounded PNG decoder and the mounted
asset service's cache, dependency metadata, async loading and invalidation model.
Source and decoded pixels each have a 64 MiB limit. It adds no dependency unless
selected. Tilequest now loads its atlas through this service and builds preloaded
tilesets. `quasar-tilemap` separates `render-2d` (atlas submission only) and `png`
(file loading, previews and PNG output); both remain enabled by default.

## Indexed images, banks and animation

`IndexedImage::new(width, height, indices)` validates one byte per pixel and
encodes index bytes into an ordinary immutable `Image`. Insert its shared
`image().clone()` once. The GPU representation is deliberately RGBA8 with the
index in red, so this needs no new GAL format, integer texture capability or
backend-specific upload path. Storage is four bytes per index on CPU/GPU after
encoding; this API optimizes reuse and palette changes, not compressed storage.

`Palette::new` accepts 1..256 `Color`s. `Palette::image()` makes a 256x1 RGBA
lookup texture, padding unused entries transparent. Insert each bank once.
`Canvas::set_palette(Some(PaletteBinding { texture, transparent_index }))`
captures the bank for subsequent draws; `None` restores ordinary RGBA sampling.
Transparent indices are discarded *before* lookup, tinting, blending or masking.
Thus index zero can be universal background color for one draw and transparent
for another. Identical RGB values do not imply identical occupancy.

Indexed source and palette lookup always use nearest texels, even when global or
sprite RGBA filtering is linear. UV regions, flips, transpose, camera, tint,
alpha blending, clipping, text and ordinary RGBA draws otherwise retain their
existing behavior. The palette itself can also be a registered GAL image.

`Palette::cycled(range, steps)` rotates a checked half-open range, retaining
other entries. Supply integer game time, e.g. `elapsed_ms / 100`. For a repeating
cycle, precompute/insert its small set of banks and select a texture identity.
For timed sequences, reuse `AnimationClip::frame_at(elapsed_ms)` as an index into
that bank list; the animation clip does not own textures or a clock.
For continuously changing colors, create a 256x1 `DynamicTexture` and stage
`palette.image().rgba()` through the existing update API. Both index and palette
resources use normal immutable dynamic upload generations retained by native
command buffers. Updating a palette uploads 1,024 bytes; it never reuploads the index
image. CPU snapshots use corresponding immutable palette images.

## Per-pixel masks and priority

`Canvas::set_pixel_state(PixelState)` captures an eight-bit mask rule per draw:

1. Rasterize the transformed, clipped quad and sample its color/index.
2. Discard a transparent/excluded index or a texel below `alpha_min` after tint.
3. Pass when `(stored & read_mask) == (reference & read_mask)`.
4. Replace `write_mask` bits with `reference` bits on covered, passing pixels.
5. Alpha-blend only when `color_write` is true.

`exclude_index` requires an indexed draw and operates independently of palette
color. `alpha_min` is an inclusive byte threshold. Defaults preserve ordinary
RGBA drawing, including its original blending rules. For custom mask writers,
set `alpha_min: 1` to exclude zero-alpha coverage (the default threshold is zero);
the priority helpers already do this. A mask-only sprite can
write a nonrectangular window from alpha/index coverage. A rectangle can clear
selected mask bits mid-canvas. Read and write comparisons use the same reference,
matching GAL stencil semantics; split a test and a conflicting write into draws.

`Priority2D::default()` reserves bit 0 for background occupancy and bit 1 for
sprite occupancy. `Priority2D::new` permits different distinct single bits;
remaining bits are available for other windows/groups. Two convenience methods
capture rules and restore the previous pixel state:

- `background_sprite(sprite, pose, priority, zero_index)` draws background color,
  clears occupancy for covered pixels, then marks its nonzero/index-opaque pixels.
  Use an indexed binding with `transparent_index: None` to draw the universal
  zero color. Existing background remains behind transparent RGBA texels.
- `priority_sprite(sprite, pose, priority, behind_background)` draws sprites in
  **highest-priority-first** order. Covered pixels claim sprite occupancy even
  when scenery hides their color. Holes claim nothing, so a later sprite can
  appear through them. Game-owned priority/OAM sorting stays outside Quasar.

Each helper emits two quads. Adjacent quads with identical texture, palette,
filter and pixel state batch, including mask writers. A passing replacement
writes the same reference bits used by its comparison, so another identical
writer sees the same pass/fail result; color blending retains primitive order.
A new priority group can clear its sprite bit using a mask-only rectangle before
drawing more sprites. All rules operate in final logical-screen space after the
existing transforms.

The CPU implementation uses a private byte plane allocated only for masked
canvases. GPU execution uses existing GAL stencil pipelines and a private
Depth24PlusStencil8 attachment per cached target size. The original color
attachment is reused; caller depth/stencil is untouched. Ordinary canvases need
no mask target. Mask state starts at zero for *each prepare/rasterize*, including
`LoadOp::Load`; color-load semantics are independent. To share masks, place the
operations in one canvas. This is not persistent cross-pass stencil state.

Private stencil targets are cached (up to eight sizes) and cleared each prepared
pass, then released by `release_targets`/`release` or owner Drop. Native command
buffers retain needed resources after cache invalidation. Quad storage remains
80 bytes with two integer effect words in axis padding. Source/palette native view
identities key bindings. Ordinary draws bind their source as the unused palette
input and need no fallback allocation/upload. All enabled backends use the same
WGSL/stencil declarations. Backend availability and native validation still depend
on driver features; GLES currently cannot compile these storage buffers on this
host's upstream-selected GLSL 3.3 target.

## Transactional live edits

`Runtime::set_tiles(&[TileEdit { layer, x, y, gid }, ...])` validates every edit
before mutating anything. Duplicate coordinates use the last GID; even overridden
entries must be valid. Checks include layers, GID ranges, diagonal-transform
shape restrictions, finite/infinite coordinates, aggregate cells and chunk
limits. Batches are bounded by `MAX_CELLS`. An invalid batch leaves document,
collision/query results, animation tables and indexes unchanged.

Existing cells are written in place, including empty GIDs. New sparse cells
append one-cell chunks and insert only their row/span entries. The row index now
uses ordered maps of spans, so insertion does not shift/rebuild all row spans.
There is no whole-map clone, cell scan, animation rebuild or index rebuild for a
runtime edit. `EditStats` reports changed existing cells, inserted cells and
newly indexed spans. Clearing a stored cell retains its allocation; this is not
chunk compaction. `Runtime::into_map()` transfers the edited document for export
without copying cells.

For `E` edits, preparation costs roughly `O(E log E + E log L + E log T + E log R
+ E log S)` with `O(E)` temporary mutation metadata: `L` layers, `T` tilesets,
`R` rows and `S` spans in a touched row. Existing-cell application is constant
work per unique edit; sparse insertion updates a row tree. The cost does not
scale with unrelated stored cells. Chunk-vector append remains amortized.

Public `Map` fields remain freely editable. `Map::set_tiles` therefore validates
that document once before its transactional batch, with no full-map clone; its
lookup scans affected layers' chunks. Use `Runtime` for repeated live edits.
Structural edits to layers, tilesets or chunk shapes still require a new Runtime.
The on-disk map schema and ordinary CLI behavior remain compatible.

## MicroMages and Jackal comparison (read-only)

The audit inspected these local revisions without changing either repository:

| Project | Revision | Relevant source and requirement |
| --- | --- | --- |
| MicroMages | `9740d30a6241a9e7e0e265e2f00adbf17fba3eeb` | `src/assets.rs` encodes four shades; `src/video.rs` tracks nonzero background occupancy separately from universal background color, discards sprite shade zero, flips atlas tiles and hides players behind doorway pixels; `src/enemy.rs` swaps palettes for freeze/hit feedback. |
| Jackal | `b6d90a1c999dcb9d6f82cdc0e88ef876aac4ee77` | `src/renderer.rs` computes rotating OAM priorities, sorts scene parts and restores opaque background under behind-background sprites. Its explicit regression proves that a hidden high-priority sprite also hides lower sprites, while transparent sprite pixels allow them through. |

Synthetic conformance tests reproduce these rules without copying game assets or
changing game renderers. The API accepts source indices directly, so an adapter
can retain MicroMages' four-shade assets and game-owned palette selection. Jackal
can retain its verified airbase background-color-to-occupancy mapping and OAM
rotation; submit its sorted parts highest first. Existing RGBA sprite assets can
use alpha coverage. Mapping game-specific levels, collision semantics, timing,
OAM rotation, sprite limits, scanline effects or hardware emulation remains the
application's responsibility. This validates the required compositing rules,
not an end-to-end port or golden-image equivalence for either external game.

## Reproducing checks and examples

```sh
python3 tools/full_checks.py -s retro-2d
cargo test --locked --workspace
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked -p quasar-render-2d -p quasar-tilemap -p tilequest -p quasar-composition --all-targets --all-features -- -D warnings
cargo test --locked -p quasar-render-2d --all-features -p quasar-composition -p tilequest --lib -- --include-ignored --test-threads=1
WGPU_BACKEND=vulkan cargo test --locked -p quasar-render-2d --all-features --lib -- --include-ignored --test-threads=1
cargo test --locked -p quasar-tilemap --release benchmark_live_edits -- --ignored --nocapture
python3 tools/verify_tilemap.py
python3 tools/benchmark_tilemap.py --output /tmp/quasar-map-benchmark.json
cargo run --locked -p tilequest --no-default-features --example retro -- /tmp/quasar-retro.png
cargo run --locked -p tilequest --example retro -- --gpu /tmp/quasar-retro-gpu.png
```

Tests cover mounted asset-cache/Arc sharing, preloaded metadata errors, real GAL
atlas reuse with no upload, palette validation/cycling/switching/animation,
transparent indices, all eight tile orientations, pixel masks and clipping,
MicroMages/Jackal priority cases, reverse prepared-frame submission and palette
native retention, zero-image-upload unchanged preparations, incremental-vs-rebuilt randomized
queries, finite/negative coordinates, duplicate edits, pointer-stable existing
cell buffers and all-or-nothing limits. GPU output is compared to independent CPU
reference pixels (exact opaque colors; at most one byte for blended channels).

See [the implementation report](retro-rendering-report.md) for completed checks,
measured edit/culling changes, GPU resource counters and remaining limitations.

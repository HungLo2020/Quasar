# quasar-tilemap

Versioned map data, Tiled JSON import/export, indexed spatial queries, and optional
2D draw-list generation. The runtime owns no window, GAL, game loop, or physics
policy. `render-2d` (default) consumes `quasar-render-2d` without its GPU feature.
Disable defaults for a serde-only parser/query library and conversion CLI.

```sh
cargo run -p quasar-tilemap -- validate examples/tilequest/assets/island.tmj --assets
cargo run -p quasar-tilemap -- inspect examples/tilequest/assets/island.tmj
cargo run -p quasar-tilemap -- build examples/tilequest/assets/island.tmj --output /tmp/island/map.json
cargo run -p quasar-tilemap -- convert /tmp/island/map.json --tiled --output /tmp/tiled/map.tmj
cargo run -p quasar-tilemap -- preview /tmp/island/map.json --camera 100 100 --size 480 320 --time 800 --output /tmp/island.png
cargo run -p quasar-tilemap -- set /tmp/island/map.json --layer 1 --cell 10 10 --gid 2 --output /tmp/edited/map.json
```

Results and errors are JSON on stdout. Exit codes: 0 success, 1 invalid data or
I/O failure, 2 usage. `--help` lists options. `--assets` validates referenced files,
PNG decoding and declared atlas dimensions. Build/convert/set also validate assets
with the default feature, copy declared images/file properties alongside output,
and retain portable relative paths. No editor or GUI toolkit is required.

## Data contract

Normalized JSON has `quasar_version: 1`; see [schema-v1.json](schema-v1.json).
Future versions fail explicitly: migration must be deliberate. Unknown normalized
fields fail; extensions belong in `metadata`. Metadata uses sorted keys for stable
serialization. Typed properties are preserved; `type: file` paths participate in
bundling. Other property types remain opaque. This is a runtime format, not a
promise to reproduce arbitrary Tiled documents byte for byte.

Supported [Tiled JSON](https://doc.mapeditor.org/en/stable/reference/json-map-format/):
orthogonal, right-down maps; finite array tile layers and infinite array chunks
(including negative coordinates); inline or external TSJ atlas tilesets; PNGs;
uniform map-sized cells, margins/spacing; timed tile animations; ordered layers,
visibility, tint, opacity, pixel offsets and parallax; rectangular/point objects;
typed properties and extension metadata. External tilesets are expanded on export.
Finite chunks export as a full layer array. All four high GID flag bits are cleared
for tile lookup; supported orthogonal transforms apply transpose then horizontal
and vertical flips, following [Tiled's GID contract](https://doc.mapeditor.org/en/stable/reference/global-tile-ids/).
Diagonal transforms require square tiles.

Unsupported semantics fail explicitly: group/image layers, non-orthogonal maps,
other render orders, compressed/base64 data, blend modes, nonzero parallax origins
or tileset offsets, transparent-color keys, image-collection tilesets, embedded
tile collision shapes, rotated/polygon/ellipse/polyline/text/tile/template objects.
No TMX/TSX XML support. Preprocess these with Tiled or extend the importer before
using them. Unknown metadata is retained but does not enable rendering semantics.

Limits include 64 MiB aggregate input JSON, 4,194,304 stored cells, 1,024 layers
and tilesets, 1,024px tile dimensions, and 16,384px atlas dimensions. PNG decode
buffers cap at 64 MiB per image; loaded atlas pixels cap at 256 MiB total. APNG is
rejected. Asset resolution rejects absolute paths, root escape and symlink escape.
Build outputs must not alias an asset. Images are shared by normalized path.

## Runtime and drawing

`load(path)` returns validated `Map` plus `AssetRoot`; `Runtime::new(map)` builds
per-layer row/span indexes with transactional incremental edits. `tile`, `for_each_cell`, `collides`,
`objects`, and `objects_in_bounds` operate without a renderer. Queries use half-open
world-space bounds and layer offsets. Parallax affects drawing only. Collision
checks a caller-selected boolean property on the layer or the base tile; visual
animation, hidden layers and opacity do not alter collision. Point objects use
point inclusion; zero-area rectangles do not collide. `animated_tile` resolves
explicit caller-provided milliseconds and returns `None` for invalid indices.

`Runtime::set_tiles(&[TileEdit { layer, x, y, gid }, ...])` validates a batch before
changing any cell. Existing cell storage is preserved; new sparse cells update
only their row/span entries. Duplicate coordinates use the last GID.
`Map::set_tiles` validates the public document once without cloning it; repeated
live edits should use Runtime. Structural map changes still require rebuilding.
`LoadedTilesets::load` decodes PNGs and inserts images into a caller-owned texture
store. `LoadedTilesets::from_textures` accepts existing image/asset-service or
registered GAL texture identities without I/O, pixel copies or GPU uploads.
The optional `png` feature enables file loading and previews; `render-2d` alone
needs no decoder or GPU. `draw_layer` appends visible tiles to a caller-owned Canvas, culls before
emitting quads, restores its previous camera, and returns traversal counts. The
caller decides layer/actor ordering and submits through its existing renderer/GAL.
The Tilequest example inserts actors between collision and foreground layers.
Runtime animation requires no texture upload or map mutation each frame.

```sh
cargo test -p quasar-tilemap
cargo test -p quasar-tilemap --no-default-features
python tools/verify_tilemap.py
cargo run --release -p quasar-tilemap -- benchmark examples/tilequest/assets/island.tmj --size 480 320 --iterations 1000
```

The benchmark reports CPU load/index time and indexed culling/draw-list p50/p95;
PNG decoding is excluded from the draw-list timing. It does not measure GPU time.
Tilequest's separate native benchmark measures GPU rendering/presentation.

See [indexed rendering and live tilemaps](../../docs/retro-rendering.md) for palette
animation, pixel priority, ownership, complexity, game audits and validation.

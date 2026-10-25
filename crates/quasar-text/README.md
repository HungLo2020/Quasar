# quasar-text

Independent font loading, Unicode shaping, text layout, cursor/selection
geometry and CPU glyph atlases for Rust games.

`quasar-text` does **not** depend on Vulkan, a window, an ECS, an editor or an
application runner. Callers register their own font bytes; the crate shapes with
cosmic-text/Harfrust, lays out wrapped and bidirectional text, and rasterizes
glyphs through swash into cache-friendly atlas pages. Bitmap-only games do not
compile this crate.

```toml
quasar-text = { path = "crates/quasar-text" }
```

```rust
use quasar_text::{FontData, FontStack, TextStyle};

let mut fonts = FontStack::new();
fonts.add_font(FontData::new("font.ttf", std::fs::read("font.ttf")?));
fonts.with_default_family("DejaVu Sans");
let mut engine = fonts.build("en-US")?;
let layout = engine.layout("Hello, नमस्ते", &TextStyle::new(20.0), Some(480.0))?;
assert!(layout.width() > 0.0);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Features

| Feature | Default | Adds |
|---|---|---|
| *(none)* | yes | Font loading, shaping, layout, caret/hit/selection, swash rasterizer, CPU atlas |
| `serde` | no | Serialization for `TextStyle`, `TextAlign`, `TextWrap`, `AtlasConfig` |

The GPU adapter lives in `quasar-render-2d/text`; the widget adapter lives in
`quasar-ui/text` and `quasar-ui/text-render2d`.

## What it owns

- `FontStack` / `TextEngine`: deterministic private font database, family/
  weight/style matching, explicit fallback order, locale-aware shaping and a
  bounded layout cache.
- `TextLayout`: immutable shaped glyphs, lines, bidi levels, caret positions,
  hit testing, grapheme boundaries and selection rectangles.
- `GlyphRasterizer` / `GlyphAtlas` / `GlyphCache`: swash rasterization keyed by
  face, glyph, size, weight, cache flags and subpixel bins; shelf-packed pages;
  versioned dirty-page reporting.
- `TextQuad`/`TextFrame`: renderer-neutral tinted atlas quads in physical pixels.
- `FontData` license metadata so games can retain the notices they ship.

## Design choices

- **Established shaping:** cosmic-text/Harfrust, fontdb and swash are reused.
  Quasar does not implement a shaping engine.
- **No system font scan:** registered fonts are the complete fallback universe,
  which keeps tests and cross-platform behavior deterministic. Add system fonts
  explicitly if desired.
- **TTF and OTF:** both are loaded through skrifa/fontdb; collections contribute
  every face.
- **Raster masks, not SDF:** UI workloads use a small set of DPI-derived sizes.
  Hinted swash masks are sharper there and avoid a second shader/cache path.
- **Renderer-neutral output:** CPU atlases and quads can feed a software
  framebuffer or a GPU adapter. No draw submission is hidden.

See [docs/text.md](../../docs/text.md) for the audit of MattMC, MicroMages,
Jackal, MissileCommand and the unavailable BlackOps repository, plus licensing,
supported scripts, measurements, integration examples and limitations.

## Tests and benchmark

```sh
cargo test -p quasar-text
cargo test -p quasar-text --features serde
cargo run --release -p quasar-text --example text_benchmark
```

Tests cover TTF/OTF loading, weight/family selection, Devanagari fallback,
Arabic/Hebrew bidi carets and selection, grapheme editing boundaries, wrapping,
alignment, DPI sizes, missing glyphs, atlas versioning/cache reuse and invalid
inputs. The ignored Vulkan tests in `quasar-ui` cover 2× DPI, clipping,
resizing, unchanged-page upload reuse and resource lifetimes.

## Measurements

Development-host release run with DejaVu Sans + Noto Devanagari:

| Operation | Result |
|---|---:|
| Cached layout lookup | ~0.12–0.15 µs/call |
| First frame, 83 glyphs (60 unique) | ~0.7–0.9 ms |
| Repeated frame, 72 quads | ~5.7–7.3 µs/frame |
| Atlas | 1 × 1024² page, 4 MiB |

Single-host regression numbers, not portability claims.

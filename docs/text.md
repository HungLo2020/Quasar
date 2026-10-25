# Fonts, shaping and text rendering

Quasar's text stack is split into an independent shaping/layout/atlas crate
(`quasar-text`), an optional GPU upload adapter in `quasar-render-2d`, and an
optional UI adapter in `quasar-ui`. Games that only need the original 5×7 bitmap
font compile none of the font dependencies.

```text
caller-owned font bytes
        │
        ▼
quasar-text::FontStack ──► fontdb (private, deterministic face set)
        │
        ▼
TextEngine (cosmic-text: harfrust shaping + fontdb fallback + unicode-bidi)
        │  layout(text, style, width) -> immutable TextLayout
        │      glyphs, lines, caret, hit-test, selection rectangles
        ▼
GlyphCache (swash rasterizer + shelf-packed CPU atlas pages)
        │  TextFrame { quads, dirty pages }
        │
        ├────► any custom renderer / software framebuffer
        │
        ▼
quasar-render-2d::GpuTextAtlas ──► RenderState2D::DynamicTexture
        │
        ▼
Canvas glyph quads on the caller's existing VulkanicGAL frame
        │
        ▼
quasar-ui::TextRenderer ──► widget text, editing carets and selection
        │
        ▼
UiRenderer::draw / caller-owned Canvas / caller-owned submission
```

## Audited implementations

These audits were read-only. No files in the audited repositories were changed.

| Project | Font implementation | Layout / editing | Unicode and scaling |
|---|---|---|---|
| **MattMC** | Minecraft 1.21 Java client. `font/*.json` provider stacks reference bitmap atlases (`nonlatin_european.png`, `accented.png`), unihex `unifont`/`unifont_jp` packs and a FreeType-backed TrueType/OpenType provider (the checked-in default stack uses bitmap + unihex; TTF/OTF provider support is present with no bundled TTF required). `GlyphStitcher` bakes glyphs into font textures. | `Font.lineHeight = 9`; `StringSplitter` wraps by advance; `TextFieldHelper` implements codepoint cursor/selection; `Font.bidirectionalShaping` uses ICU `Bidi` + `ArabicShaping`. | Providers can cover a very large Unicode range. TTF/OTF glyphs are rasterized by FreeType, oversampled, then baked. Font fallback is an ordered provider list with optional filters. |
| **MicroMages** | `src/font.rs` is a hard-coded 5×7 bitmap table; menu/HUD draw scaled rectangles. No font file, atlas or shaping library. | 6-pixel advance, integer scale, uppercase/ASCII subset, manual centered helpers. | No Unicode beyond the table; no kerning, wrapping or bidi. Crisp pixel output at integer scales. |
| **Jackal** | `assets/fonts/hud.png` is a 96×16 sheet of twelve 8×16 score/label glyphs extracted from the original HUD; a separate 5×7 table drives menu text. | HUD draws fixed 8×16 sprite regions; menu text uses a 6-pixel advance and 8-pixel line step. | ASCII uppercase/digits only, no shaping. Palette/atlas fidelity was the priority. |
| **MissileCommand** | Hard-coded 8×8 bit patterns in `src/render.rs`; no font asset or atlas. | 8-pixel advance, integer scale, manual centering; uppercase/digits/punctuation. | ASCII only; 256×240 software framebuffer. |
| **BlackOps** | Not present on this host. No audit is claimed. | — | — |

The common pattern is a lightweight pixel font with integer scaling for the
retro games and a provider/atlas system for the large Java client. MicroMages,
Jackal and MissileCommand are uppercase ASCII/digit workloads with fixed cell
advances; they need crisp integer scaling, not shapes. MattMC needs broad
Unicode coverage, ordered fallback, TTF/OTF rasterization, ICU bidi and a
multi-line editor. Quasar therefore keeps both paths: bitmap text remains the
default, while TTF/OTF shaping and fallback are opt-in. The unavailable BlackOps
repository is not represented in the audit.

## Library choice

`quasar-text` uses **`cosmic-text` 0.19** rather than a custom shaper. It provides:

- **Harfrust** shaping (a Rust HarfBuzz port): kerning, ligatures, cluster
  mapping, script/language features and complex-script shaping.
- **fontdb** face matching for family, weight, stretch and style, plus
  deterministic fallback across registered faces.
- **unicode-bidi** paragraph ordering for mixed LTR/RTL text.
- Line breaking, word wrapping and alignment.
- Cursor hit-testing and visual cursor positions tied to shaped glyphs.
- **swash** glyph rasterization with subpixel bins and alpha masks.

Rustybuzz, swash and fontdb were evaluated individually. Cosmic-text combines
them into a coherent layout/cursor model, which avoids reimplementing bidi and
cluster boundary rules in Quasar. Fontdue was not selected because it provides
rasterization without shaping or bidi. No custom shaping engine is present.

System font discovery is deliberately disabled (`fontconfig` is off). The
caller's registered faces are the complete fallback universe, so Windows,
macOS, Linux and CI produce the same face set for the same assets. Applications
that want system fallback can add those font bytes explicitly.

## Feature isolation

| Crate / feature | Adds | Default |
|---|---|---|
| `quasar-text` (none) | font loading, shaping, layout, cursor/selection, CPU atlas | yes |
| `quasar-text/serde` | serializable styles and atlas configuration | no |
| `quasar-render-2d/text` | `GpuTextAtlas`, persistent dynamic texture pages | no |
| `quasar-ui/text` | `ShapedTextMeasure`, `UiTextStyles` | no |
| `quasar-ui/text-render2d` | `TextRenderer` widget integration | no |
| `quasar-ui/render2d` | bitmap `UiRenderer` only | no |

The core `quasar-ui` and `quasar-render-2d` libraries still have no text/font
dependency. `Canvas::text` and `TextureId::FONT` remain source-compatible.

## Font loading

```rust
use quasar_text::{FontData, FontStack, TextStyle};

let mut fonts = FontStack::new();
fonts.add_font(
    FontData::new("DejaVuSans.ttf", std::fs::read("DejaVuSans.ttf")?)
        .with_license("Bitstream Vera"),
);
fonts.with_default_family("DejaVu Sans");
let mut engine = fonts.build("en-US")?;
let layout = engine.layout("Hello, नमस्ते", &TextStyle::new(20.0), Some(480.0))?;
```

- TTF and OTF are both accepted by fontdb/skrifa. Collections contribute each
  face.
- The engine owns all font bytes for its lifetime; callers can also keep an
  `Arc<[u8]>` and register it without copying into a filesystem path.
- `default_family` controls `Family::SansSerif`; a family can still be selected
  explicitly in `TextStyle`.
- Fallback scans the registered faces for a face that supports the missing
  codepoint. Registering a primary font plus script fonts gives deterministic
  fallback order.
- `TextEngine` caches shaped layouts by text, style and width. Cache capacity is
  bounded (512 by default, 4096 for the UI renderer) and evicts in insertion
  order. Identical frames reuse layouts instead of reshaping.

## Layout, bidi and editing

`TextLayout` owns the shaped buffer and exposes:

- `glyphs()`: visual-order glyph IDs, font IDs, x/baseline, advances, line
  metrics, byte/char cluster ranges and RTL level.
- `lines()`: visual-line top/baseline/height/width, direction and character
  range.
- `caret(char_index)`: visual caret x/top/height, including RTL runs.
- `hit(x, y)`: character index at a layout point.
- `selection_rects(anchor, cursor)`: visual rectangles for mixed-direction text.
- `previous_boundary` / `next_boundary`: grapheme-cluster boundaries.

Multiple paragraphs are separated by `\n`; `TextWrap::{None, Word, Glyph,
WordOrGlyph}` and `TextAlign::{Start, Center, End, Justify}` are per-layout
attributes. The cache key includes all of them.

`quasar-ui` extends its existing `TextMeasure` trait rather than replacing it:

```rust
fn measure_style(&self, text: &str, style: TextStyle, size: f32) -> Vec2;
fn caret_index(&self, text: &str, style: TextStyle, size: f32, x: f32) -> Option<usize>;
fn caret_x(&self, text: &str, style: TextStyle, size: f32, index: usize) -> Option<f32>;
fn previous_boundary(&self, text: &str, index: usize) -> usize;
fn next_boundary(&self, text: &str, index: usize) -> usize;
```

`ShapedTextMeasure` implements these using the same `TextEngine` that
`TextRenderer` uses. `TextInput` stores a horizontal `text_scroll` on
`UiNode`, updates it from the shaped caret, and uses the shaped hit test for
pointer selection. The renderer uses the same layout for selection rectangles
and the caret, so clicks, keyboard movement, scrolling and pixels cannot drift.
Shift-selection, drag-selection, Select All, grapheme-aware backspace/delete,
placeholder text, password masking and IME preedit are supported. Committed IME
text remains separate from preedit (`UiInput::text` vs `UiInput::preedit`);
preedit is rendered with an accent underline and never inserted until commit.

`quasar-window` now forwards winit `Ime::Preedit` into
`InputState::preedit`/`preedit_cursor`; `UiBindings`/`from_input_state` expose
it to UI. Starfall enables IME while its menus are available.

## Rasterization and caches

- `GlyphRasterizer` asks swash for a mask at the physical font size and
  quantized x/y subpixel bin.
- `GlyphKey` includes font ID, glyph ID, size bits, weight, cache flags and
  subpixel bins. Unchanged keys reuse the existing atlas entry; no rerasterizing
  or atlas write occurs.
- `GlyphAtlas` uses shelf packing with configurable page size (default
  1024×1024), padding and page cap (8). Alpha masks are expanded to white RGBA
  with coverage in alpha; color glyphs copy RGBA. Quads are tinted at draw time.
- Pages track a monotonically increasing version. A caller that only needs CPU
  output can consume `TextFrame { quads, pages_dirty }` and upload pages itself.
- `GpuTextAtlas` creates one `RenderState2D::DynamicTexture` per atlas page and
  uploads a page only when its version changes. This preserves texture identity
  and avoids per-frame glyph uploads. It is integrated into the same GAL frame
  and submission as the existing 2D/3D renderers.
- `Canvas` draws glyphs as ordinary fractional-position sprites with linear
  filtering and straight alpha blending. Clip rectangles are captured per
  draw, so UI panel clipping applies to glyphs, carets and selection.
- Batches are formed by texture (atlas page) and filter; glyphs on one page are
  one instanced batch in the normal case.

### Why no SDF/MSDF yet

The measured workload is UI text at a small set of DPI-derived sizes (1×, 1.25×,
2×). Swash's hinted alpha masks are sharper at those sizes than a single SDF
atlas, and a pixel-size keyed atlas avoids blurry upscaling. The atlas cost is
bounded and measured; an SDF path would add shader complexity and a second glyph
cache without evidence that arbitrary continuous scaling is required. SDF/MSDF
remains a future feature if a game needs continuously animated text scale or
very large display text.

## UI integration

`TextRenderer` is constructed once from a shared `TextEngine`:

```rust
let engine = Rc::new(RefCell::new(fonts.build("en-US")?));
let styles = UiTextStyles::new("DejaVu Sans");
let renderer = TextRenderer::new(engine, styles, TextRendererOptions::default())?;
let measure = renderer.measure();       // pass to Ui::update
let mut painter = UiRenderer::new();
painter.enable_text(renderer);
// once per frame, after Ui::update and before drawing:
painter.prepare_text(&mut state_2d, &mut gal, &mut textures, &ui)?;
painter.draw(&mut canvas, &ui);
```

`UiTextStyles` maps `TextStyle::{Body, Heading, Small, Dim}` to family, weight,
italic and line-height. `TextRenderer` covers labels, buttons, slider labels and
values, checkboxes, selects, tabs, text fields, progress labels, lists, binding
rows and modal text. It draws the same widget chrome as the bitmap path and
falls back to bitmap rendering if no text pages have been prepared.

Text input scroll uses physical shaped caret positions; the renderer clips to
the field's inner rectangle. Selection is drawn behind glyphs. The caret and
selection use the same glyph layout, not a separate character-width estimate.

## Demo usage

**Starfall** (`Theme::modern`) loads `DejaVuSans.ttf`,
`DejaVuSans-Bold.ttf` and `NotoSansDevanagari-Regular.ttf` from
`examples/starfall/assets/fonts`. The main menu shows Greek, Cyrillic, Devanagari
(fallback face), Hebrew/Arabic bidi text; options include a Unicode callsign text
field with IME preedit support, persisted in `starfall-ui.toml`. Native runs are
verified at 1× and 2× DPI.

**Falling Sand** deliberately keeps `Theme::retro`, `BitmapTextMeasure` and the
5×7 bitmap font. It does not enable any font feature and its pixel menus are
unchanged.

## Font licensing

The demo assets are redistributed with their license notices in
`examples/starfall/assets/fonts/`:

- `DejaVuSans.ttf`, `DejaVuSans-Bold.ttf` — Bitstream Vera license; DejaVu
  changes are public domain. See `LICENSE-DejaVu.txt`.
- `NotoSansDevanagari-Regular.ttf` — SIL Open Font License 1.1. See
  `LICENSE-Noto.txt`.
- The OTF load test uses `NimbusSans-Regular.otf` with `LICENSE-URW.txt` under
  `crates/quasar-text/tests/fonts/`.

Games are responsible for licensing fonts they ship. Quasar does not download
fonts or scan system font directories.

## Supported scripts

Bundled demo fonts cover Latin, Latin Extended, Greek, Cyrillic, Hebrew, Arabic,
Armenian, Georgian and Devanagari (with complex shaping). Harfrust can shape
other scripts when the caller supplies suitable font bytes. CJK, emoji and
script-specific fonts are not bundled; add Noto/other OFL fonts explicitly.
Color glyphs are accepted by the atlas, but no color emoji font ships with the
demo.

## Measurements

`cargo run --release -p quasar-text --example text_benchmark` on the development
host (release, DejaVu Sans + Noto Devanagari, 420px wrapped text):

| Operation | Result |
|---|---:|
| Cached layout lookup | ~0.12–0.15 µs/call |
| Cold layout (unique text) | ~40–45 µs/call |
| First frame, 83 glyphs (60 unique) | ~0.7–0.9 ms |
| Repeated frame, 72 quads | ~5.7–7.3 µs/frame |
| First GPU atlas upload | 4 MiB (one 1024² page) |
| Repeated-frame GPU upload | 0 bytes |
| Atlas pages / memory | 1 page / 4 MiB (1024² RGBA) |
| Rasterized glyphs after first frame | 60 |
| Additional rasterizations on repeated frame | 0 |

The UI benchmark (`quasar-ui --example ui_benchmark`) remains ~22–28 µs/update
for 129 bitmap-text nodes; shaped text is measured separately because layout
caching and DPI are different workloads. Native Vulkan tests verify resource
balance and no re-upload on unchanged pages.

## Tests and verification

Headless `quasar-text` tests cover TTF and OTF loading, family/weight selection,
Harfrust shaping, Devanagari fallback, bidi caret/selection, grapheme
boundaries, wrapping/alignment/reflow, missing glyphs, cache reuse, atlas page
versions, DPI sizes and invalid inputs. `quasar-ui` tests cover shaped
measurement, pointer hit-testing against caret layout, grapheme editing,
selection, scrolling and IME preedit. Ignored Vulkan tests cover shared-GAL text
rendering, 2× DPI, clipping, resizing, unchanged-page upload reuse and resource
lifetime balance. `cargo test --workspace`, strict Clippy, fmt, docs and native
Starfall screenshots are listed in the release report.

## Known limitations

- No system font discovery by default; applications must register every face
  they want to use. This is intentional for deterministic fallback and testing.
- No bundled CJK/emoji font. Fallback is only as broad as the registered faces.
- No rich-text spans inside one `Widget` label, inline images, ruby or vertical
  writing.
- UI labels are single-line unless they contain explicit `\n`; the layout engine
  supports wrapping but no current widget exposes a wrap-width property.
- No clipboard copy/paste commands in `TextInput` yet; selection, drag, Select
  All and IME preedit are implemented.
- Atlas pages are uploaded whole when a new glyph is added. Unchanged pages are
  not uploaded, but sub-rectangle uploads are future work. Default page memory
  is 4 MiB per active page.
- No SDF/MSDF path; see the measured rationale above.
- Font variations are selected through weight/italic matching only; arbitrary
  OpenType variable-axis values are not exposed yet.
- Atlas pages are a fixed size chosen at construction; changing page size
  requires a new cache.
- Text shaders use the existing 2D sampler; no specialized subpixel LCD
  rendering is claimed.
- `TextRenderer` uses `Rc<RefCell<TextEngine>>` for cheap shared measurement and
  drawing on one UI thread. Build one renderer per UI thread; it is not `Send`.
  The independent `quasar-text` engine can still be owned by whatever thread
  performs shaping.

## Release readiness

The feature is additive and opt-in. Bitmap-only games keep their existing
dependencies and APIs. The independent text crate has ordinary Cargo usage and
no Vulkan/window/ECS dependency. Starfall demonstrates real TTF text, Unicode,
fallback, bidi, DPI, editing and persistence; Falling Sand proves the retro path
is untouched. Remaining work is limited to the explicit limitations above, not
to missing acceptance-critical functionality.

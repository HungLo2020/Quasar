# quasar-render-2d

An ordered 2D renderer that consumes VulkanicGAL's public API. It imports no
native graphics APIs or GAL backend implementations. VulkanicGAL owns driver
abstraction, shaders, resources, synchronization, and surfaces. Windowed
presentation uses `quasar-frame`'s `FrameSurface` for target caching and
acquire/present; `RenderState2D` composition does not require a surface.

```rust
use quasar_render_2d::{Canvas, Rect, Textures, Renderer2D};
let textures = Textures::default();
let mut canvas = Canvas::new(640, 400)?;
canvas.rect(Rect::new(20.0, 30.0, 80.0, 40.0), [92, 230, 204, 255]);
canvas.text([20.0, 90.0], "QUASAR", 2.0, [255; 4]);
let mut renderer = Renderer2D::headless()?;
let rgba = renderer.render_to_image(&canvas, &textures)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

For a desktop surface, construct `Renderer2D::for_window` with an owned window
`Arc`, physical surface dimensions, and `DisplayOptions`, then call
`present(&canvas, &textures)`. `for_window` is the Vulkan convenience path; it
is available with the `vulkan` feature. Backend-neutral callers create a
windowed `VulkanicGal` themselves and use
`Renderer2D::from_windowed_gal(gal, size, options)`. Forward physical size changes to `resize`.
Zero-sized surfaces defer presentation. Fit, integer scaling, aspect overrides,
and nearest/linear texture sampling share viewport math with the framebuffer
presenter.
The logical canvas dimensions determine the game coordinate system.

`Image::new` accepts immutable RGBA8 data. Insert it into `Textures`, retain the
returned `TextureId`, and draw `Sprite`s. Source rectangles use texture pixels;
destinations use world coordinates. `set_transform` and `set_camera` apply
affine transforms. `set_clip` captures a logical screen rectangle per draw;
clipping happens in the fragment shader. Keep a texture store across frames.
Removing an image invalidates its ID; GPU copies enter GAL retirement on the
next valid frame, retaining resources needed by pending or incomplete draws. IDs cannot alias images in another store. Built-in white and font
textures cannot be removed.

Draw order is preserved, using straight alpha blending. Adjacent quads with the
same texture and filter become one instanced draw. Images are uploaded once and
cached; ordinary frames upload geometry and uniforms. Buffer growth rebuilds
only affected buffer bindings, preserving image uploads. `stats()` reports
quad/batch counts and upload bytes. The built-in bitmap font
covers uppercase ASCII, digits, and simple punctuation. The optional `text`
feature adds a `GpuTextAtlas` adapter for `quasar-text`: shaped Unicode text is
rasterized into persistent glyph atlas pages, uploaded only when a page version
changes, and drawn as fractional-position tinted quads on the same caller-owned
`Canvas`. No scene graph or window is added.

The default features are `gpu` and `vulkan`; the optional `text` feature adds
`quasar-text` and is off by default. `GpuTextAtlas::sync` creates one
`DynamicTexture` per atlas page and reuses unchanged pages without re-uploading
them. Bitmap-only games compile no font dependencies.

The default features are `gpu` and `vulkan`; `headless()` therefore uses the
Vulkan implementation by default. `headless_with(BackendChoice)` requests a
specific compiled backend. `opengl` is available as an alternative Cargo
feature, but this crate does not create an OpenGL window or context and does not
swap OpenGL buffers; use `from_windowed_gal` with a caller-owned context and
presentation path. Three bounded transient slots track prepared command
references and GPU completion. Native presentation retains synchronous
retirement by default; `set_frame_overlap(true)` opts into completion polling.
Readback explicitly waits. `from_gal` accepts an owned compatible GAL; the GAL
currently permits one live instance per process. Linux Vulkan and wgpu have pixel-parity coverage. The OpenGL Cargo feature
compiles, but renderer initialization currently rejects its GLSL convention.

With `default-features = false`, drawing and `reference::rasterize` compile
without windowing or Vulkan. This CPU reference path supports headless snapshots
and conformance tests. Native presentation uses the GPU quad shaders.
`render_to_image` uses those same shaders and reads real GPU pixels back as
logical-size RGBA8, rather than capturing OS window chrome or letterboxing.

Run the driver-dependent checks explicitly:

```sh
cargo test -p quasar-render-2d -- --include-ignored
cargo test -p quasar-render-2d --no-default-features
cargo check -p quasar-render-2d --no-default-features --features gpu,opengl
```

## Caller-owned composition (Git tag `2.1`)

`RenderState2D::new(&mut gal)` owns rendering resources, without owning GAL or a
surface. Use `prepare(&mut gal, &canvas, &textures, target, options, LoadOp::Load)`
to preserve an existing image, or `LoadOp::Clear` to clear with `canvas.clear`.
The result contains a normal GAL `CommandList` and draw/upload statistics.
The caller orders its passes and submits the list on that same GAL. The HUD
omits depth attachment use, so it does not alter the world's depth image.

The caller transitions offscreen color attachments to `ColorAttachment` before
first use and adds explicit dependencies when moving between attachment,
sampled and copy uses. GAL manages acquired swapchain-image transitions. See
[the composition example](../../examples/composition/README.md) for a complete
world/HUD transaction, including an explicit orchestration path and an optional
`quasar-frame` path. The windowed renderer uses `quasar-frame`'s surface helper;
`RenderState2D` itself has no surface dependency.

`register_texture(&mut gal, &mut textures, source_view)` validates a same-GAL,
sampled single-mip 2D RGBA8/BGRA8 view and makes a private view of the existing
image. It returns a normal `TextureId`; no image copy or CPU pixels are required.
The source owner controls texture contents and orders writes before sampling.
Publish a new ID when replacing a generation, then remove the old ID. The CPU
reference renderer returns an error for GPU-only images.

## Software framebuffers and dynamic textures

`FramebufferPresenter::new(window, window_size, source, options)` is the
Vulkan-backed windowed path for a CPU-rendered RGBA8 framebuffer.
`FramebufferPresenter::from_renderer(renderer, source)` wraps a caller-created
windowed renderer instead. Call
`present(&pixels)` with tightly packed, top-to-bottom pixels; scaling,
filtering, letterboxing, resize, deferred surfaces and completion polling use
the same code as `Renderer2D`. `render_to_image` returns the source-resolution
pixels through the same GPU pipeline.

For shared-device composition, use the lower-level dynamic-texture API:

```rust
let id = state.create_dynamic_texture(&mut gal, &mut textures, width, height)?;
state.update_dynamic_texture(id, &rgba)?;
let prepared = state.prepare(&mut gal, &canvas, &textures, target, options, LoadOp::Load)?;
```

The texture, view, samplers and bindings are created once. `update_dynamic_texture`
stages pixels; `prepare` consumes them into the prepared command list using a
per-frame upload slot. Each command list owns its framebuffer snapshot, so
frames submitted out of preparation order keep their own pixels. Up to three
slots keep uploads from overwriting in-flight frames; slots are reused only
after their submission completes. A dynamic texture must be updated before each
`prepare` that draws it. `destroy_dynamic_texture` releases its GPU resources;
dynamic textures are not reclaimed by ordinary `Textures` removal.

Prepared command lists contain `TrackSubmission` receipts. Retaining or cloning
lists keeps their slots reserved. Dropping unsubmitted lists abandons them;
rejected submissions do not mark them accepted. Three retained preparations
exhaust the slots and return `InFlight` rather than overwriting pending draws.
Submitted slots become reusable after their submission completes; capacity
pressure waits only for a submitted slot, never a device-idle operation.
Do not remove the receipt commands when integrating custom GAL commands.

`release_targets(&mut gal)` invalidates cached passes. `release(&mut gal)`
releases all state and is idempotent. Both reject pending prepared commands
without changing their ownership. Submit/drop all copies first. Accepted uses
retire through GAL's existing dependency/submission lifetime system. Always
release state explicitly before dropping it; dropping a state without release
leaves its cached handles owned by GAL until GAL teardown. All calls and handles
must belong to the construction GAL, and renderer access must be exclusive.

## PNG atlases and animation

The optional `png` feature adds `Image::load_png`/`decode_png` with bounded decode
buffers, palette/transparency expansion, grayscale and 16-bit conversion to RGBA8.
It also provides `write_png` for tightly packed RGBA8 output. It does not require
the `gpu` feature. APNG is rejected. Insert the decoded image
into the caller-owned `Textures`, then use `SpriteAtlas::grid` for checked uniform
regions with optional margin/spacing. `AnimationClip` selects frame indices from
caller-provided elapsed milliseconds; neither helper owns a clock or GPU device.

`Canvas::sprite_flipped` supports horizontal/vertical flips;
`sprite_transformed` additionally supports transpose before flips. These change
quad coordinates and preserve batching, filtering, tint and camera transforms.
`Canvas::camera` lets adapters restore the previous camera after drawing. Existing
`Sprite` construction and `Canvas::sprite` behavior remain compatible.

See [quasar-tilemap](../quasar-tilemap/README.md) for map assets and the
[Tilequest example](../../examples/tilequest/README.md) for native/headless usage.

## Indexed color and pixel priority

`IndexedImage`, `Palette` and per-draw `PaletteBinding` provide nearest-index
lookup, transparent indices, reusable palette banks and deterministic cycling.
Ordinary RGBA draws retain their existing behavior. Animate only a small palette
using the existing `AnimationClip` or `DynamicTexture` APIs.

`PixelState` compares/replaces eight mask bits using alpha or raw-index coverage.
`Priority2D`, `Canvas::background_sprite` and `Canvas::priority_sprite` reproduce
background occupancy and first-sprite priority, including hidden sprites that
block later sprites. CPU reference and GPU stencil implementations share rules.
Masks reset per canvas/prepare and require no caller-owned depth attachment.

The optional `assets` feature adds a bounded `PngLoader` for `quasar-assets`.
Loaded image clones share pixels; existing GAL views still use `register_texture`.
See [the full API, ownership and validation guide](../../docs/retro-rendering.md).

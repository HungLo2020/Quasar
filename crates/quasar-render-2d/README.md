# quasar-render-2d

An ordered 2D renderer that consumes VulkanicGAL's public API. It imports no
native graphics APIs or GAL backend implementations. VulkanicGAL owns driver
abstraction, shaders, resources, synchronization, and surfaces.

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
`present(&canvas, &textures)`. Forward physical size changes to `resize`.
Zero-sized surfaces defer presentation. Fit, integer scaling, aspect overrides,
and nearest/linear texture sampling share viewport math with `quasar-present`.
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
quad/batch counts and upload bytes. The bitmap font
covers uppercase ASCII, digits, and simple punctuation; custom fonts can use
sprite atlases. No scene graph, asset decoder, or font shaping is included.

The default `gpu` feature currently requires explicit-binding GLSL conventions
and uses Vulkan. Three bounded transient slots track prepared command references
and GPU completion. Native presentation retains synchronous retirement by default;
`set_frame_overlap(true)` opts into completion polling. Readback explicitly waits.
`from_gal` accepts an owned compatible GAL; the GAL currently permits one live instance per
process. Only Linux Vulkan has been verified.

With `default-features = false`, drawing and `reference::rasterize` compile
without windowing or Vulkan. This CPU reference path supports headless snapshots
and conformance tests. Native presentation uses the GPU quad shaders.
`render_to_image` uses those same shaders and reads real GPU pixels back as
logical-size RGBA8, rather than capturing OS window chrome or letterboxing.

Run the driver-dependent checks explicitly:

```sh
cargo test -p quasar-render-2d -- --include-ignored
cargo test -p quasar-render-2d --no-default-features
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
`quasar-frame` path. This crate does not depend on `quasar-frame`.

`register_texture(&mut gal, &mut textures, source_view)` validates a same-GAL,
sampled single-mip 2D RGBA8/BGRA8 view and makes a private view of the existing
image. It returns a normal `TextureId`; no image copy or CPU pixels are required.
The source owner controls texture contents and orders writes before sampling.
Publish a new ID when replacing a generation, then remove the old ID. The CPU
reference renderer returns an error for GPU-only images.

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

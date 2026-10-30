# quasar-render-2d

Ordered sprites, quads, bitmap/shaped text, indexed palettes and pixel masking.
GPU rendering uses native wgpu types through VulkanicGAL. CPU drawing and
`reference::rasterize` remain available with `default-features = false`, without
windowing, wgpu or physics. Default GPU features are `gpu` and `native`; backend
features `vulkan`, `metal`, `dx12`, `gles` forward upstream support.

```rust
use quasar_render_2d::{Canvas, Rect, Textures, Renderer2D};
let textures = Textures::default();
let mut canvas = Canvas::new(640, 400)?;
canvas.rect(Rect::new(20., 30., 80., 40.), [92, 230, 204, 255]);
let mut renderer = Renderer2D::headless()?;
let rgba = renderer.render_to_image(&canvas, &textures)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Renderer2D::for_window` accepts an owned window Arc, physical size and display
options. `present` uses Quasar's optional native surface helper. Forward resize
events; zero size defers. Fit/integer scaling, aspect correction, letterboxing,
nearest/linear sampling and mouse-to-canvas mapping share `Viewport` math.
`Renderer2D::from_gpu` accepts a caller-created `quasar_frame::Gpu` owner.

For composition, `RenderState2D::new(&gpu)` owns renderer resources only.
`prepare(&gpu, &canvas, &textures, &target, options, LoadOp::Load)` returns a native
single-use `CommandBuffer` plus draw/upload statistics. Use `Clear` for the
canvas clear color. Order 3D, raw native passes and 2D buffers on the same queue;
wgpu derives transitions and lifetimes. The 2D renderer leaves caller depth
unchanged. [Composition](../../examples/composition/README.md) exercises this.

Insert immutable RGBA8 `Image`s in `Textures`, then draw their `TextureId` with
`Sprite`. Source rectangles use texture pixels, destinations world coordinates.
Transforms, camera and captured per-quad clips preserve draw order. Adjacent
compatible quads batch; straight alpha blending preserves ordinary RGBA drawing.
Images upload once, geometry/uniforms per preparation. `stats()` reports batches,
quads and uploaded bytes. Removing an image invalidates its ID. Native commands
retain resources after owner removal. IDs cannot alias another texture store.

`register_texture(&gpu, &mut textures, &view)` retains an existing sampled native
RGBA8/BGRA8 2D single-layer texture view, without reading files, making a second
device or uploading a copy. The caller owns write ordering. Publish a new ID for
a replacement texture. GPU-only images have dimensions but no CPU pixels, so the
reference renderer rejects them. Native validation checks device/type compatibility.

`create_dynamic_texture` returns a stable texture ID. Stage tightly packed RGBA8
with `update_dynamic_texture` or transfer a Vec with its `_owned` variant.
A drawing preparation consumes pending pixels into an immutable GPU generation;
unchanged preparations reuse that generation and upload zero bytes. Native
command buffers retain their own generations, including abandoned/reordered
preparations. Updating creates a new native texture; this favors correctness
under arbitrary submission order over persistent texture allocation. There is no
fixed three-frame exhaustion rule. `destroy_dynamic_texture` removes the ID;
encoded commands remain valid.

`FramebufferPresenter` instead owns a private native image and reuses its texture
and bindings. It encodes each software framebuffer upload before drawing in the
same immediately submitted frame. Geometry and viewport buffers also grow/reuse
native allocations; per-preparation staging copies preserve ordered snapshots.
Explicit readback waits. Standalone presentation defaults to waiting;
`set_frame_overlap(true)` enables polling.

`IndexedImage`, `Palette`, `PaletteBinding`, palette cycles and `PixelState`
implement reusable palette swaps/animation and stencil priority/masking. Palette
indices use nearest sampling; changing a palette does not reupload index images.
Both CPU and GPU apply transparent/excluded indices, alpha thresholds, read/write
masks and color-write suppression. See [retro rendering](../../docs/retro-rendering.md).

Optional `text` adds `GpuTextAtlas` and `quasar-text`; bitmap-only applications
compile no font dependencies. Unchanged glyph atlas pages upload zero bytes.
`release_targets` clears stencil caches; `release` clears renderer caches and
marks the state released. Drop the renderer to release pipeline/layout owners.
Prepared buffers remain valid under native resource ownership.

```sh
cargo test -p quasar-render-2d --no-default-features
WGPU_BACKEND=vulkan cargo test -p quasar-render-2d --features text -- --include-ignored --test-threads=1
```

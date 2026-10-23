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
Removing an image invalidates its ID; GPU copies are released on the next valid
frame. IDs cannot alias images in another store. Built-in white and font
textures cannot be removed.

Draw order is preserved, using straight alpha blending. Adjacent quads with the
same texture and filter become one instanced draw. Images are uploaded once and
cached; ordinary frames upload geometry and uniforms. Buffer growth can rebuild
the cache. `stats()` reports quad/batch counts and upload bytes. The bitmap font
covers uppercase ASCII, digits, and simple punctuation; custom fonts can use
sprite atlases. No scene graph, asset decoder, or font shaping is included.

The default `gpu` feature currently requires explicit-binding GLSL conventions
and uses Vulkan. One frame is retired before resources are reused. `from_gal`
accepts an owned compatible GAL; the GAL currently permits one live instance per
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

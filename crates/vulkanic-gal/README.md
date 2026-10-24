# VulkanicGAL

Independent graphics abstraction layer extracted from MattMC. It owns resources,
validates explicit command lists and hazards, submits GPU work, and manages
presentation. It does not depend on Quasar windowing, Pong, or an editor.

- `VulkanicGal::create`: create a headless Vulkan or OpenGL GAL, depending on
  the selected backend feature.
- `VulkanicGal::create_windowed_vulkan`: retain an owned native window and create
  its Vulkan surface. Accepts raw-window-handle traits; no winit dependency in
  the library. Win32/Metal/X11/Wayland surfaces use ash-window. Available with
  the `vulkan` feature.
- `create_native_windowed_vulkan`: unsafe legacy integer-handle constructor.
  Available with the `vulkan` feature.
- `create_borrowed_opengl`: use an existing current OpenGL host context.
  Available with the `opengl` feature. It does not create a window or context.
- `commands`, `resources`, `frame`, `handles`, `sync`, `error`, `metrics`: public
  backend-neutral API. `cargo doc -p vulkanic-gal --no-deps` builds its reference.
- `backends/`: private Vulkan/OpenGL implementations. Native API dependencies
  stay behind this boundary, checked by the crate's tests.

## Backend features

`vulkan` is the default feature:

```toml
vulkanic-gal = { path = "...", default-features = false, features = ["vulkan"] }
vulkanic-gal = { path = "...", default-features = false, features = ["opengl"] }
```

- `vulkan` enables `ash`, `ash-window` and `shaderc` (including its native
  build) and the Vulkan backend.
- `opengl` enables `glow`, `libloading`, `libc` and the OpenGL backend. That
  backend is deliberately limited to isolated Linux EGL/GLX contexts and a
  caller-owned borrowed host context; this crate does not create OpenGL windows
  or swap buffers.
- Building with neither feature is supported: the public API is still available,
  `create` reports `UnsupportedFeature` for both backends, and no native
  graphics dependency is compiled.
- `BackendChoice` names both backends even when one is not compiled; disabling
  the feature affects availability, not the public enum.
- The OpenGL backend was previously compiled unconditionally. It is now opt-in:
  enable `opengl` when calling `BackendChoice::OpenGl` or
  `create_borrowed_opengl`.

Use `quasar-render-2d` for a ready-made pixel framebuffer presenter or 2D
renderer, or build a renderer against the GAL directly. The default explicit
shader cache is disabled; set `VULKANIC_SPIRV_CACHE_DIR` to opt into
application-owned cache storage. Other inherited diagnostic environment
variables retain their upstream names.

This crate is distributed under the MIT license; see the repository
[root LICENSE](../../LICENSE) and [UPSTREAM.md](UPSTREAM.md) for provenance.

The initial import retains the backend's process-wide graphics lock, so use
one GAL at a time. GPU support and portability limits are described in the
[workspace README](../../README.md). See [provenance and adaptations](UPSTREAM.md).

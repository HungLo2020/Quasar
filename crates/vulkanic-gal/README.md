# VulkanicGAL

Independent graphics abstraction layer extracted from MattMC. It owns resources,
validates explicit command lists and hazards, submits GPU work, and manages
presentation. It does not depend on Quasar windowing, Pong, or an editor.

- `VulkanicGal::create`: create a headless Vulkan/OpenGL GAL.
- `VulkanicGal::create_windowed_vulkan`: retain an owned native window and create
  its Vulkan surface. Accepts raw-window-handle traits; no winit dependency in
  the library. Win32/Metal/X11/Wayland surfaces use ash-window.
- `create_native_windowed_vulkan`: unsafe legacy integer-handle constructor.
- `commands`, `resources`, `frame`, `handles`, `sync`, `error`, `metrics`: public
  backend-neutral API. `cargo doc -p vulkanic-gal --no-deps` builds its reference.
- `backends/`: private Vulkan/OpenGL implementations. Native API dependencies
  stay behind this boundary, checked by the crate's tests.

Use `quasar-present` for a ready-made pixel framebuffer presenter, or build a
renderer against the GAL directly. The default explicit shader cache is disabled;
set `VULKANIC_SPIRV_CACHE_DIR` to opt into application-owned cache storage.
Other inherited diagnostic environment variables retain their upstream names.

The initial import retains the backend's process-wide graphics lock, so use
one GAL at a time. GPU support and portability limits are described in the
[workspace README](../../README.md). See [provenance and adaptations](UPSTREAM.md).

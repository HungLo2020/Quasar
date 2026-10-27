# VulkanicGAL

Independent graphics abstraction layer extracted from MattMC. It owns resources,
validates explicit command lists and hazards, submits GPU work, and manages
presentation. It does not depend on Quasar windowing, Pong, or an editor.

- `VulkanicGal::create`: create a headless Vulkan, OpenGL or wgpu GAL,
  depending on the selected backend feature.
- `VulkanicGal::create_windowed`: retain an owned native window and create a
  presentation surface on Vulkan or wgpu. `create_windowed_vulkan` and
  `create_windowed_wgpu` are shorthands. Available with `vulkan` or `wgpu`.
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
- `backends/`: private Vulkan/OpenGL/wgpu implementations and the shared
  SPIR-V toolchain. Native API dependencies stay behind this boundary, checked
  by the crate's tests; the workspace's `quasar-workspace-checks` tests also
  forbid every other crate from depending on them.

## Backend features

`vulkan` is the default feature:

```toml
vulkanic-gal = { path = "...", default-features = false, features = ["vulkan"] }
vulkanic-gal = { path = "...", default-features = false, features = ["opengl"] }
vulkanic-gal = { path = "...", default-features = false, features = ["wgpu"] }
```

- `vulkan` enables `ash`, `ash-window` and `shaderc` (including its native
  build) and the Vulkan backend.
- `opengl` enables `glow`, `libloading`, `libc` and the OpenGL backend. That
  backend is deliberately limited to isolated Linux EGL/GLX contexts and a
  caller-owned borrowed host context; this crate does not create OpenGL windows
  or swap buffers.
- `wgpu` enables `wgpu`, `naga` and `shaderc` and the wgpu backend, which
  runs on Vulkan, Metal or Direct3D 12 (see below).
- Building with no backend feature is supported: the public API is still
  available, `create` reports `UnsupportedFeature` for every backend, and no
  native graphics dependency is compiled.
- `BackendChoice` names every backend even when one is not compiled; disabling
  a feature affects availability, not the public enum.
- `VULKANIC_BACKEND` (`vulkan`, `wgpu` or `opengl`) overrides the backend every
  `create*` constructor uses, including `create_windowed_vulkan` and
  `create_native_windowed_vulkan`, so an unmodified application can run on
  another compiled-in backend.
- The OpenGL backend was previously compiled unconditionally. It is now opt-in:
  enable `opengl` when calling `BackendChoice::OpenGl` or
  `create_borrowed_opengl`.

## Choosing and reporting the backend

Games that do not care call the existing constructors or pass
`GalOptions::default()` (`GraphicsBackend::Auto`): native Vulkan first, then
wgpu, then (headless only) OpenGL, among the backends compiled in. Games that
let players choose store a `GraphicsBackend` id in their settings and pass it
through `GalOptions`:

```rust
use vulkanic_gal::{GalOptions, GraphicsBackend, VulkanicGal};

// Settings menu: offer what this build and platform contain.
for backend in GraphicsBackend::selectable(true) {
    println!("{} ({})", backend.display_name(), backend.id()); // "wgpu / Vulkan (wgpu-vulkan)"
}

// Startup: the saved id, falling back to automatic selection if it fails.
let backend: GraphicsBackend = settings.graphics_backend.parse().unwrap_or_default();
let gal = VulkanicGal::create_windowed_with(
    GalOptions::new(backend).with_fallback(),
    "game",
    window,
    surface,
)?;

// Debug screen: "wgpu / Vulkan | NVIDIA GeForce RTX 2070 | NVIDIA 580.178.04 | Vulkan through wgpu"
let info = gal.backend_info();
println!("{info}");
```

- `GraphicsBackend` ids: `auto`, `vulkan`, `opengl`, `wgpu`, `wgpu-vulkan`,
  `wgpu-metal`, `wgpu-dx12`. `is_compiled` and `selectable` account for both
  Cargo features and the target platform (Metal on Apple, Direct3D 12 on
  Windows).
- An explicit choice is authoritative: without `with_fallback` a backend that
  cannot start returns its error, and `VULKANIC_BACKEND` replaces it only when
  `allow_env_override` is set. The override always applies to `Auto` and to the
  `BackendChoice` constructors, so unmodified programs can still be switched
  for testing.
- `backend_info()` returns the requested and selected backend (wgpu's native
  API included), any override or fallback (`notes`, with the failure reason),
  and the device name, driver, API version and device kind. `Renderer2D`,
  `FramebufferPresenter` and every `VulkanicGal` expose it.
- `quasar-render-2d`'s `Renderer2D::for_window_with` and
  `headless_with_options` take `GalOptions`; `for_window` and `headless` use
  `Auto`, so they now fall back to wgpu when native Vulkan cannot start.
  `quasar-render-3d` renders through whatever GAL the game created.

## The wgpu backend

The wgpu backend exists for portability: Metal on macOS without MoltenVK,
Direct3D 12 on Windows, and any Vulkan driver wgpu supports. Where Vulkan is
available, the native Vulkan backend stays the default and the faster choice:
wgpu validates and tracks resource state again behind the GAL's own
validation, so CPU submission cost is higher. Compare with the Starfall
profile (`docs/profiling.md`) on the same machine before switching a game.

It reports the Vulkan backend's `ShaderConventions` (explicit-binding GLSL,
zero-to-one depth, top-left origin), so renderers need no backend branches:

- GLSL and SPIR-V are compiled by the shared Shaderc toolchain, then SPIRV-Tools
  inlines calls and splits combined image samplers, and naga reads the result.
  GAL binding `b` becomes wgpu binding `2b`; the sampler of a combined sampler
  becomes `2b + 1`.
- `RasterYDirection::Up` is wgpu's native clip space; `Down` pipelines get a
  vertex-module variant with Y negated by naga and the declared front face
  swapped, exactly as the Vulkan backend swaps it with its viewport.
- Bind group layouts come from shader reflection. Float textures bound in an
  unfilterable format (depth sampled as a value, `R32Float` without
  `FLOAT32_FILTERABLE`) select a pipeline variant with unfilterable layouts.
- Barriers are validated by the GAL and then left to wgpu's own tracking.
- Host writes keep their position in the command stream (staged and copied at
  the op). Unaligned host writes, buffer copies and tightly packed texture
  rows go through an internal byte-copy compute pass, so frontends keep
  passing any offset and any `bytes_per_row`.
- Triangle fans draw generated indices. `ProvokingVertex::Last` with flat
  shader outputs rotates each primitive (generated indices for direct draws, a
  rotated copy of the index buffer for indexed and indexed-indirect draws).
- `TextureRowOrder::Reverse` copies row by row; `GenerateMipmaps` renders each
  level with a linear blit.

Limits and explicit errors: no descriptor arrays, one dynamic offset per
binding, no 3D depth/stencil textures or 3D mip generation, `Rgb16Float` is
stored as `Rgba16Float` and cannot be copied to or from buffers, integer
textures are read with `texelFetch` (naga rejects filtered sampling of integer
data), and non-indexed indirect draws cannot use triangle fans or last-vertex
flat shading. Frame copies need a surface that allows copy usage. RenderDoc and
Tracy integration and GPU timestamps are Vulkan-only. wgpu's own environment
variables apply: `WGPU_BACKEND`, `WGPU_ADAPTER_NAME`, `WGPU_POWER_PREF`.

Its GPU tests (`backends/wgpu/tests.rs`) run every program on wgpu and on
Vulkan when available and require identical pixels and bytes; they report an
environment gap without an adapter unless `VULKANIC_REQUIRE_GPU=1`.

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

# Native wgpu migration validation

This migration replaces VulkanicGAL's private graphics implementation with two
re-exports of external crates.io wgpu 30.0.1. Upstream source is unmodified.
The comparison baseline is Quasar commit
`7a82ff3f23f0c6720dd1bcd0864be18ae8608b41` (0.2.19), rebuilt from an archive of
that exact revision. Migration measurements were recorded at workspace version
0.2.19 before the release bump to 0.2.20. The original local validation preceded
the release workflow. No other repository was changed.

## Scope

All built-in graphics consumers use native resources, descriptors, WGSL, command
encoders and single-use command buffers through `vulkanic_gal`. This includes
2D/3D, indexed palettes/stencil masks, imported models, lighting, normal maps,
directional/spot shadows, text/UI, frame presentation/readback, profiling and the
nine graphics examples. The CPU simulation/asset/physics interfaces remain
independently usable. See [migration contracts](native-wgpu-migration.md).

Destination buffers and native bindings are cached. Per-preparation immutable
staging copies precede draws, preserving arbitrary pending/reordered snapshots.
Dynamic image generations remain independent. The private software framebuffer
presenter reuses one native image with an ordered upload. Nightwatch reuses an
explicit shadow atlas with whole producer/receiver frames submitted together;
ordinary shadow `prepare` still creates independent generations.

## Validation environment

Linux x86_64, NVIDIA GeForce RTX 2070, driver 580.178.04. GPU pixel tests use
upstream Vulkan. Native window tests use X11/Xvfb with driver access. Ordinary
headless simulations create neither a window nor a GPU; headless screenshots
exercise the same native renderers using offscreen attachments.

Native tests cover:

- Identical public upstream type identity and dependency/source guards keeping
  direct graphics imports confined to VulkanicGAL.
- Native device coexistence, adoption of caller-created device objects, aligned
  readback/BGRA conversion, validation errors and asynchronous timestamp results.
- RGBA/indexed/palette/masking CPU/GPU pixel parity, external views, atlas/text
  updates, dynamic snapshots, removed owners and reordered frames.
- Scalar/instanced/culling parity, lighting, normal maps, shadow filters,
  independent shadow generations, reused whole-frame shadow targets, growth,
  sampler changes, model replacement and composition with raw native commands.
- Live example resize/fullscreen/DPI, deferred zero-size presentation and recovery,
  input/replay, deliberate error paths and native ownership cleanup.

## Commands and results

The final workspace test command completed **530 tests, zero failures, zero
ignored tests**, across 121 test binaries (including empty binaries):

```sh
WGPU_BACKEND=vulkan cargo test --locked --workspace --all-features --all-targets \
  --no-fail-fast -- --include-ignored --test-threads=1
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

`tools/full_checks.py --keep-going` also completed **all 85 steps with no
failures or skipped steps**. It covers backend/CPU feature combinations, strict
Clippy permutations, ordinary tests/doctests, documentation with warnings denied,
all built-in graphics examples, X11 resize/fullscreen/forced-DPI exercises,
headless replay/screenshots, shared-device GPU fixtures and the GLES capability
fixture. Nightwatch's `--no-default-features --features graphics` offscreen path
also runs without its desktop feature. Xvfb was added to PATH, Wayland was unset
for forced-X11 exercises, and `WGPU_BACKEND=vulkan` selected the rendering backend.

Strict workspace Clippy and formatting pass. The same-device invalid-view test
uses two devices within one native instance; independent instances are tested
with valid, independent resources. No invalid cross-instance panic is treated as
an acceptable rendering result.

For release 0.2.20 the gates were repeated after the version bump: the complete
native suite again passed 530 tests with none ignored, all 85 matrix steps passed
without skips, and default/all-feature strict Clippy passed. `verify_pong.py`
confirmed matching 12,000-tick solo/local replay states and PNGs;
`verify_tilemap.py` passed CLI interoperability, replay and relocation checks.

## Interactive Nightwatch

The final optimized desktop build passed an X11 input/screenshot harness exercising
mouse firing, rifle selection, reload, jump, pause, H/N shadow/normal toggles,
resize, F5 model reload, deliberate corruption of a temporary copied model,
failed-reload preservation and recovery. Restoring effects/assets while paused
restores pixels exactly. The deliberate draw-budget failure writes its benchmark
artifact and exits cleanly. Shutdown reports `live_buffers=0`,
`live_textures=0`, `live_views=0` using native owned-resource reports. Original
bundled assets were not mutated. Local artifacts are in
`/tmp/nightwatch-collision-native-gzjhkvtd`.

## Exact starting-revision pixel comparison

The final optimized Nightwatch binary and the exactly rebuilt starting revision
render identical 640×360 RGBA screenshots at seed 42 for all six cases below:

| Autoplay tick | Render options | Differing channels | Maximum channel difference |
|---|---|---:|---:|
| 1 | Default | 0 | 0 |
| 30 | Default | 0 | 0 |
| 120 | Default | 0 | 0 |
| 30 | Shadows disabled | 0 | 0 |
| 30 | Normal maps disabled | 0 | 0 |
| 30 | Instancing and culling disabled | 0 | 0 |

Simulation hashes also agree for every pair. The
[pixel summary](benchmarks/2026-10-30-native-wgpu/pixel-parity.json) and
[comparison script](benchmarks/2026-10-30-native-wgpu/compare_pixels.py) preserve the
cases and checks. This compares against the former engine, in addition to the
new CPU/GPU and scalar/instanced regression suites.

## Performance

See the [paired comparison](benchmarks/2026-10-30-native-wgpu/README.md),
[summary](benchmarks/2026-10-30-native-wgpu/summary.json), reproduction script and
30 raw reports. Three alternating pairs compare exactly rebuilt baseline and
native binaries. Nightwatch CPU frame p50 changed from 0.810 to 0.713 ms (about
12% lower), with identical gameplay hashes in every run. Prepare-plus-submit mean
increased from 0.442 to 0.583 ms. Stage boundaries differ between implementations;
there is no claim that every stage improved.

Composition/framebuffer presentation timings vary significantly on this desktop.
Polling p50 changed from 8.856 to 9.117 ms for composition and 9.853 to 10.251 ms
for framebuffer. These results do not establish a universal display speedup.
Rust allocation counts increased: 77.23 to 270.57/frame for composition and
55.31 to 227.75/frame for framebuffer. Framebuffer allocated bytes dropped from
about 1.035 MB to 31.8 KB/frame by avoiding the old full CPU-frame Vec copy.
The raw distributions and timing-scope limits matter when assessing these costs.

## Platform and measurement limits

Metal and DX12 feature combinations compile on this Linux host; their native
GPU execution needs the corresponding OS and hardware. Web targets were not built
or executed; backend feature forwarding alone does not establish renderer
portability, and the setup/readback helpers target native execution. Release CI
includes Windows/macOS graphics-consumer compilation checks; those checks do not
establish native GPU pixel parity on either platform.

GLES is an upstream capability limitation on this driver: its selected GLSL 3.3
target cannot compile the renderers' storage buffers/dynamic arrays. The dedicated
capability fixture verifies a clean scoped error. Disabling upstream indirect
validation with `WGPU_VALIDATION_INDIRECT_CALL=0` is limited to that diagnostic:
it gets past an earlier unsupported upstream compute validation shader, but does
not make the renderer's storage-buffer shaders supported. No upstream workaround
or second renderer/backend was added. GLES feature compilation is not a claim of
GLES pixel parity here.

GPU timestamps bracket submitted command buffers, including explicitly encoded
staging copies. Implicit `Queue::write_*` transfers precede this interval; asset
and immutable dynamic-image uploads can therefore be excluded. Old/new GPU
numbers do not represent identical total transfer intervals. Unsupported timing
and private backend details remain absent, rather than fabricated zero samples.
Native core validation remains upstream behavior even when driver debug layers
are disabled.

Native command buffers retain resources until submission/drop/completion. Raw
prepared frames have no artificial fixed slot ceiling, so applications must bound
outstanding frames to bound memory. Dynamic-image updates allocate independent
native textures. Whole-process Rust allocation counts include upstream and
profiling overhead and are not equivalent to GPU resource allocations. Native
error scopes cover validation/internal/allocation errors; cross-instance resource
misuse can still trigger upstream assertions and is outside the same-device
composition contract.

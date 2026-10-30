# One GAL: 3D cube and 2D HUD

This ordinary Cargo example uses `quasar-render-3d` and `quasar-render-2d` on one
caller-owned VulkanicGAL device and acquired frame target. The cube and HUD use
views of the same GPU checker texture; there is no framebuffer readback/upload
between them. The example intentionally owns its raw window and event loop instead of using
`quasar-app`, so the composition/acquire/present path is exercised directly and
serves as the lower-level escape hatch for custom event loops.

```sh
cargo run -p quasar-composition
cargo run -p quasar-composition -- --headless --screenshot /tmp/composition.png
cargo run -p quasar-composition -- --frames 40 --exercise-window
cargo run -p quasar-composition -- --explicit --sync --frames 40 --exercise-window
```

Escape quits; left/right arrows adjust cube rotation. `--headless` renders an
offscreen GPU snapshot and requires Vulkan, but no display. Native screenshots
also use logical-size GPU readback. CPU-only game simulation remains available
in Pong with `--no-default-features`; this 3D example is explicitly GPU-dependent.

## Composition and ownership

`Demo::commands` in [src/lib.rs](src/lib.rs) is the rendering composition:

1. Borrow the application's GAL to prepare indexed 3D mesh commands with depth.
2. Prepare HUD commands on the same target with `LoadOp::Load`, preserving world
   color. The HUD does not attach or alter world depth.
3. Order the command lists as world, offscreen attachment dependency where
   applicable, then HUD. Acquired-image transitions belong to GAL.
4. Submit one combined frame batch and present once.

Native encoder buffers are ordered world, application passes, then HUD and
submitted on one native queue. The `--explicit` path spells out acquire, prepare
and present; the ordinary path calls `FrameSurface::render`. Both use identical
renderers and native commands. `submit_explicit` demonstrates direct queue use
for offscreen composition. No renderer requires the surface helper or a runner.

The application owns the native device/surface and source texture. Each renderer
owns pipelines and caches; the 2D renderer retains the source view without a copy.
Native command buffers retain referenced resources and are consumed on submit.
Dropping prepared buffers abandons work; dropping mesh/renderer owners leaves
already encoded work valid. No global device lock or artificial slot exhaustion
exists. See [migration](../../docs/native-wgpu-migration.md).

The ignored GPU suite covers shared-texture color pixels, depth ordering, dynamic
framebuffer composition, reverse submission after owner Drop, error/panic
recovery, invalid 3D view registration, preloaded tilemaps and stencil overlays.
Native exercises cover resize, fullscreen, zero-size deferral, intentional
preparation failure and subsequent recovery. Native ownership reports must show
zero live user-owned buffers/textures/views after teardown.

```sh
WGPU_BACKEND=vulkan cargo test -p quasar-composition --lib -- --include-ignored --test-threads=1
cargo run -p quasar-composition -- --frames 40 --exercise-window
```

## Measuring synchronous versus overlapping frames

```sh
python3 tools/benchmark_composition.py --frames 600 --pairs 3 --output /tmp/composition-benchmark
```

Benchmark runs use the same native scene and distinguish synchronous waiting
from completion polling. Reports contain CPU submit/acquire/present/frame
percentiles, optional native GPU timestamps, actual requested waits, Rust
allocation traffic and optional native allocator reports. Unsupported counters
are absent. CPU completion observation is not input-to-photon latency.

Current runs should be compared on the same adapter/driver and build profile.
Historical measurements below describe the old graphics engine and are retained
as evidence rather than a current API or performance guarantee.

## Recorded baseline: 2026-10-23

NVIDIA RTX 2070, driver 580.178.04, Linux/Wayland, Rust 1.94.0. Three alternating
pairs, 600 measured frames per run after 64 warmup frames, release, validation
off, GPU timestamps on, no requested vsync. Values are medians across the three
run summaries; see the [summary](benchmarks/2026-10-23-rtx2070/summary.json) and
the six adjacent raw reports for distributions and GPU sample counts. Each run
records exactly one GAL creation and balances all resources at teardown.

| Metric | Synchronous | Completion polling |
|---|---:|---:|
| CPU frame p50 (ms) | 0.1221 | 0.1248 |
| CPU frame mean (ms) | 0.1860 | 0.2137 |
| GPU frame p50 (ms) | 0.0283 | 0.0281 |
| Engine frame to observed completion p50 (ms) | 0.1219 | 0.2492 |
| Blocking timeline waits / 600 frames | 588 | 1 |
| Device-idle waits | 0 | 0 |
| GAL resource creates / destroys | 0 / 0 | 0 / 0 |
| Rust allocations / 600 frames | 63066 | 62898 |
| Host-upload bytes / 600 frames | 1795200 | 1795200 |

On this small workload polling removed almost all blocking timeline waits, but
did not yield a consistent CPU improvement. The final paired runs show slightly
higher CPU p50 and substantially higher observed completion latency; earlier
iterations varied with desktop load. GPU time and upload volume stayed similar.
There is no claim of a universal speedup or lower display latency. The existing
standalone 2D API retains synchronous retirement by default and exposes
`set_frame_overlap(true)` for opt-in overlap. The composition demo offers both
policies for larger or latency-sensitive workloads.

The one polling wait includes final draining/backpressure. Zero steady-state GAL
resource creates/destroys is not zero Rust allocation: command construction and
GAL still allocate, with roughly 105 whole-process allocations per measured
frame including instrumentation. The benchmark does not reserve the machine or
GPU, and desktop/OS activity affects timing tails. Physical display latency and
MattMC gameplay parity were not measured.

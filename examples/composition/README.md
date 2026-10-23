# One GAL: 3D cube and 2D HUD

This ordinary Cargo example uses `quasar-render-3d` and `quasar-render-2d` on one
caller-owned VulkanicGAL device and acquired frame target. The cube and HUD use
views of the same GPU checker texture; there is no framebuffer readback/upload
between them. The example owns its window and event loop.

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

The explicit `submit_explicit` implementation opens/closes GAL recording scopes
and submits directly. It was implemented and tested before extracting
`quasar-frame::record_and_submit`. `--explicit` retains that direct submission
path; both native modes use the extracted surface-target cache. `RenderState2D`
and `MeshRenderer3D` do not require the surface helper, and clients can
coordinate raw GAL acquire/present without it; the windowed `Renderer2D` and
`FramebufferPresenter` use it. There is no renderer registry, ECS, render graph
or plugin system.

The application owns GAL, its surface and source texture. Each renderer owns
its pipelines, descriptors and transient storage. The 2D renderer creates a
private view of the shared image instead of copying it. Targets, textures and
renderers must all belong to the same GAL. Release renderer/pass/binding caches
before releasing their owners, and use GAL retirement for submitted work.

Three bounded transient slots use `SubmissionUsage` receipts to distinguish
unsubmitted commands from incomplete GPU work. Cloning a command list retains
its reservation; dropping all unsubmitted copies abandons it. A renderer refuses
cleanup while prepared copies remain. Only completed ranges are reused. Bounded
pressure waits for an eligible submission, without ordinary device-idle waits.

## Verification

```sh
cargo test --workspace --locked -- --include-ignored
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p quasar-composition -- --frames 40 --exercise-window
```

GPU tests are explicitly ignored by default. The composition suite verifies:

- The cube renders, HUD color loads preserve it, and the shared texture samples
  correctly without a CPU copy.
- A farther triangle drawn last cannot overwrite a nearer triangle.
- Pending and cloned commands reserve resources even when unrelated uploads
  complete; three retained preparations return bounded `InFlight` pressure.
- Frames prepared together retain their exact pixels when submitted out of order.
- Abandonment, rejected submissions and preparation errors allow later valid use.
- The extracted recording scope closes during panic unwinding.
- Repeated target sizes and outstanding submissions retire all resource handles.

The native exercise resizes, enters/exits fullscreen, defers/restores a zero-size
surface, intentionally rejects a frame, cancels its acquired image, and continues
presenting. Final resource counters must balance. Linux/Wayland with NVIDIA
Vulkan was exercised locally. CI is configured to use Mesa/software Vulkan and Xvfb; other native
platforms are not established by these tests.

## Measuring synchronous versus overlapping frames

```sh
python3 tools/benchmark_composition.py --frames 600 --pairs 3 --output /tmp/composition-benchmark
```

The driver builds release, disables GAL validation for timing, requests GPU
queries, uses 64 warmup frames, requests no vsync, and alternates synchronous and
completion-polling runs. Correctness tests run separately with validation on.
`--sync` keeps the original per-frame waiting policy. Both benchmark paths use
the same scene, shaders, caches and slot allocator, isolating the scheduling
change. This is not a comparison against an archived v0.2.0 binary or MattMC.

Reports include CPU frame/submit/acquire/present distributions, GPU timing sample
counts, waits, GAL creates/destroys, whole-process Rust allocation counts/bytes,
upload traffic, submission-completion latency, and engine-frame completion
latency. Engine-frame latency runs from acquire entry through host-observed GPU
completion. These observations include polling cadence and exclude input event
dispatch, compositor and physical display latency. Physical input-to-photon
latency needs separate instrumentation.

Polling can trade latency for CPU/GPU overlap. Keep synchronous mode available
for latency-sensitive clients and compare appropriate workloads before selecting
policy. A tiny cube does not establish gameplay performance for a large world.

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

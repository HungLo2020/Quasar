# Graphics profiling

The measurement foundation of issue #18 is implemented by the independently
usable `quasar-profile` crate and the shared GAL/frame measurements exposed by
VulkanicGAL and `quasar-frame`. Live benchmark integration with configured
regression thresholds remains outstanding. Profiling does not require a custom
build system or editor.

## Pipeline

1. VulkanicGAL's `SubmitProfile` records validation, command generation,
   backend encode/submit, native command-buffer work, hazards, GPU scope
   timestamps, uploads, resource deltas, waits and swapchain state.
2. `quasar-frame::FrameReport` adds acquire/present/CPU frame time around one
   surface transaction. `OffscreenTarget` provides the same command/readback
   path for headless screenshots and renderer tests.
3. `quasar-profile::FrameSample` adds renderer draws/upload counters and maps a
   `FrameReport` into neutral metrics.
4. `FrameProfiler` retains a bounded window and computes min/mean/p50/p95/p99/max
   for CPU/GPU frame time, acquire, present, submit, draws, uploads, resource
   churn and synchronization waits. Optional GPU scopes are preserved as
   distributions.
5. `ProfileThresholds::check` checks only explicitly supplied p99 limits.
   `write_json` writes a machine-readable `ProfileSummary`.

## Integration benchmark

`examples/starfall` is the representative game integration benchmark. It runs
one GAL, one `FrameSurface`, 3D world rendering and a 2D HUD with GPU scope tags
for `world_3d` and `hud_2d`:

```sh
cargo run -p starfall -- --benchmark --benchmark-frames 120 \
  --benchmark-file /tmp/starfall-profile.json
```

The artifact includes frame CPU/GPU distributions, per-scope GPU time, draws,
geometry/texture/uniform uploads, host writes, resource creates/destroys,
blocking waits and device-idle waits. The existing composition fixture remains
available for detailed device/backends comparisons:

```sh
cargo run -p quasar-composition -- --frames 200 --benchmark /tmp/composition.json
```

Existing composition benchmark artifacts under
`examples/composition/benchmarks/` remain historical fixtures; run them on the
same device/driver and compare distributions, not absolute numbers across hosts.

## Correctness and regression coverage

- `FrameProfiler`, percentile, ring-window, optional GPU, threshold and JSON tests.
- `quasar-frame::OffscreenTarget` render/readback is exercised by Starfall's
  ignored Vulkan render test (`.png` headless screenshot is also supported).
- Starfall tests assert non-uniform 3D+HUD pixels, >10 draws, exact replay and
  deterministic state hashes.
- The current GitHub workflow runs formatting and the ordinary workspace tests
  on Linux, plus selected compile checks on Windows/macOS. It does not run the
  ignored GPU tests or native rendering/benchmark matrix. Those checks are
  available locally through `tools/full_checks.py` or individual test commands.
- `ProfileThresholds::check` is opt-in and currently exercised only by unit
  tests. Live benchmarks emit measurements without applying those limits. The
  stored composition fixture test checks metric shape and statistical
  consistency, not a new run's performance against the historical artifact.
- CI does not claim GPU performance from compile-only runs or compare
  thresholds across different GPUs/drivers.

## Sample measurements

Starfall release, RTX 2070/Linux X11, 1280x720, 120 frames:

| Metric | p50 | p95 | p99 |
|---|---:|---:|---:|
| CPU frame | 14.38 ms | 15.90 ms | 16.81 ms |
| GPU frame | 65.5 µs | 68.1 µs | 75.8 µs |
| CPU submit | 91.7 µs | 131.6 µs | 155.5 µs |
| Acquire | 4.4 µs | 6.9 µs | 18.5 µs |
| Present/retire | 14.22 ms | 15.73 ms | 16.67 ms |
| 3D GPU scope | 40.96 µs | — | 45.66 µs |
| HUD GPU scope | 8.58 µs | — | 10.27 µs |
| Draws | 145 | 145 | 145 |
| Host writes | 18,368 B | 18,368 B | 18,368 B |

This Xephyr run was presentation-bound at approximately the 60 Hz display
cadence; CPU submission and GPU work remained sub-millisecond. A native
non-vsync surface can produce much lower CPU-frame numbers. Report both the
submit/GPU distributions and the presentation policy rather than treating the
CPU frame as a GPU benchmark.

Single-host observations. Acquire/present variation reflects swapchain and
compositor behavior, not simulation time.

## Implemented coverage

- Independent profiling crate, no mandatory runner/renderer/GPU ownership.
- CPU frame, GPU frame when supported, scope, upload, resource, draw and
  synchronization measurements.
- Bounded sampling with p50/p95/p99 and machine-readable artifacts.
- Optional threshold checks with explicit violations.
- Correct sample mapping from `FrameReport`; no fake GPU numbers when timestamps
  are unavailable.
- Headless offscreen correctness/readback coverage.
- Representative Starfall integration benchmark plus existing composition
  fixture compatibility.
- Documented hardware limits, unsupported capabilities and non-portable claims.

## Limitations

- GPU timestamps depend on Vulkan/backend support and timestamp validity; zero
  scope samples mean unavailable, not zero GPU cost.
- CPU/GPU event timing is not input-to-photon or display latency.
- No automatic cross-machine threshold calibration; thresholds must be tuned per
  device class and run in a stable performance environment.
- Device memory allocation bytes are not exposed by every backend; the profiler
  reports resource counts and upload bytes rather than fabricating allocation
  byte totals.
- The composition benchmark's historical files are not silently rewritten.

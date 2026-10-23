# Software-framebuffer presentation benchmarks

These are single-run release measurements from migrating the legacy framebuffer
presenter into `quasar-render-2d`. They are observations on one desktop, not
performance thresholds.

- Date: 2026-10-23
- Host: Linux x86-64, Intel Core i7-10750H, NVIDIA RTX 2070 (580.178.04)
- Build: `--release`, Vulkan GPU timestamps enabled
- Workload: 640x400 RGBA8 framebuffer into a 960x600 window, 64 warmup frames
  and 299 measured frames, vsync on
- Method: the legacy harness used the removed crate; the new harness used
  `FramebufferPresenter` with synchronous retirement or completion polling. Both
  regenerated the same pixels per frame and measured only the `present` call.

| Metric | Legacy presenter | New synchronous | New completion-poll |
|---|---:|---:|---:|
| CPU present p50 (ms) | 6.535 | 6.444 | 6.544 |
| CPU present p95 (ms) | 7.201 | 7.065 | 7.041 |
| GPU frame p50 (ms) | 0.2509 | 0.2589 | 0.2584 |
| Submission-to-observed-completion p50 (ms) | not instrumented | 0.547 | 6.961 |
| Texture upload bytes / run | 306,176,000 | 306,176,000 | 306,176,000 |
| Blocking timeline waits | 296 | 298 | 0 |
| Device-idle waits | 0 | 0 | 0 |
| GAL resource creates / destroys | 1,196 / 1,196 | 0 / 0 | 0 / 0 |
| Rust allocations / run | 23,981 | 20,710 | 20,710 |
| Rust allocated bytes / run | 309,554,944 | 310,812,719 | 310,816,531 |

## Interpretation

- Presentation cost is dominated by vsync; all three paths land around 6.4-6.5 ms
  CPU p50.
- The replacement removes all steady-state GAL resource churn. The legacy
  presenter recreated its frame target and render pass every frame; the new
  presenter caches the surface target and pass, and reuses one persistent
  texture, one view, samplers, bindings, and three upload slots.
- GPU frame time is effectively unchanged within run-to-run variance
  (new synchronous is about 3% higher p50 in this single run; polling is within
  0.3% of synchronous).
- Rust allocation count drops by about 14%. Bytes are about 0.4% higher because
  the new path stages one framebuffer snapshot per prepared command list so
  out-of-order submissions keep their own pixels.
- Completion polling removes blocking timeline waits entirely. It trades that
  for a latency observation window that includes poll cadence, so its measured
  submission-to-completion p50 is much larger even though GPU work is unchanged.
- Legacy completion latency was not instrumented because it waited for every
  submission synchronously; its CPU present time is the blocking proxy.

No regression was found that would justify keeping the old crate. The
replacement is retained without a compatibility wrapper, and the old presenter
remains available in published Git tags.

Reproduce the new columns with:

```sh
cargo run --release -p framebuffer -- --frames 364 --benchmark /tmp/framebuffer-sync.json
cargo run --release -p framebuffer -- --frames 364 --poll --benchmark /tmp/framebuffer-poll.json
```

The legacy harness was temporary and is not part of the repository.

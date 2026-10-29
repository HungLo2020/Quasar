# quasar-profile

Optional bounded graphics measurements and performance-regression summaries. It
does not create a GAL, renderer, window or global profiler.

```rust
use quasar_profile::{FrameProfiler, FrameSample};

let mut profiler = FrameProfiler::new(240);
profiler.record(FrameSample {
    frame: 1,
    cpu_frame_nanos: 2_000_000,
    gpu_frame_nanos: Some(1_500_000),
    ..FrameSample::default()
});
let summary = profiler.summary();
assert_eq!(summary.cpu_frame.p99, 2_000_000);
```

With the default `frame` feature, `FrameSample::from_frame_report` maps
`quasar_frame::FrameReport`/VulkanicGAL `SubmitProfile` fields. Add renderer
draw/upload counters with `with_renderer_stats`.

- `FrameProfiler`: bounded ring, total frame count, min/mean/p50/p95/p99/max.
- `ProfileThresholds`: opt-in p99 limits for CPU/GPU frame, acquire, present,
  draws, resource creates and host writes.
- `ProfileSummary`: machine-readable JSON with optional GPU scope distributions.
- **Features:** `json` (default), `frame` (default). The core sample/summary
  types work without either feature.

```sh
cargo test -p quasar-profile
cargo test -p quasar-profile --no-default-features
cargo clippy -p quasar-profile --all-targets -- -D warnings
cargo run -p starfall -- --benchmark --benchmark-frames 120 \
  --benchmark-file /tmp/starfall-profile.json
```

GPU numbers are reported only when the backend provides valid timestamps.
Thresholds and measurements are single-machine observations, not portable
performance guarantees. See [`docs/profiling.md`](../../docs/profiling.md).

## Live benchmark sessions

`BenchmarkSession` adds configurable warmup, bounded sample retention, live
`ProfileThresholds` checks and bounded caller-provided metadata. Empty runs and
missing GPU measurements fail explicit budgets. Optional `config` and
`diagnostics` features provide prefix-based Quasar Config adapters and structured
completion/failure records without installing global state. Session JSON keeps
existing summary keys and adds a `benchmark` context object.

Frame samples now distinguish renderer preparation, GAL validation, hazard
analysis, backend encode/submit, visible/culled instances and instance uploads.
New fields have deserialization defaults for old artifacts.

See [submission benchmarks](../../docs/render-submission.md) for configuration,
Nightwatch integration and the generic repeated-command benchmark runner.

# quasar-diagnostics

Explicit structured logs, spans, counters, failure chains and optional CPU frame
timing. There is no global logger, background runtime, compulsory subscriber or
dependency on a window/graphics stack.

```rust
use quasar_diagnostics::{Diagnostics, Field, Level, MemorySink};
use std::sync::Arc;

let sink = Arc::new(MemorySink::new(256));
let diagnostics = Diagnostics::new(sink.clone());
diagnostics.event_with(
    Level::Info,
    "game",
    "loaded",
    [Field::new("assets", 12_u64)],
);
let span = diagnostics.span("game", "simulate");
// ... work ...
span.finish();
```

## API

- `Diagnostics` is a cheap cloneable handle around an explicit `Sink`.
- `Diagnostics::disabled()` is a no-op handle; every emitter checks it before
  formatting a record.
- `MemorySink` is a bounded ring buffer with a dropped-record counter.
- `CallbackSink` and `TextSink` are small built-ins; applications can implement
  `Sink` for JSON, files, overlays or crash reporting.
- `Record` has a timestamp, level, target, name, structured `Field` values, span
  id, duration and optional `Failure` chain.
- `Diagnostics::span` emits one duration record when dropped or `finish`ed.
  Disabled spans do not allocate ids or call the clock.
- `Counter` and `Gauge` are atomic, independently shareable values. They are not
  globally registered.
- `Level` implements `FromStr` for shared configuration/CLI filtering.
- `DiagnosticsConfig` bounds level filtering, field count, field/name length and
  error-chain depth. `Failure::from_error` copies a bounded source chain.
- With the `timing` feature, `FrameTimer` records last/min/max/total/EMA CPU frame
  durations using `Instant`.

No global state is installed. Attach a handle to an optional subsystem hook, or
pass one explicitly to your own code. The `serde` feature makes records and
enums serializable for a custom sink.

## Integrations

Feature-gated hooks are provided by:

- `quasar-window`: window diagnostics attachment, resize/focus/close and desktop
  metadata failures.
- `quasar-input`: gamepad attach/detach/assignment and event errors.
- `quasar-audio`: audio command queue/device failures.
- `quasar-frame`: resize and presentation lifecycle records.
- `quasar-assets`: load/cache/invalidation diagnostics.
- `quasar-render-3d`: optional preparation spans and submission work counts.
- `quasar-profile`: optional live benchmark completion/threshold-failure reports.
- `quasar-net`: no network crate exists in this repository yet; the standalone
  diagnostics API is ready for a future optional adapter.

Enable a crate's `diagnostics` feature and call its `set_diagnostics`/handle
method. Disabled builds do not compile the diagnostics dependency and do not
take the hook branch.

## Cost, privacy and size

- Disabled handles add only an `Option` check at a call site. Enabled records
  allocate owned strings and fields; spans call a process-local clock.
- `MemorySink` retains at most its configured record count. Each record is
  truncated by `DiagnosticsConfig`, and failures have a bounded chain.
- Timestamps are process wall-clock Unix nanoseconds. No thread, process,
  device, path or user data is collected implicitly; fields are exactly what the
  caller supplies.
- `FrameTimer` is CPU wall-clock timing, not GPU timestamps, input latency or
  presentation latency.
- No performance parity claim is made with specialized tracing runtimes. A local
  release probe (`examples/overhead.rs`) measured roughly 0.2 ns/event disabled
  and 96 ns/event for an enabled, one-field `MemorySink` record on the development
  machine. The design favors explicit ownership and a small disabled path.

## Verification

```sh
cargo test -p quasar-diagnostics
cargo test -p quasar-diagnostics --features timing,serde
cargo clippy -p quasar-diagnostics --all-targets --features timing,serde -- -D warnings
```

Tests cover filtering, truncation, bounded sinks/drops, span lifetimes, disabled
paths, failure chains, cross-thread counters, text formatting, frame timing and
serde round-trips.

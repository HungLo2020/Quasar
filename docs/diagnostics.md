# Diagnostics and observability

Issue #17 is implemented by the independent `quasar-diagnostics` crate and
optional hooks in window/input/audio/frame/asset. The core has no global logger,
runtime, background thread or mandatory third-party logging dependency.

## Architecture decisions

- **Explicit ownership.** A `Diagnostics` handle clones an `Arc` over a sink and
  immutable configuration. Sessions, tests and multiple game systems can use
  different sinks without process-wide registration.
- **No disabled-path formatting.** Callers check `is_enabled` or use an
  `Option<Diagnostics>` hook. Disabled spans allocate no ids and call no clock.
- **Bounded records.** `DiagnosticsConfig` limits fields, field/name characters
  and failure-chain depth before the sink sees a record. `MemorySink` has a
  fixed capacity and tracks dropped records.
- **Structured first.** Records carry target, name, typed fields, span identity,
  duration and an owned failure chain. Text is one possible rendering, not the
  storage model.
- **Explicit failure capture.** `Failure::from_error` copies a bounded
  `std::error::Error` source chain; no borrowed error is retained across threads.
- **Optional timing.** `FrameTimer` is behind `timing`; it measures CPU frame
  intervals only, not GPU/presentation timing.
- **Feature-gated integrations.** Existing crates do not acquire diagnostics
  unless their `diagnostics` feature is enabled. Hook methods accept an explicit
  handle and remain absent from the default dependency graph.

## Instrumented points

| Crate | Current hook |
|---|---|
| window | attachment, resize, focus, close, desktop metadata synchronization failures |
| input | gamepad connect/disconnect/assignment and event errors |
| audio | command queue/device failure paths |
| frame | resize and presentation lifecycle |
| assets | load success/failure, cache hits, invalidation |
| net | no network crate exists yet; use `quasar-diagnostics` directly when one is added |

## Privacy and log size

Nothing is collected implicitly. The caller supplies every field. Records own
strings, so sinks must treat them as potentially sensitive. Use a bounded
`MemorySink`, a custom sink with retention policy, or `TextSink`; configure
`DiagnosticsConfig` to cap field and error size. Timestamps are Unix wall-clock
nanoseconds from `SystemTime`, not a privacy-preserving monotonic clock.

## Cost and limitations

- Enabled records allocate; disabled handles are a cheap branch/`Option` check.
- No global subscriber, runtime, sampling, rate limiting, compression, rotation
  or network export is built in. Implement those in a `Sink` or the application.
- Span propagation is explicit; there is no async task-local/thread-local context
  propagation.
- Frame timing is CPU wall-clock, not a GPU or input-to-photon benchmark.
- There is no `quasar-net` crate; no network hook is claimed.
- The crate has deterministic unit/integration tests but no published benchmark
  comparison with `tracing` or other logging runtimes.

## Local cost measurement

`cargo run --release -p quasar-diagnostics --example overhead` on the development
machine reported approximately **0.23 ns/event** for a disabled handle and
**96 ns/event** for an enabled `MemorySink` record (one structured field, a
bounded 1024-record ring that dropped older records). These are local
microbenchmark observations, not a portable guarantee; sink cost, allocator and
CPU dominate enabled records. The disabled path performs no clock call, string
formatting or sink work.

## Verification

```sh
cargo test -p quasar-diagnostics
cargo test -p quasar-diagnostics --features timing,serde
cargo clippy -p quasar-diagnostics --all-targets --features timing,serde -- -D warnings
cargo check -p quasar-input --features diagnostics
cargo check -p quasar-window --features diagnostics
cargo check -p quasar-audio --features diagnostics
cargo check -p quasar-frame --features diagnostics
cargo check -p quasar-assets --features diagnostics
```

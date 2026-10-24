# Background job scheduling

Issue #39 is implemented by the independently importable `quasar-jobs` crate.
It is a bounded, caller-owned executor for CPU/file work. It is not a global
service, async runtime, ECS scheduler, renderer dependency, or replacement for
`quasar-app`.

## Why a small pool instead of Rayon?

Rayon is mature and excellent for data-parallel fork/join work, but this issue
requires properties Rayon does not expose directly:

- a hard bound on outstanding submissions with explicit backpressure;
- per-job cancellation and dependency failure propagation;
- total FIFO ordering within equal priorities;
- deterministic single-thread/manual execution for headless tests;
- shutdown and metrics owned by the caller's pool rather than a global runtime.

The implementation therefore uses `std` threads, `Mutex`/`Condvar`, and
per-job `mpsc` result channels. It has no runtime dependency and no global
singleton. Callers are free to use Rayon inside a `quasar-jobs` task when
data-parallel algorithms are a better fit.

## Architecture

- `JobPool` owns a bounded worker set and a shared scheduler state.
- `State` holds waiting/ready/running jobs, a priority/FIFO ready heap, counters
  and dependency edges.
- `JobPriority` is ordered high-to-low; equal priorities use submission sequence.
- `JobHandle<T>` receives one erased typed result, exposes status/cancellation and
  can be used as a `JobDependency`.
- `JobContext` exposes the job id and a cooperative cancellation flag.
- `JobError` preserves task errors as `Error::source` and classifies queue,
  dependency, cancellation, panic and shutdown failures.
- A manual pool uses the same scheduler but executes with `run_next`/`run_all`
  on the caller thread. This is the deterministic test path.

### Dependencies

A dependency token is obtained from a submitted handle. The scheduler keeps a
count of unresolved dependencies; a job becomes ready when all dependencies
complete successfully. Any failed/cancelled dependency marks the dependent as
`DependencyFailed` and propagates to its dependents. Dependency tokens carry the
owning pool identity, so a token from another pool is rejected.

### Cancellation and shutdown

Queued cancellation is immediate and observable through the handle. Running
cancellation is cooperative: tasks poll `JobContext::is_cancelled` (or use
`check_cancelled`). A running job whose flag was set before completion is
reported as `Cancelled`, even if its closure returned success. Current tasks
cannot be forcibly pre-empted.

`shutdown()` stops new submissions, lets queued/running work finish and joins
workers. A manual pool cancels queued work because no worker exists to run it.
`shutdown_cancelling()` requests cancellation first. `Drop` only signals
cancellation and detaches workers; this avoids a self-join deadlock when a task
holds the last `Arc<JobPool>`. Call `shutdown()` explicitly for deterministic
teardown. A permanently blocked task can still delay that explicit shutdown.

## Benchmarks

Run:

```sh
cargo run --release -p quasar-jobs --example job_benchmark
```

The sampled workload is 2,000 tasks, each performing 40,000 deterministic
multiply/add operations. A local Linux x86_64 run on an Intel i7-10750H
(rustc 1.94.0) produced:

| Workers | Wall time | Throughput | Average queue delay | Max queue delay |
|---:|---:|---:|---:|---:|
| 1 | 12.73 ms | 157,176 tasks/s | 5,745 µs | 11,001 µs |
| 2 | 6.50 ms | 307,652 tasks/s | 2,622 µs | 5,348 µs |
| 4 | 3.55 ms | 563,234 tasks/s | 652 µs | 1,129 µs |
| 8 | 3.91 ms | 511,780 tasks/s | 1.9 µs | 21.3 µs |
| Manual | 11.33 ms | 176,483 tasks/s | n/a | n/a |

These are single-host observations, not portable guarantees. The 8-worker run
shows scheduler/CPU contention; 4 workers was the best point in this sample.
Queue delay includes time between submission and task start. Run the benchmark
multiple times before making capacity decisions.

Memory use is dominated by the outstanding-job bound. Each outstanding job keeps
one closure, one result slot and dependency metadata; choose `max_outstanding`
from the largest acceptable burst, not from the total lifetime number of jobs.
The standalone benchmark process peaked at about **5.1 MiB RSS** (Linux
`/usr/bin/time -v`, excluding compiler processes), so the pool's own scheduling
state is small relative to task payloads. This is not an allocator-level
measurement.

## Read-only game audit

- MissileCommand: gameplay is a 120 Hz pure state update; audio synthesis,
  record/preferences JSON and framebuffer presentation are small and currently
  main-thread. A job pool is not needed for its hot path. If a future wave or
  audio asset becomes expensive, decode/generate it in a task and apply the
  owned result at a tick boundary.
- MicroMages: startup asset decoding (PNG/Ogg/JSON), audio setup and level data
  are plausible CPU/file workloads. They can run in a caller-owned pool while
  the main thread keeps the simulation and VulkanicGAL presenter. No game
  mutation belongs in a worker, and the game remains responsible for applying
  returned assets.
- Neither repository was modified. `quasar-jobs` does not impose a job graph or
  runner on either game.

## Interoperability

- Asset loading: move an `Arc<AssetServer>` clone into a task and return typed
  handles. The asset server remains caller-owned.
- Procgen/nav/physics: return owned results; use `quasar-sim` for deterministic
  tick/replay coordination rather than scheduling simulation state from workers.
- Diagnostics: enable the `diagnostics` feature and pass an explicit
  `Diagnostics` handle to `JobPool::with_diagnostics`.
- No mandatory `quasar-app`, window, ECS, renderer or network dependency.

## Acceptance criteria

- Independently usable in ordinary Cargo builds without Quasar-specific tooling.
- Bounded outstanding work with explicit backpressure and optional blocking wait.
- Configurable worker count, including deterministic manual execution.
- Cancellation of queued jobs and cooperative cancellation of running jobs.
- Panic isolation, task error source preservation and dependency failure
  propagation.
- Graceful and cancelling shutdown paths with no orphaned result handles.
- Priority and FIFO ordering with deterministic manual results.
- Metrics and optional diagnostics.
- No global mandatory thread pool, audio-callback work or async runtime.
- Tests for cancellation, failures, deadlocks/shutdown, dependency order, bounds,
  concurrent submission, threaded dependency chains, self-join avoidance,
  stress and property-based ordering.
- Benchmarks documenting throughput, scaling and queue latency.

## Limitations

- Cooperative cancellation cannot stop a task that ignores the flag or blocks
  forever; graceful shutdown then waits indefinitely by design.
- No work stealing, priorities across pools, NUMA/affinity policy or bulk graph
  cycle validation.
- One queue per pool; callers choose worker counts and must avoid accidental
  oversubscription.
- Do not block every worker waiting on the same pool's capacity; that is a
  classic bounded-pool deadlock. Spawn sub-work without waiting, or use a
  separate pool.
- Not intended for async I/O; use an async runtime for non-blocking network/file
  protocols.

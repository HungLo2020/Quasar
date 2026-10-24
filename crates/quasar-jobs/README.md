# quasar-jobs

Optional bounded background task scheduling for Quasar. It is independently
usable, has no global pool, and does not require async, an ECS, a renderer or an
application runner.

```toml
quasar-jobs = { path = "../Quasar/crates/quasar-jobs" }
```

```rust
use quasar_jobs::{JobPool, JobPoolConfig, JobPriority};

let pool = JobPool::new(JobPoolConfig::new(4, 256))?;
let decode = pool.spawn(|| {
    // Move caller-owned data into the task.
    Ok::<_, std::io::Error>(vec![1_u8, 2, 3])
})?;
let analyse = pool.spawn_with_dependencies(&[decode.dependency()], |_| {
    Ok::<_, std::io::Error>(42_u64)
})?;
println!("{} {}", decode.wait()?.len(), analyse.wait()?);
pool.shutdown()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Behavior

- `workers == 0` is a deterministic manual pool: `run_next()`/`run_all()` execute
  jobs on the caller thread in priority/submission order.
- `max_outstanding` bounds queued + running jobs. `spawn` returns
  `QueueFull`; `spawn_wait` waits for capacity (except on manual pools).
  Same-pool nested blocking submission, nested handle waits and worker
  self-shutdown return `NestedBlocking` instead of deadlocking.
- Higher `JobPriority` values run first; equal priorities preserve submission
  order.
- Dependencies may be attached while outstanding. Completed-success
  dependencies are satisfied; failed/cancelled dependencies fail dependents.
  Dependency tokens cannot cross pool boundaries.
- Cancellation is cooperative for running jobs through `JobContext`; queued jobs
  finish immediately as `Cancelled`.
- Panics are caught per job and reported as `JobErrorKind::Panicked`; workers
  remain usable.
- `shutdown()` stops submissions, finishes queued/running work and joins workers.
  `shutdown_cancelling()` cancels queued work and requests cooperative
  cancellation before joining. `Drop` only signals cancellation/detaches, so a
  task holding the last `Arc<JobPool>` cannot self-join; call `shutdown()`
  explicitly for deterministic teardown.
- `metrics()` reports outstanding, queued, running, submitted, completed,
  failed, cancelled, panicked and peak outstanding counts.

The pool owns threads, not assets or game state. Tasks may move owned values or
share caller-owned `Arc` resources.

## Features

- `diagnostics`: emits submitted/finished events through an explicit
  `quasar_diagnostics::Diagnostics` handle.

## Verification

```sh
cargo test -p quasar-jobs
cargo test -p quasar-jobs --features diagnostics
cargo clippy -p quasar-jobs --all-targets -- -D warnings
cargo clippy -p quasar-jobs --all-targets --features diagnostics -- -D warnings
cargo run --release -p quasar-jobs --example job_benchmark
```

Tests cover dependency order, priority/FIFO order, queue bounds, blocking
submission, queued/running cancellation, panic and error propagation,
graceful/cancelling shutdown, cross-pool dependency rejection, concurrent and
chained submission, nested wait/submission rejection, self-join avoidance,
stress, result-taking, metrics and a property test over priority ordering. The benchmark reports
throughput and submission-to-start queue latency.

## Limitations

- Running tasks cannot be forcibly stopped; cancellation is cooperative. A task
  that ignores cancellation and blocks forever also blocks graceful shutdown.
- Blocking pool threads are appropriate for CPU/file work, not async I/O.
- One queue per pool. Choosing worker counts is caller policy; avoid pooling
  every subsystem into a single oversubscribed global default.
- Dependency graphs are constructed incrementally from handles. The pool does
  not provide a separate bulk graph validator or cycle detector because tokens
  cannot reference future/unsubmitted jobs.
- There is no work stealing or cross-pool scheduling. Priority is meaningful
  within a pool, not across independent pools.
- No thread-affinity or NUMA policy is imposed.
- Same-pool nested waits/submission/shutdown are rejected. Blocking on unrelated
  synchronization while workers are occupied still requires caller care.

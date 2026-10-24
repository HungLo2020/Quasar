# Asynchronous assets and hot reload

Issue #10 is implemented as optional features on top of the synchronous
`quasar-assets` server. No async executor, GPU object or application runner is
owned by the asset core.

## Features

- `async`: `AsyncAssetServer`, `AsyncRequest<T>`, `AsyncLoaded<T>`,
  `AsyncInvalidation`
- `hot-reload`: `AssetWatcher`, `WatchBatch`, `AssetServer::watch_roots`,
  `AssetServer::invalidate`
- `diagnostics`: asset load/cache/invalidation hooks
- `serde` and `storage` remain independent.

## Architecture

### Async worker

`AsyncAssetServer::new` moves a fully configured synchronous `AssetServer` onto
one named worker thread. The caller builds mounts/loaders first, then submits
typed jobs. Jobs run serially, which makes cache mutation, loader behavior and
test ordering deterministic without a scheduler.

```rust
let mut assets = AssetServer::new();
assets.register(MyLoader);
assets.mount_file("", "assets")?;
let queue = AsyncAssetServer::new(assets, AsyncConfig { queue_capacity: 64 })?;
let mut request = queue.load::<MyLoader>("ui/title.txt")?;
if let Some(result) = request.wait(std::time::Duration::from_millis(16)) {
    let loaded = result?;
    // Use loaded.handle; old handles remain valid after eviction/reload.
}
```

Requests have a one-result channel and the job queue is bounded by
`AsyncConfig::queue_capacity`; saturation returns `AssetErrorKind::QueueFull`.
Dropping or cancelling an `AsyncRequest` marks queued work cancelled. A loader
already running cannot be forcibly stopped; the worker checks cancellation before
and after loading and returns `AssetErrorKind::Cancelled`.

The worker's synchronous `Drop` closes the queue and joins the thread, waiting
for any in-flight loader. A loader that blocks forever therefore blocks shutdown.

### Revision and conflict handling

`AssetServer::revision` increments on invalidation, cache clear and unload.
`Handle::revision` records the revision a value was loaded under.
`AsyncRequest::submitted_revision` and `AsyncLoaded::revision`/`is_stale` let an
application reject stale results. `AsyncAssetServer::reload::<L>(path, cascade)` invalidates
then loads the target; the resulting `AsyncLoaded::invalidated` lists every path
whose cache entry was removed. Existing handles keep their old value and report
their old revision.

### Hot reload

`AssetWatcher::new(root, virtual_prefix)` watches a filesystem root recursively
using `notify`. `poll` returns a sorted/deduplicated `WatchBatch`; watcher errors
and bounded-queue drops are reported separately. It never mutates a server.
Applications map changed paths to their loader types and call:

```rust
let mut reload = queue.reload::<MyLoader>("ui/title.txt", true)?;
let loaded = reload.wait(std::time::Duration::from_secs(1));
```

Cascade invalidation walks the cached dependency graph. A dependency's cached
dependents are removed, but game-owned handles remain memory-safe until dropped.

### GPU adapters

The asset core does not own textures, buffers or a GPU device. A game that maps
an asset to GPU resources should use an explicit adapter keyed by
`AssetPath`/revision, replace its own GPU resource on a new handle, and retire
the old GPU object through its renderer's normal lifetime rules. No renderer is
linked into `quasar-assets`.

## Tests and failure injection

Run:

```sh
cargo test -p quasar-assets --features async
cargo test -p quasar-assets --features hot-reload
cargo test -p quasar-assets --features async,hot-reload,serde,storage,diagnostics
cargo clippy -p quasar-assets --all-targets --features async,hot-reload,serde,storage,diagnostics -- -D warnings
```

Tests cover successful loads, loader failure followed by recovery, a blocked
loader with a full queue, cancellation of queued work, invalidation ordering,
dependent invalidation, old-handle lifetime after reload, revision changes,
bounded watcher event queues, watcher path mapping from a temporary directory and
explicit server roots. The watcher tests poll with deadlines rather than sleeping
for a fixed duration.

## Limitations

- One worker, one serialized asset server. There is no work stealing, rayon or
  parallel loader execution.
- No async file I/O; the worker reads and decodes synchronously. This is simple
  and bounded, but large assets occupy the worker.
- No automatic dependency prefetch: dependencies are metadata until the
  application requests them.
- Reloads are explicit. The watcher reports paths; it does not know which Rust
  loader type owns a path, so typed reload requests come from the game/adapter.
- Windows/macOS hot-reload events are provided by `notify` and were not run on
  hardware here. Linux inotify path tests were run.
- No GPU ownership, texture updates, compression containers, network assets,
  content hashing or editor integration.
- No portable performance claim is made. A local release probe
  (`cargo run --release -p quasar-assets --example async_benchmark --features async`)
  loading 2,000 memory-backed 256-byte `Bytes` assets took approximately 4.5 ms
  synchronously versus 7.7 ms through the single async worker on the development
  machine (about 2.3 vs 3.8 µs per asset). The worker adds queue/channel overhead;
  it is intended for non-blocking game loops, not higher single-threaded
  throughput. Real file and decode time dominates.

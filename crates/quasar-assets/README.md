# quasar-assets

Validated virtual paths, mounts, typed loaders, dependency metadata and a
bounded cache. The crate is headless; its core is synchronous, with optional
threaded loading and filesystem hot-reload features. It does not require an
editor, build script, ECS, renderer, or a particular directory layout.

```rust
use quasar_assets::{Asset, AssetLoader, AssetServer, FileSource, LoadContext};
use std::sync::Arc;

#[derive(Debug)]
struct Text(String);
impl Asset for Text {}

struct TextLoader;
impl AssetLoader for TextLoader {
    type Asset = Text;
    fn load(&self, context: &LoadContext<'_>) -> Result<Text, quasar_assets::AssetError> {
        Ok(Text(String::from_utf8_lossy(context.bytes()).into_owned()))
    }
}

let mut server = AssetServer::new();
server.register(TextLoader);
server.mount_file("", "assets")?;
let text = server.load::<TextLoader>("ui/title.txt")?;
println!("{}", text.0);
```

## Design

- **Validated paths.** `AssetPath` is relative, `/`-separated, non-empty, has no
  `.`/`..`/empty segments, and rejects backslashes, colons, control characters
  and NUL. All public read/load APIs either accept an `AssetPath` or parse one.
- **Mounts.** `AssetServer::mount` maps a validated prefix to an
  `AssetSource`. The longest matching prefix wins. An empty prefix is the root.
  Duplicate prefixes are rejected explicitly.
- **Sources.** `FileSource` canonicalizes its root and checks each file against
  it, preventing symlink escapes. `MemorySource` supports embedded/generated
  content and deterministic tests. `AssetSource` is a small public trait for
  archives or other backends.
- **Typed loaders.** `AssetLoader` has an associated `Asset` type. Loaders are
  registered explicitly; the server never guesses a decoder from an extension.
  `LoadContext` exposes bytes, path, source name, schema version and
  `require_schema_version`. `BytesLoader`/`Bytes` provide a baseline raw loader;
  format-specific loaders stay optional. The `gltf` feature provides the
  static 3D loader described below.
- **Typed handles.** `Handle<T>` is cheap to clone, dereferences to `T`, records
  its path/id/schema version, and keeps the value alive even if the cache evicts
  it.
- **Cache budgets.** `CacheBudget` limits entries and source/retained bytes.
  Eviction skips values with live handles or cached dependents. Budgets are soft
  while handles are live; a later load trims again.
- **Dependencies.** `Asset::dependencies` records paths. The server exposes
  `dependencies`, `dependents` and deterministic `load_order` with cycle errors.
  `unload` refuses live handles and dependents.
- **Schema handling.** Loaders and assets expose a schema version; mismatch is an
  `AssetErrorKind::Schema`. Format-specific version parsing stays in the loader.
- **Errors.** `AssetError` categories are stable and machine-matchable:
  invalid path, not mounted, not found, I/O, schema, loader, budget, dependency,
  busy, configuration, duplicate mount and wrong type.

## Cargo and Basalt

The core has no required dependencies and does not read environment variables.
Use any directory:

```rust
server.mount_file("", "assets")?;                     // Cargo checkout
server.mount_file("textures", "assets/textures")?;   // nested mount
```

With the optional `storage` feature, `FileSource::from_basalt` and
`AssetServer::mount_basalt` resolve `app/<relative>` first and fall back to the
Basalt game root. This works with `quasar-storage::GamePaths` and does not imply
a packaged build step.

```rust
server.mount_basalt("", &paths, "assets")?;
```

## Async loading and hot reload

The default server is synchronous. Optional features add a bounded worker and
filesystem watching without an executor or runtime:

```toml
quasar-assets = { path = "...", features = ["async", "hot-reload", "diagnostics"] }
```

```rust
let mut assets = AssetServer::new();
assets.register(TextLoader);
assets.mount_file("", "assets")?;
let queue = AsyncAssetServer::new(assets, AsyncConfig { queue_capacity: 64 })?;
let mut load = queue.load::<TextLoader>("ui/title.txt")?;
let loaded = load.wait(std::time::Duration::from_millis(16));

// A watcher reports paths; reload is explicit because only the application
// knows which typed loader owns a path.
let watcher = AssetWatcher::new("assets", "")?;
let changed = watcher.poll();
let mut reload = queue.reload::<TextLoader>("ui/title.txt", true)?;
```

`AsyncRequest` supports cancellation, a bounded queue and a one-result channel.
`AsyncAssetServer::reload` invalidates the target and, with `cascade`, cached
dependents before loading the new value. `Handle` values own an `Arc`, so old
handles remain valid and carry their old revision after invalidation.
`AssetServer::revision`, `Handle::revision`, `AsyncRequest::submitted_revision`
and `AsyncLoaded::revision` support conflict/staleness handling. The watcher
queue is bounded and reports dropped events. See
[async assets](../../docs/async-assets.md) for the full lifecycle and limitations.

## Features and limits

- `serde` makes `AssetPath` and `AssetId` serializable.
- `storage` adds the optional Basalt path helper.
- `diagnostics` attaches explicit load/cache/invalidation hooks.
- `async` adds the background worker; `hot-reload` adds `notify`-based watching.
- The async worker is serial and synchronous internally. File watching reports
  paths but does not choose a typed loader or automatically mutate GPU resources.
  Memory-mapped files, compression containers, automatic dependency prefetch and
  partial opaque regions are not implemented. Shutdown waits for an in-flight
  loader to return.
- `FileSource` has a default 256 MiB per-file limit; configure it with
  `with_max_file_bytes`.
- The cache budget counts source bytes or `Asset::memory_size` when larger. It
  is an LRU approximation and never invalidates a live handle.

## Verification

```sh
cargo test -p quasar-assets
cargo test -p quasar-assets --features serde,storage
cargo clippy -p quasar-assets --all-targets -- -D warnings
cargo clippy -p quasar-assets --all-targets --features serde,storage -- -D warnings
```

Headless tests cover path rejection, longest-prefix mounts, symlink escape
rejection, typed cache hits, eviction, live handles after cache clear, dependency
ordering/cycles, unload restrictions, schema mismatch, missing sources/loaders,
memory/file sources and Basalt mounts.

## Optional 3D model assets

The dependency-free `model` feature exposes CPU geometry/material/image/sampler
and transform data. `gltf` adds bounded glTF/GLB loading with PNG/JPEG images,
mounted external dependencies, base64 buffers, flat-normal generation and
explicit unsupported-feature errors. No renderer or runtime object model is
required. `ModelLimits` supplies per-import budgets; recorded external reads
participate in existing dependency invalidation and async loading.

`LoadContext::read_relative`, `read_limited` and loader source-byte budgets are
also available to other format loaders. See [3D model assets](../../docs/model-assets-3d.md)
for scope, URI handling, cache/revision policy, GPU adapters and validation.

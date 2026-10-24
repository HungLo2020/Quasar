#![cfg(feature = "async")]

use quasar_assets::{
    Asset, AssetErrorKind, AssetLoader, AssetServer, AsyncAssetServer, AsyncConfig, BytesLoader,
    FileSource, LoadContext, MemorySource,
};
#[cfg(feature = "hot-reload")]
use std::time::Instant;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug)]
struct Text(String);
impl Asset for Text {
    fn memory_size(&self) -> usize {
        self.0.len()
    }
}
struct TextLoader;
impl AssetLoader for TextLoader {
    type Asset = Text;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        if context.bytes() == b"bad" {
            return Err(quasar_assets::AssetError::new(
                AssetErrorKind::Loader,
                "injected failure",
            ));
        }
        Ok(Text(String::from_utf8_lossy(context.bytes()).into_owned()))
    }
}

struct BlockingLoader {
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Arc<AtomicBool>,
}
impl AssetLoader for BlockingLoader {
    type Asset = Text;
    fn load(&self, _: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        self.started.store(true, Ordering::Release);
        let (lock, condition) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = condition.wait(released).unwrap();
        }
        Ok(Text("blocked".into()))
    }
}

struct BaseLoader;
impl AssetLoader for BaseLoader {
    type Asset = Text;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        Ok(Text(String::from_utf8_lossy(context.bytes()).into_owned()))
    }
}
#[derive(Debug)]
struct ChildText(String);
impl Asset for ChildText {
    fn dependencies(&self) -> Vec<quasar_assets::AssetPath> {
        vec![quasar_assets::AssetPath::parse("base.txt").unwrap()]
    }
    fn memory_size(&self) -> usize {
        self.0.len()
    }
}
struct ChildLoader;
impl AssetLoader for ChildLoader {
    type Asset = ChildText;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        Ok(ChildText(
            String::from_utf8_lossy(context.bytes()).into_owned(),
        ))
    }
}

fn memory() -> Arc<MemorySource> {
    let mut source = MemorySource::new("async fixture");
    source.insert("a.txt", b"alpha".to_vec()).unwrap();
    source.insert("b.txt", b"beta".to_vec()).unwrap();
    source.insert("c.txt", b"gamma".to_vec()).unwrap();
    source.insert("bad.txt", b"bad".to_vec()).unwrap();
    Arc::new(source)
}

fn take<T>(
    request: &mut quasar_assets::AsyncRequest<T>,
) -> Result<quasar_assets::AsyncLoaded<T>, quasar_assets::AssetError> {
    request
        .wait(Duration::from_secs(3))
        .expect("async result before timeout")
}

#[test]
fn async_server_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AsyncAssetServer>();
}

#[test]
fn async_loads_report_success_and_loader_failures_without_killing_the_worker() {
    let mut server = AssetServer::new();
    server.register(TextLoader);
    server.mount("", memory()).unwrap();
    let server = AsyncAssetServer::new(server, AsyncConfig::default()).unwrap();
    let mut first = server.load::<TextLoader>("a.txt").unwrap();
    assert_eq!(take(&mut first).unwrap().handle.0, "alpha");
    let mut bad = server.load::<TextLoader>("bad.txt").unwrap();
    assert_eq!(take(&mut bad).unwrap_err().kind(), AssetErrorKind::Loader);
    let mut recovered = server.load::<TextLoader>("b.txt").unwrap();
    assert_eq!(take(&mut recovered).unwrap().handle.0, "beta");
}

#[test]
fn async_queue_is_bounded_and_queued_work_can_be_cancelled() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let started = Arc::new(AtomicBool::new(false));
    let mut server = AssetServer::new();
    server.register(TextLoader);
    server.register(BlockingLoader {
        gate: Arc::clone(&gate),
        started: Arc::clone(&started),
    });
    server.mount("", memory()).unwrap();
    let server = AsyncAssetServer::new(server, AsyncConfig { queue_capacity: 1 }).unwrap();
    let mut first = server.load::<BlockingLoader>("a.txt").unwrap();
    while !started.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    let mut queued = server.load::<BlockingLoader>("b.txt").unwrap();
    let third = server.load::<BlockingLoader>("c.txt");
    assert!(matches!(
        third,
        Err(error) if error.kind() == AssetErrorKind::QueueFull
    ));
    queued.cancel();
    {
        let (lock, condition) = &*gate;
        *lock.lock().unwrap() = true;
        condition.notify_all();
    }
    assert_eq!(take(&mut first).unwrap().handle.0, "blocked");
    assert!(matches!(
        take(&mut queued),
        Err(error) if error.kind() == AssetErrorKind::Cancelled
    ));
    let mut recovered = server.load::<TextLoader>("a.txt").unwrap();
    // Different loader type is fine; the worker is still alive.
    assert_eq!(take(&mut recovered).unwrap().handle.0, "alpha");
}

#[test]
fn reload_invalidates_dependencies_and_old_handles_remain_valid() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("base.txt"), b"base-v1").unwrap();
    std::fs::write(directory.path().join("child.txt"), b"child-v1").unwrap();
    let mut server = AssetServer::new();
    server.register(BaseLoader).register(ChildLoader);
    server
        .mount("", Arc::new(FileSource::new(directory.path()).unwrap()))
        .unwrap();
    let server = AsyncAssetServer::new(server, AsyncConfig::default()).unwrap();
    let mut base = server.load::<BaseLoader>("base.txt").unwrap();
    let base_old = take(&mut base).unwrap().handle;
    let mut child = server.load::<ChildLoader>("child.txt").unwrap();
    let child_old = take(&mut child).unwrap().handle;
    let old_revision = base_old.revision();
    std::fs::write(directory.path().join("base.txt"), b"base-v2").unwrap();
    let mut reload = server.reload::<BaseLoader>("base.txt", true).unwrap();
    let loaded = take(&mut reload).unwrap();
    assert_eq!(loaded.handle.0, "base-v2");
    assert!(loaded.revision > old_revision);
    assert!(
        loaded
            .invalidated
            .iter()
            .any(|path| path.as_str() == "base.txt")
    );
    assert!(
        loaded
            .invalidated
            .iter()
            .any(|path| path.as_str() == "child.txt")
    );
    assert_eq!(base_old.0, "base-v1");
    assert_eq!(child_old.0, "child-v1");
    let mut child_reload = server.load::<ChildLoader>("child.txt").unwrap();
    assert_eq!(take(&mut child_reload).unwrap().handle.0, "child-v1");
}

#[test]
fn async_invalidation_is_ordered_and_reports_a_revision() {
    let mut server = AssetServer::new();
    server.register(TextLoader);
    server.mount("", memory()).unwrap();
    let server = AsyncAssetServer::new(server, AsyncConfig::default()).unwrap();
    let mut load = server.load::<TextLoader>("a.txt").unwrap();
    let loaded = take(&mut load).unwrap();
    let revision = loaded.revision;
    let mut invalidation = server.invalidate("a.txt", false).unwrap();
    let report = invalidation
        .wait(Duration::from_secs(3))
        .expect("invalidation result")
        .unwrap();
    assert!(report.revision > revision);
    assert_eq!(
        report.paths,
        vec![quasar_assets::AssetPath::parse("a.txt").unwrap()]
    );
    let mut reloaded = server.load::<TextLoader>("a.txt").unwrap();
    assert!(take(&mut reloaded).unwrap().revision > revision);
}

#[test]
fn bytes_loader_works_through_the_async_worker() {
    let mut server = AssetServer::new();
    server.register(BytesLoader);
    server.mount("", memory()).unwrap();
    let server = AsyncAssetServer::new(server, AsyncConfig::default()).unwrap();
    let mut request = server.load::<BytesLoader>("a.txt").unwrap();
    assert_eq!(take(&mut request).unwrap().handle.as_slice(), b"alpha");
}

#[cfg(feature = "hot-reload")]
#[test]
fn watcher_reports_changed_virtual_paths() {
    use quasar_assets::AssetWatcher;
    let directory = tempfile::tempdir().unwrap();
    let watcher = AssetWatcher::new(directory.path(), "assets").unwrap();
    std::fs::write(directory.path().join("watched.txt"), b"one").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = false;
    while Instant::now() < deadline {
        let batch = watcher.poll();
        if batch
            .paths
            .iter()
            .any(|path| path.as_str() == "assets/watched.txt")
        {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen, "watcher did not report the changed file");
}

#[cfg(feature = "hot-reload")]
#[test]
fn watcher_reports_server_roots_for_file_mounts() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("root.txt"), b"x").unwrap();
    let mut server = AssetServer::new();
    server
        .mount("game", Arc::new(FileSource::new(directory.path()).unwrap()))
        .unwrap();
    let roots = server.watch_roots();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].0, "game");
    assert!(roots[0].1.is_dir());
    let source = FileSource::new(directory.path()).unwrap();
    assert_eq!(
        quasar_assets::AssetSource::watch_root(&source),
        Some(source.root())
    );
}

//! Optional filesystem watching for hot reload.
//!
//! The watcher only reports virtual paths and errors. It does not own the asset
//! server, loaders or GPU resources, and it never mutates a cache implicitly.
//! Applications poll it and submit [`crate::AsyncAssetServer::reload`] or
//! [`crate::AssetServer::invalidate`] requests explicitly.
use crate::{AssetError, AssetErrorKind, AssetPath, AssetSource};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Component, Path},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
};

/// A normalized set of changed virtual paths and watcher errors.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WatchBatch {
    pub paths: Vec<AssetPath>,
    pub errors: Vec<String>,
    /// Events dropped because the bounded watcher queue was full.
    pub dropped: u64,
}
impl WatchBatch {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.errors.is_empty() && self.dropped == 0
    }
}

/// Watches one filesystem root and maps events to a virtual mount prefix.
pub struct AssetWatcher {
    root: std::path::PathBuf,
    prefix: Option<AssetPath>,
    events: Receiver<notify::Result<notify::Event>>,
    dropped: Arc<AtomicU64>,
    queue_capacity: usize,
    _watcher: RecommendedWatcher,
}
impl AssetWatcher {
    pub fn new(root: impl AsRef<Path>, prefix: impl AsRef<str>) -> Result<Self, AssetError> {
        Self::with_capacity(root, prefix, 64)
    }
    /// Creates a watcher with a bounded raw-event queue. When full, events are
    /// dropped and counted rather than growing memory without limit.
    pub fn with_capacity(
        root: impl AsRef<Path>,
        prefix: impl AsRef<str>,
        queue_capacity: usize,
    ) -> Result<Self, AssetError> {
        if queue_capacity == 0 {
            return Err(AssetError::new(
                AssetErrorKind::Configuration,
                "watcher queue_capacity must be greater than zero",
            ));
        }
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            AssetError::new(
                AssetErrorKind::Io,
                format!("watch root {}: {error}", root.as_ref().display()),
            )
        })?;
        if !root.is_dir() {
            return Err(AssetError::new(
                AssetErrorKind::Io,
                format!("watch root {} is not a directory", root.display()),
            ));
        }
        let prefix = if prefix.as_ref().is_empty() {
            None
        } else {
            Some(AssetPath::parse(prefix)?)
        };
        let (sender, events): (SyncSender<_>, Receiver<_>) = sync_channel(queue_capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let callback_dropped = Arc::clone(&dropped);
        let mut watcher = notify::recommended_watcher(move |event| {
            if let Err(TrySendError::Full(_)) = sender.try_send(event) {
                callback_dropped.fetch_add(1, Ordering::Relaxed);
            }
        })
        .map_err(|error| {
            AssetError::new(
                AssetErrorKind::Configuration,
                format!("create asset watcher: {error}"),
            )
        })?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| {
                AssetError::new(
                    AssetErrorKind::Io,
                    format!("watch {}: {error}", root.display()),
                )
            })?;
        Ok(Self {
            root,
            prefix,
            events,
            dropped,
            queue_capacity,
            _watcher: watcher,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn prefix(&self) -> Option<&AssetPath> {
        self.prefix.as_ref()
    }
    pub fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }
    /// Drains pending events without blocking. Paths are sorted and deduplicated
    /// so a batch is deterministic.
    pub fn poll(&self) -> WatchBatch {
        let mut batch = WatchBatch::default();
        loop {
            match self.events.try_recv() {
                Ok(Ok(event)) => {
                    for path in event.paths {
                        match self.map_path(&path) {
                            Ok(Some(path)) => batch.paths.push(path),
                            Ok(None) => {}
                            Err(error) => batch.errors.push(error.to_string()),
                        }
                    }
                }
                Ok(Err(error)) => batch.errors.push(error.to_string()),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        batch.paths.sort();
        batch.paths.dedup();
        batch.dropped = self.dropped.swap(0, Ordering::Relaxed);
        batch
    }
    fn map_path(&self, path: &Path) -> Result<Option<AssetPath>, AssetError> {
        let relative = match path.strip_prefix(&self.root) {
            Ok(relative) => relative.to_path_buf(),
            Err(_) => {
                let Ok(canonical) = std::fs::canonicalize(path) else {
                    return Ok(None);
                };
                let Ok(relative) = canonical.strip_prefix(&self.root) else {
                    return Ok(None);
                };
                relative.to_path_buf()
            }
        };
        if relative.as_os_str().is_empty() {
            return Ok(None);
        }
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => {
                    let Some(part) = part.to_str() else {
                        return Ok(None);
                    };
                    parts.push(part);
                }
                _ => return Ok(None),
            }
        }
        let relative = parts.join("/");
        let path = match &self.prefix {
            Some(prefix) => prefix.join(relative),
            None => AssetPath::parse(relative),
        }?;
        Ok(Some(path))
    }
}

/// Convenience: create watchers for every filesystem root exposed by a server.
pub fn watchers_for_server(server: &crate::AssetServer) -> Result<Vec<AssetWatcher>, AssetError> {
    server
        .watch_roots()
        .into_iter()
        .map(|(prefix, root)| AssetWatcher::new(root, prefix))
        .collect()
}

/// Expose root discovery through a source for applications that do not use a
/// server when setting up watches.
pub fn source_root(source: &dyn AssetSource) -> Option<&Path> {
    source.watch_root()
}

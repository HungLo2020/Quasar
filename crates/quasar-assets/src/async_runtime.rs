//! Optional background loading on top of [`AssetServer`].
//!
//! The worker owns the server and processes jobs serially, which keeps loader
//! and cache mutations deterministic. Requests have a bounded job queue and a
//! one-result channel; cancellation is advisory for an already-running loader
//! and effective for queued work. No async executor or runtime is required.
use crate::{
    AssetError, AssetErrorKind, AssetLoader, AssetPath, AssetServer, Handle, InvalidationReport,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

type Job = Box<dyn FnOnce(&mut AssetServer, &AtomicU64) + Send + 'static>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncConfig {
    /// Maximum queued jobs; each in-flight job has a one-result channel.
    pub queue_capacity: usize,
}
impl Default for AsyncConfig {
    fn default() -> Self {
        Self { queue_capacity: 64 }
    }
}

/// A loaded asset plus the server revision and any paths invalidated by a
/// reload job.
#[derive(Debug)]
pub struct AsyncLoaded<T> {
    pub handle: Handle<T>,
    pub revision: u64,
    pub invalidated: Vec<AssetPath>,
}
impl<T> AsyncLoaded<T> {
    /// True when a later invalidation/reload has superseded this result. The
    /// value remains usable and memory-safe; the caller decides whether to
    /// replace its own state.
    pub fn is_stale(&self, current_revision: u64) -> bool {
        self.revision < current_revision
    }
}

/// Typed handle for one asynchronous load. Dropping cancels queued work.
pub struct AsyncRequest<T> {
    id: u64,
    path: AssetPath,
    submitted_revision: u64,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<Result<AsyncLoaded<T>, AssetError>>,
}
impl<T> AsyncRequest<T> {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn path(&self) -> &AssetPath {
        &self.path
    }
    pub fn submitted_revision(&self) -> u64 {
        self.submitted_revision
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    /// Non-blocking result check. `None` means still pending or disconnected.
    pub fn try_take(&mut self) -> Option<Result<AsyncLoaded<T>, AssetError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(AssetError::new(
                AssetErrorKind::Cancelled,
                "async request disconnected",
            ))),
        }
    }
    /// Waits up to `timeout`; `None` means still pending.
    pub fn wait(&mut self, timeout: Duration) -> Option<Result<AsyncLoaded<T>, AssetError>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(result) => Some(result),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Err(AssetError::new(
                AssetErrorKind::Cancelled,
                "async request disconnected",
            ))),
        }
    }
}
impl<T> Drop for AsyncRequest<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Typed invalidation request.
pub struct AsyncInvalidation {
    id: u64,
    path: AssetPath,
    receiver: Receiver<Result<InvalidationReport, AssetError>>,
}
impl AsyncInvalidation {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn path(&self) -> &AssetPath {
        &self.path
    }
    pub fn try_take(&mut self) -> Option<Result<InvalidationReport, AssetError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(AssetError::new(
                AssetErrorKind::Cancelled,
                "invalidation request disconnected",
            ))),
        }
    }
    pub fn wait(&mut self, timeout: Duration) -> Option<Result<InvalidationReport, AssetError>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(result) => Some(result),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Err(AssetError::new(
                AssetErrorKind::Cancelled,
                "invalidation request disconnected",
            ))),
        }
    }
}

/// Owns one background asset worker. The synchronous [`AssetServer`] is moved
/// into the worker; build mounts/loaders before calling [`Self::new`].
pub struct AsyncAssetServer {
    jobs: Option<SyncSender<Job>>,
    revision: Arc<AtomicU64>,
    next_request: AtomicU64,
    thread: Option<JoinHandle<()>>,
    queue_capacity: usize,
}
impl AsyncAssetServer {
    pub fn new(server: AssetServer, config: AsyncConfig) -> Result<Self, AssetError> {
        if config.queue_capacity == 0 {
            return Err(AssetError::new(
                AssetErrorKind::Configuration,
                "async queue_capacity must be greater than zero",
            ));
        }
        let (jobs, receiver) = sync_channel::<Job>(config.queue_capacity);
        let revision = Arc::new(AtomicU64::new(server.revision()));
        let worker_revision = Arc::clone(&revision);
        let thread = thread::Builder::new()
            .name("quasar-assets-async".into())
            .spawn(move || {
                let mut server = server;
                while let Ok(job) = receiver.recv() {
                    job(&mut server, &worker_revision);
                    worker_revision.store(server.revision(), Ordering::Release);
                }
            })
            .map_err(|error| {
                AssetError::new(
                    AssetErrorKind::Configuration,
                    format!("spawn asset worker: {error}"),
                )
            })?;
        Ok(Self {
            jobs: Some(jobs),
            revision,
            next_request: AtomicU64::new(1),
            thread: Some(thread),
            queue_capacity: config.queue_capacity,
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }
    /// Enqueues a typed load. Queue saturation is explicit and the worker
    /// remains usable.
    pub fn load<L: AssetLoader>(
        &self,
        path: impl AsRef<str>,
    ) -> Result<AsyncRequest<L::Asset>, AssetError> {
        let path = AssetPath::parse(path)?;
        let id = self.next_request.fetch_add(1, Ordering::Relaxed).max(1);
        let (sender, receiver) = sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_path = path.clone();
        let submitted_revision = self.revision();
        let job: Job = Box::new(move |server, _revision| {
            let result = run_load::<L>(server, &worker_path, &worker_cancelled);
            let _ = sender.send(result);
        });
        self.enqueue(job, &path)?;
        Ok(AsyncRequest {
            id,
            path,
            submitted_revision,
            cancelled,
            receiver,
        })
    }
    /// Invalidates a path (and optionally dependents) then loads it again. The
    /// returned report lists every affected path; dependent assets are removed
    /// from the cache but their existing handles remain valid.
    pub fn reload<L: AssetLoader>(
        &self,
        path: impl AsRef<str>,
        cascade: bool,
    ) -> Result<AsyncRequest<L::Asset>, AssetError> {
        let path = AssetPath::parse(path)?;
        let id = self.next_request.fetch_add(1, Ordering::Relaxed).max(1);
        let (sender, receiver) = sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_path = path.clone();
        let submitted_revision = self.revision();
        let job: Job = Box::new(move |server, _revision| {
            if worker_cancelled.load(Ordering::Acquire) {
                let _ = sender.send(Err(cancelled_error(&worker_path)));
                return;
            }
            let result = server.invalidate(&worker_path, cascade).and_then(|report| {
                if worker_cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error(&worker_path));
                }
                server.load::<L>(&worker_path).map(|handle| AsyncLoaded {
                    revision: handle.revision(),
                    handle,
                    invalidated: report.paths,
                })
            });
            let _ = sender.send(result);
        });
        self.enqueue(job, &path)?;
        Ok(AsyncRequest {
            id,
            path,
            submitted_revision,
            cancelled,
            receiver,
        })
    }
    /// Queues cache invalidation without a typed reload.
    pub fn invalidate(
        &self,
        path: impl AsRef<str>,
        cascade: bool,
    ) -> Result<AsyncInvalidation, AssetError> {
        let path = AssetPath::parse(path)?;
        let id = self.next_request.fetch_add(1, Ordering::Relaxed).max(1);
        let (sender, receiver) = sync_channel(1);
        let worker_path = path.clone();
        let job: Job = Box::new(move |server, _revision| {
            let _ = sender.send(server.invalidate(&worker_path, cascade));
        });
        self.enqueue(job, &path)?;
        Ok(AsyncInvalidation { id, path, receiver })
    }
    fn enqueue(&self, job: Job, path: &AssetPath) -> Result<(), AssetError> {
        let Some(jobs) = &self.jobs else {
            return Err(AssetError::at(
                AssetErrorKind::Cancelled,
                path,
                "asset worker stopped",
            ));
        };
        match jobs.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(AssetError::at(
                AssetErrorKind::QueueFull,
                path,
                "asset worker queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(AssetError::at(
                AssetErrorKind::Cancelled,
                path,
                "asset worker stopped",
            )),
        }
    }
}
impl Drop for AsyncAssetServer {
    fn drop(&mut self) {
        // Dropping the sender wakes recv() and lets queued jobs finish before
        // the worker is joined.
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_load<L: AssetLoader>(
    server: &mut AssetServer,
    path: &AssetPath,
    cancelled: &AtomicBool,
) -> Result<AsyncLoaded<L::Asset>, AssetError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(cancelled_error(path));
    }
    let handle = server.load::<L>(path)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(cancelled_error(path));
    }
    Ok(AsyncLoaded {
        revision: handle.revision(),
        handle,
        invalidated: Vec::new(),
    })
}
fn cancelled_error(path: &AssetPath) -> AssetError {
    AssetError::at(AssetErrorKind::Cancelled, path, "asset request cancelled")
}

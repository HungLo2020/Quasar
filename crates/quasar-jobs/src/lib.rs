//! Optional bounded background task scheduling for Quasar.
//!
//! `quasar-jobs` is a small, independently usable executor with no global pool
//! and no async runtime. It provides:
//!
//! - caller-owned worker pools with configurable worker counts;
//! - bounded outstanding work and explicit [`JobErrorKind::QueueFull`] backpressure;
//! - FIFO/priority ordering within a worker pool;
//! - dependency graphs with failure/cancellation propagation;
//! - cooperative cancellation and panic containment;
//! - a deterministic, single-threaded manual mode for headless tests;
//! - graceful explicit shutdown (drop only signals cancellation and detaches);
//! - per-pool metrics and optional `quasar-diagnostics` events.
//!
//! The pool owns threads but never owns game assets or simulation state.
//! Callers move owned handles into tasks, or share their own `Arc` resources.
//! Jobs are ordinary Rust closures returning `Result<T, E>`.
//!
//! # Example
//!
//! ```
//! use quasar_jobs::JobPool;
//!
//! let pool = JobPool::manual();
//! let a = pool.spawn(|_| Ok::<_, std::convert::Infallible>(2_u32)).unwrap();
//! let dependency = a.dependency();
//! let b = pool
//!     .spawn_with_dependencies(&[dependency], |_| {
//!         Ok::<_, std::convert::Infallible>(5_u32)
//!     })
//!     .unwrap();
//! pool.run_all();
//! assert_eq!(a.wait().unwrap(), 2);
//! assert_eq!(b.wait().unwrap(), 5);
//! ```
//!
//! Tasks are not forced onto a mandatory global thread pool. A caller can use a
//! manual pool in tests or create one pool per subsystem/world with its own
//! limits.

use std::{
    any::Any,
    cell::Cell,
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, VecDeque},
    error::Error,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering as AtomicOrdering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(feature = "diagnostics")]
use quasar_diagnostics::{Diagnostics, Field, Level};

/// Stable identifier for a submitted job.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JobId(u64);
impl JobId {
    pub fn value(self) -> u64 {
        self.0
    }
}

/// Higher priorities are dequeued first; equal priorities retain submission order.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JobPriority(i32);
impl JobPriority {
    pub const LOW: Self = Self(-100);
    pub const NORMAL: Self = Self(0);
    pub const HIGH: Self = Self(100);
    pub fn new(value: i32) -> Self {
        Self(value)
    }
    pub fn value(self) -> i32 {
        self.0
    }
}

/// Lifecycle state of one job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}
impl JobStatus {
    fn as_u8(self) -> u8 {
        match self {
            Self::Queued => 0,
            Self::Running => 1,
            Self::Completed => 2,
            Self::Failed => 3,
            Self::Cancelled => 4,
        }
    }
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Running,
            2 => Self::Completed,
            3 => Self::Failed,
            4 => Self::Cancelled,
            _ => Self::Queued,
        }
    }
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Broad failure category for a job or submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobErrorKind {
    QueueFull,
    Shutdown,
    UnknownDependency,
    DependencyFailed,
    Cancelled,
    Panicked,
    Task,
    InvalidConfig,
    NestedBlocking,
    Internal,
}

/// Failure returned by submission, execution or a dependency.
///
/// Task errors keep their original error as [`Error::source`] when possible.
pub struct JobError {
    kind: JobErrorKind,
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}
impl JobError {
    pub fn new(kind: JobErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }
    pub fn kind(&self) -> JobErrorKind {
        self.kind
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn cancelled() -> Self {
        Self::new(JobErrorKind::Cancelled, "job was cancelled")
    }
    pub fn shutdown() -> Self {
        Self::new(JobErrorKind::Shutdown, "job pool has shut down")
    }
    pub fn queue_full(max: usize) -> Self {
        Self::new(
            JobErrorKind::QueueFull,
            format!("job queue is full (max outstanding {max})"),
        )
    }
    pub fn dependency_failed(id: JobId) -> Self {
        Self::new(
            JobErrorKind::DependencyFailed,
            format!("dependency {} did not complete successfully", id.0),
        )
    }
    pub fn unknown_dependency(id: JobId) -> Self {
        Self::new(
            JobErrorKind::UnknownDependency,
            format!("dependency {} is not owned by this pool", id.0),
        )
    }
    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::new(JobErrorKind::InvalidConfig, message)
    }
    pub fn nested_blocking(message: impl Into<String>) -> Self {
        Self::new(JobErrorKind::NestedBlocking, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(JobErrorKind::Internal, message)
    }
    fn task<E: Error + Send + Sync + 'static>(error: E) -> Self {
        Self {
            kind: JobErrorKind::Task,
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }
    fn panic(payload: Box<dyn Any + Send>) -> Self {
        let message = if let Some(message) = payload.downcast_ref::<&str>() {
            (*message).to_owned()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            "task panicked with a non-string payload".to_owned()
        };
        Self::new(JobErrorKind::Panicked, format!("job panicked: {message}"))
    }
}
impl fmt::Debug for JobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("source", &self.source.as_ref().map(ToString::to_string))
            .finish()
    }
}
impl fmt::Display for JobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl Error for JobError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

/// Snapshot of pool counters. `queued` includes dependency-waiting jobs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JobMetrics {
    pub workers: usize,
    pub max_outstanding: usize,
    pub outstanding: usize,
    pub queued: usize,
    pub running: usize,
    pub submitted: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub panicked: u64,
    pub peak_outstanding: usize,
}

/// Pool configuration. `workers == 0` creates a deterministic manual pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobPoolConfig {
    pub workers: usize,
    pub max_outstanding: usize,
}
impl JobPoolConfig {
    pub fn new(workers: usize, max_outstanding: usize) -> Self {
        Self {
            workers,
            max_outstanding,
        }
    }
    pub fn manual() -> Self {
        Self::new(0, 1024)
    }
}
impl Default for JobPoolConfig {
    fn default() -> Self {
        let workers = thread::available_parallelism().map_or(1, |value| value.get());
        Self::new(workers, 1024)
    }
}

/// Context available to a running task.
#[derive(Clone, Debug)]
pub struct JobContext {
    id: JobId,
    cancel: Arc<AtomicBool>,
}
impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }
    /// Cooperative cancellation flag. Blocking tasks should poll this or split
    /// work into smaller jobs.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(AtomicOrdering::Acquire)
    }
    pub fn check_cancelled(&self) -> Result<(), JobError> {
        if self.is_cancelled() {
            Err(JobError::cancelled())
        } else {
            Ok(())
        }
    }
}

/// Handle for one submitted job. Dropping the handle does not cancel the job.
pub struct JobHandle<T> {
    inner: Arc<Inner>,
    id: JobId,
    receiver: Receiver<ErasedOutcome>,
    cancel: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
    taken: AtomicBool,
    _result: std::marker::PhantomData<fn() -> T>,
}
impl<T> JobHandle<T> {
    pub fn id(&self) -> JobId {
        self.id
    }
    pub fn status(&self) -> JobStatus {
        JobStatus::from_u8(self.status.load(AtomicOrdering::Acquire))
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(AtomicOrdering::Acquire)
    }
    /// Requests cancellation. Queued jobs are finished immediately; running
    /// jobs observe [`JobContext::is_cancelled`] cooperatively.
    pub fn cancel(&self) {
        self.inner.cancel(self.id);
    }
    /// Dependency token accepted by [`JobPool::spawn_with_dependencies`].
    pub fn dependency(&self) -> JobDependency {
        JobDependency {
            owner: Arc::clone(&self.inner),
            id: self.id,
            status: Arc::clone(&self.status),
        }
    }
}
impl<T: Send + 'static> JobHandle<T> {
    /// Waits for completion and returns the value or failure.
    ///
    /// A worker of the owning pool cannot block on a job in that same pool;
    /// it receives [`JobErrorKind::NestedBlocking`] after trying an already
    /// completed result. Use another pool or return the handle to the caller.
    pub fn wait(&self) -> Result<T, JobError> {
        if self.inner.is_worker_thread() {
            if let Some(result) = self.try_take() {
                return result;
            }
            return Err(JobError::nested_blocking(
                "a worker cannot block waiting for a job in its own pool",
            ));
        }
        if self.taken.load(AtomicOrdering::Acquire) {
            return Err(JobError::internal("job result has already been taken"));
        }
        let outcome = self
            .receiver
            .recv()
            .map_err(|_| JobError::internal("job completion channel disconnected"))?;
        self.taken.store(true, AtomicOrdering::Release);
        decode::<T>(outcome)
    }
    /// Waits at most `timeout`. `None` means the job is still running.
    /// Same-pool workers receive an immediate `NestedBlocking` error rather
    /// than blocking.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Result<T, JobError>> {
        if self.inner.is_worker_thread() {
            return Some(self.try_take().unwrap_or_else(|| {
                Err(JobError::nested_blocking(
                    "a worker cannot block waiting for a job in its own pool",
                ))
            }));
        }
        if self.taken.load(AtomicOrdering::Acquire) {
            return None;
        }
        match self.receiver.recv_timeout(timeout) {
            Ok(outcome) => {
                self.taken.store(true, AtomicOrdering::Release);
                Some(decode::<T>(outcome))
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Err(JobError::internal(
                "job completion channel disconnected",
            ))),
        }
    }
    /// Non-blocking completion check.
    pub fn try_take(&self) -> Option<Result<T, JobError>> {
        if self.taken.load(AtomicOrdering::Acquire) {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(outcome) => {
                self.taken.store(true, AtomicOrdering::Release);
                Some(decode::<T>(outcome))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(JobError::internal(
                "job completion channel disconnected",
            ))),
        }
    }
}
impl<T> fmt::Debug for JobHandle<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobHandle")
            .field("id", &self.id)
            .field("status", &self.status())
            .finish()
    }
}

/// A dependency on another job. Obtain one from [`JobHandle::dependency`].
#[derive(Clone)]
pub struct JobDependency {
    owner: Arc<Inner>,
    id: JobId,
    status: Arc<AtomicU8>,
}
impl fmt::Debug for JobDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobDependency")
            .field("id", &self.id)
            .field("status", &self.status())
            .finish()
    }
}
impl JobDependency {
    pub fn id(&self) -> JobId {
        self.id
    }
    pub fn status(&self) -> JobStatus {
        JobStatus::from_u8(self.status.load(AtomicOrdering::Acquire))
    }
}
impl<T> From<&JobHandle<T>> for JobDependency {
    fn from(handle: &JobHandle<T>) -> Self {
        handle.dependency()
    }
}

type ErasedOutcome = Result<Box<dyn Any + Send>, JobError>;
type Task = Box<dyn FnOnce(&JobContext) -> ErasedOutcome + Send>;

/// Owns a bounded worker set. Create one per subsystem/world; no pool is global.
pub struct JobPool {
    inner: Arc<Inner>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
impl JobPool {
    /// Starts a threaded pool. With `workers == 0`, jobs are executed only by
    /// [`Self::run_next`]/[`Self::run_all`] on the caller thread.
    pub fn new(config: JobPoolConfig) -> Result<Self, JobError> {
        Self::start(config, None)
    }
    /// Deterministic single-threaded pool for headless tests and diagnostics.
    pub fn manual() -> Self {
        Self::new(JobPoolConfig::manual()).expect("manual job pool config is valid")
    }
    #[cfg(feature = "diagnostics")]
    pub fn with_diagnostics(
        config: JobPoolConfig,
        diagnostics: Diagnostics,
    ) -> Result<Self, JobError> {
        Self::start(config, Some(diagnostics))
    }
    fn start(
        config: JobPoolConfig,
        #[cfg(feature = "diagnostics")] diagnostics: Option<Diagnostics>,
        #[cfg(not(feature = "diagnostics"))] _diagnostics: Option<()>,
    ) -> Result<Self, JobError> {
        if config.max_outstanding == 0 {
            return Err(JobError::invalid_config(
                "max_outstanding must be greater than zero",
            ));
        }
        let inner = Arc::new(Inner {
            id: NEXT_POOL_ID.fetch_add(1, AtomicOrdering::Relaxed),
            config,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            #[cfg(feature = "diagnostics")]
            diagnostics,
        });
        let mut workers = Vec::with_capacity(config.workers);
        for index in 0..config.workers {
            let worker_inner = Arc::clone(&inner);
            match thread::Builder::new()
                .name(format!("quasar-jobs-{index}"))
                .spawn(move || worker_loop(worker_inner))
            {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    inner.begin_shutdown(false);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(JobError::internal(format!(
                        "failed to spawn job worker: {error}"
                    )));
                }
            }
        }
        Ok(Self {
            inner,
            workers: Mutex::new(workers),
        })
    }
    pub fn is_manual(&self) -> bool {
        self.inner.config.workers == 0
    }
    pub fn worker_count(&self) -> usize {
        self.inner.config.workers
    }
    pub fn max_outstanding(&self) -> usize {
        self.inner.config.max_outstanding
    }
    pub fn metrics(&self) -> JobMetrics {
        let state = self.inner.lock();
        state.metrics(self.inner.config)
    }
    /// Submits a job without blocking. Returns [`JobErrorKind::QueueFull`] when
    /// the configured outstanding limit is reached.
    pub fn spawn<T, E, F>(&self, task: F) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
        F: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
    {
        self.submit(JobPriority::NORMAL, &[], task, false)
    }
    /// Submits with a priority; higher values run first.
    pub fn spawn_with_priority<T, E, F>(
        &self,
        priority: JobPriority,
        task: F,
    ) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
        F: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
    {
        self.submit(priority, &[], task, false)
    }
    /// Submits behind dependencies. A failed/cancelled dependency fails the job;
    /// a completed dependency is treated as satisfied.
    pub fn spawn_with_dependencies<T, E, F>(
        &self,
        dependencies: &[JobDependency],
        task: F,
    ) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
        F: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
    {
        self.submit(JobPriority::NORMAL, dependencies, task, false)
    }
    /// Like [`Self::spawn`], but waits for outstanding capacity instead of
    /// returning [`JobErrorKind::QueueFull`].
    ///
    /// If a worker of this same pool calls this while the pool is full, it
    /// returns [`JobErrorKind::NestedBlocking`] rather than consuming the
    /// worker while waiting for work that only that worker could run.
    pub fn spawn_wait<T, E, F>(&self, task: F) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
        F: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
    {
        self.submit(JobPriority::NORMAL, &[], task, true)
    }
    fn submit<T, E, F>(
        &self,
        priority: JobPriority,
        dependencies: &[JobDependency],
        task: F,
        wait: bool,
    ) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
        F: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
    {
        let (sender, receiver) = sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let status = Arc::new(AtomicU8::new(JobStatus::Queued.as_u8()));
        let wrapped: Task = Box::new(move |context| {
            task(context)
                .map(|value| Box::new(value) as Box<dyn Any + Send>)
                .map_err(JobError::task)
        });

        let mut state = self.inner.lock();
        loop {
            if state.shutdown {
                return Err(JobError::shutdown());
            }
            if state.jobs.len() < self.inner.config.max_outstanding {
                break;
            }
            if self.inner.is_worker_thread() {
                return Err(JobError::nested_blocking(
                    "a worker cannot block waiting for capacity in its own job pool",
                ));
            }
            if !wait || self.is_manual() {
                return Err(JobError::queue_full(self.inner.config.max_outstanding));
            }
            state = self
                .inner
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }

        let mut blocking = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            if !Arc::ptr_eq(&dependency.owner, &self.inner) {
                return Err(JobError::unknown_dependency(dependency.id));
            }
            match dependency.status() {
                JobStatus::Completed => {}
                JobStatus::Failed | JobStatus::Cancelled => {
                    return Err(JobError::dependency_failed(dependency.id));
                }
                JobStatus::Queued | JobStatus::Running => {
                    if !state.jobs.contains_key(&dependency.id) {
                        return Err(JobError::unknown_dependency(dependency.id));
                    }
                    blocking.push(dependency.id);
                }
            }
        }

        let id = JobId(state.next_id);
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| JobError::internal("job id space exhausted"))?;
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        let job = Job {
            deps: blocking.len(),
            dependents: Vec::new(),
            priority,
            sequence,
            status: JobStatus::Queued,
            cancel: Arc::clone(&cancel),
            status_cell: Arc::clone(&status),
            task: Some(wrapped),
            completion: Some(sender),
        };
        state.jobs.insert(id, job);
        for dependency in blocking {
            if let Some(job) = state.jobs.get_mut(&dependency) {
                job.dependents.push(id);
            }
        }
        if state.jobs.get(&id).is_some_and(|job| job.deps == 0) {
            state.ready.push(ReadyJob {
                priority,
                sequence,
                id,
            });
        }
        state.submitted = state.submitted.saturating_add(1);
        state.peak_outstanding = state.peak_outstanding.max(state.jobs.len());
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.inner.diagnostics {
            diagnostics.event_with(
                Level::Debug,
                "quasar.jobs",
                "submitted",
                [
                    Field::new("job", id.0),
                    Field::new("priority", i64::from(priority.0)),
                    Field::new("queued", state.jobs.len().saturating_sub(state.running)),
                ],
            );
        }
        drop(state);
        self.inner.wake.notify_all();
        Ok(JobHandle {
            inner: Arc::clone(&self.inner),
            id,
            receiver,
            cancel,
            status,
            taken: AtomicBool::new(false),
            _result: std::marker::PhantomData,
        })
    }
    /// Executes one ready job on the caller thread. Returns `None` when no job
    /// is ready or the pool has no runnable work. Intended for manual pools.
    pub fn run_next(&self) -> Option<JobId> {
        let (id, task, cancel) = self.inner.take_ready(false)?;
        self.inner.execute(id, task, cancel);
        Some(id)
    }
    /// Drains all currently-ready manual work, including jobs spawned by tasks.
    pub fn run_all(&self) -> usize {
        let mut executed = 0;
        while self.run_next().is_some() {
            executed += 1;
        }
        executed
    }
    /// Requests cancellation of queued and running jobs.
    pub fn cancel_all(&self) {
        self.inner.cancel_all();
    }
    /// Stops accepting work and waits for queued/running jobs to finish.
    /// Manual pools cancel queued work because no worker can execute it.
    /// A worker of this pool receives `NestedBlocking` instead of joining itself.
    pub fn shutdown(&self) -> Result<(), JobError> {
        if self.inner.is_worker_thread() {
            return Err(JobError::nested_blocking(
                "a worker cannot join its own job pool during shutdown",
            ));
        }
        let manual = self.is_manual();
        self.inner.begin_shutdown(manual);
        let workers = {
            let mut workers = self
                .workers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *workers)
        };
        let mut worker_panicked = false;
        for worker in workers {
            if worker.join().is_err() {
                worker_panicked = true;
            }
        }
        if worker_panicked {
            Err(JobError::internal("a job worker panicked"))
        } else {
            Ok(())
        }
    }
    /// Cancels outstanding work and then shuts down.
    pub fn shutdown_cancelling(&self) -> Result<(), JobError> {
        self.cancel_all();
        self.shutdown()
    }
}
impl Drop for JobPool {
    fn drop(&mut self) {
        // Never join here: a task may hold the last Arc<JobPool>, making Drop run
        // on one of this pool's own workers. Signal cancellation safely and let
        // workers exit; explicit shutdown() remains the deterministic teardown.
        self.inner.begin_shutdown(true);
    }
}

struct Inner {
    id: u64,
    config: JobPoolConfig,
    state: Mutex<State>,
    wake: Condvar,
    #[cfg(feature = "diagnostics")]
    diagnostics: Option<Diagnostics>,
}
impl Inner {
    fn is_worker_thread(&self) -> bool {
        WORKER_POOL.with(|current| current.get() == Some(self.id))
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn begin_shutdown(&self, cancel_queued: bool) {
        let mut state = self.lock();
        state.shutdown = true;
        if cancel_queued {
            let ids: Vec<JobId> = state
                .jobs
                .iter()
                .filter(|(_, job)| job.status == JobStatus::Queued)
                .map(|(id, _)| *id)
                .collect();
            for id in ids {
                self.finish_job(
                    &mut state,
                    id,
                    Err(JobError::shutdown()),
                    JobStatus::Cancelled,
                );
            }
        }
        drop(state);
        self.wake.notify_all();
    }
    fn cancel(&self, id: JobId) {
        let mut state = self.lock();
        let queued = if let Some(job) = state.jobs.get(&id) {
            job.cancel.store(true, AtomicOrdering::Release);
            job.status == JobStatus::Queued
        } else {
            false
        };
        if queued {
            self.finish_job(
                &mut state,
                id,
                Err(JobError::cancelled()),
                JobStatus::Cancelled,
            );
        }
        drop(state);
        self.wake.notify_all();
    }
    fn cancel_all(&self) {
        let mut state = self.lock();
        let queued: Vec<JobId> = state
            .jobs
            .iter()
            .filter(|(_, job)| job.status == JobStatus::Queued)
            .map(|(id, _)| *id)
            .collect();
        for id in queued {
            self.finish_job(
                &mut state,
                id,
                Err(JobError::cancelled()),
                JobStatus::Cancelled,
            );
        }
        for job in state.jobs.values() {
            if job.status == JobStatus::Running {
                job.cancel.store(true, AtomicOrdering::Release);
            }
        }
        drop(state);
        self.wake.notify_all();
    }
    fn take_ready(&self, wait: bool) -> Option<(JobId, Task, Arc<AtomicBool>)> {
        let mut state = self.lock();
        loop {
            while let Some(ready) = state.ready.pop() {
                let cancelled = state.jobs.get(&ready.id).is_some_and(|job| {
                    job.status == JobStatus::Queued && job.cancel.load(AtomicOrdering::Acquire)
                });
                if cancelled {
                    self.finish_job(
                        &mut state,
                        ready.id,
                        Err(JobError::cancelled()),
                        JobStatus::Cancelled,
                    );
                    continue;
                }
                let id = ready.id;
                let (task, cancel) = {
                    let Some(job) = state.jobs.get_mut(&id) else {
                        continue;
                    };
                    if job.status != JobStatus::Queued {
                        continue;
                    }
                    let Some(task) = job.task.take() else {
                        continue;
                    };
                    job.status = JobStatus::Running;
                    job.status_cell
                        .store(JobStatus::Running.as_u8(), AtomicOrdering::Release);
                    (task, Arc::clone(&job.cancel))
                };
                state.running = state.running.saturating_add(1);
                return Some((id, task, cancel));
            }
            if state.shutdown && state.jobs.is_empty() {
                return None;
            }
            if state.shutdown && state.running == 0 {
                // No worker can make progress on an orphaned dependency graph.
                let ids: Vec<JobId> = state.jobs.keys().copied().collect();
                for id in ids {
                    self.finish_job(
                        &mut state,
                        id,
                        Err(JobError::shutdown()),
                        JobStatus::Cancelled,
                    );
                }
                continue;
            }
            if !wait {
                return None;
            }
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
    fn execute(&self, id: JobId, task: Task, cancel: Arc<AtomicBool>) {
        let context = JobContext {
            id,
            cancel: Arc::clone(&cancel),
        };
        let result = catch_unwind(AssertUnwindSafe(|| task(&context)));
        let (outcome, status) = match result {
            Ok(outcome) => {
                if cancel.load(AtomicOrdering::Acquire) {
                    (Err(JobError::cancelled()), JobStatus::Cancelled)
                } else if outcome.is_ok() {
                    (outcome, JobStatus::Completed)
                } else {
                    (outcome, JobStatus::Failed)
                }
            }
            Err(payload) => (Err(JobError::panic(payload)), JobStatus::Failed),
        };
        let mut state = self.lock();
        self.finish_job(&mut state, id, outcome, status);
        drop(state);
        self.wake.notify_all();
    }
    fn finish_job(&self, state: &mut State, id: JobId, outcome: ErasedOutcome, status: JobStatus) {
        let mut queue = VecDeque::from([(id, outcome, status)]);
        while let Some((id, outcome, status)) = queue.pop_front() {
            let Some(mut job) = state.jobs.remove(&id) else {
                continue;
            };
            if job.status == JobStatus::Running {
                state.running = state.running.saturating_sub(1);
            }
            job.status = status;
            job.status_cell
                .store(status.as_u8(), AtomicOrdering::Release);
            match status {
                JobStatus::Completed => state.completed = state.completed.saturating_add(1),
                JobStatus::Failed => {
                    state.failed = state.failed.saturating_add(1);
                    if outcome
                        .as_ref()
                        .err()
                        .is_some_and(|error| error.kind() == JobErrorKind::Panicked)
                    {
                        state.panicked = state.panicked.saturating_add(1);
                    }
                }
                JobStatus::Cancelled => state.cancelled = state.cancelled.saturating_add(1),
                JobStatus::Queued | JobStatus::Running => {}
            }
            #[cfg(feature = "diagnostics")]
            if let Some(diagnostics) = &self.diagnostics {
                diagnostics.event_with(
                    Level::Debug,
                    "quasar.jobs",
                    "finished",
                    [
                        Field::new("job", id.0),
                        Field::new("status", format!("{status:?}")),
                        Field::new("queued", state.jobs.len()),
                    ],
                );
            }
            if let Some(sender) = job.completion.take() {
                let _ = sender.send(outcome);
            }
            for dependent in job.dependents {
                let Some(child) = state.jobs.get_mut(&dependent) else {
                    continue;
                };
                if child.status != JobStatus::Queued {
                    continue;
                }
                if status == JobStatus::Completed {
                    if child.deps == 0 {
                        continue;
                    }
                    child.deps -= 1;
                    if child.deps == 0 {
                        if child.cancel.load(AtomicOrdering::Acquire) {
                            queue.push_back((
                                dependent,
                                Err(JobError::cancelled()),
                                JobStatus::Cancelled,
                            ));
                        } else {
                            state.ready.push(ReadyJob {
                                priority: child.priority,
                                sequence: child.sequence,
                                id: dependent,
                            });
                        }
                    }
                } else {
                    queue.push_back((
                        dependent,
                        Err(JobError::dependency_failed(id)),
                        JobStatus::Failed,
                    ));
                }
            }
        }
    }
}

fn worker_loop(inner: Arc<Inner>) {
    WORKER_POOL.with(|current| current.set(Some(inner.id)));
    while let Some((id, task, cancel)) = inner.take_ready(true) {
        inner.execute(id, task, cancel);
    }
    WORKER_POOL.with(|current| current.set(None));
}

thread_local! {
    /// Distinguishes a pool's own workers from unrelated worker/caller threads.
    static WORKER_POOL: Cell<Option<u64>> = const { Cell::new(None) };
}

static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct State {
    shutdown: bool,
    next_id: u64,
    next_sequence: u64,
    jobs: HashMap<JobId, Job>,
    ready: BinaryHeap<ReadyJob>,
    running: usize,
    submitted: u64,
    completed: u64,
    failed: u64,
    cancelled: u64,
    panicked: u64,
    peak_outstanding: usize,
}
impl State {
    fn metrics(&self, config: JobPoolConfig) -> JobMetrics {
        JobMetrics {
            workers: config.workers,
            max_outstanding: config.max_outstanding,
            outstanding: self.jobs.len(),
            queued: self.jobs.len().saturating_sub(self.running),
            running: self.running,
            submitted: self.submitted,
            completed: self.completed,
            failed: self.failed,
            cancelled: self.cancelled,
            panicked: self.panicked,
            peak_outstanding: self.peak_outstanding,
        }
    }
}

struct Job {
    deps: usize,
    dependents: Vec<JobId>,
    priority: JobPriority,
    sequence: u64,
    status: JobStatus,
    cancel: Arc<AtomicBool>,
    status_cell: Arc<AtomicU8>,
    task: Option<Task>,
    completion: Option<SyncSender<ErasedOutcome>>,
}

struct ReadyJob {
    priority: JobPriority,
    sequence: u64,
    id: JobId,
}
impl Eq for ReadyJob {}
impl PartialEq for ReadyJob {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence && self.id == other.id
    }
}
impl Ord for ReadyJob {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}
impl PartialOrd for ReadyJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn decode<T: Send + 'static>(outcome: ErasedOutcome) -> Result<T, JobError> {
    match outcome {
        Ok(value) => value
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| JobError::internal("job result type mismatch")),
        Err(error) => Err(error),
    }
}

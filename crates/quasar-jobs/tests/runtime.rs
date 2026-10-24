use quasar_jobs::{JobErrorKind, JobPool, JobPoolConfig, JobPriority, JobStatus};
use std::{
    error::Error,
    fmt, io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

fn infallible<T>(value: T) -> Result<T, std::convert::Infallible> {
    Ok(value)
}

#[test]
fn manual_dependency_graph_runs_in_order() {
    let pool = JobPool::manual();
    let order = Arc::new(Mutex::new(Vec::new()));
    let a = {
        let order = Arc::clone(&order);
        pool.spawn(move |_| {
            order.lock().unwrap().push("a");
            infallible(1_u32)
        })
        .unwrap()
    };
    let b = {
        let order = Arc::clone(&order);
        pool.spawn_with_dependencies(&[a.dependency()], move |_| {
            order.lock().unwrap().push("b");
            infallible(2_u32)
        })
        .unwrap()
    };
    let c = {
        let order = Arc::clone(&order);
        pool.spawn_with_dependencies(&[a.dependency(), b.dependency()], move |_| {
            order.lock().unwrap().push("c");
            infallible(3_u32)
        })
        .unwrap()
    };
    assert_eq!(pool.run_all(), 3);
    assert_eq!(*order.lock().unwrap(), ["a", "b", "c"]);
    assert_eq!(a.wait().unwrap(), 1);
    assert_eq!(b.wait().unwrap(), 2);
    assert_eq!(c.wait().unwrap(), 3);
    let metrics = pool.metrics();
    assert_eq!(metrics.completed, 3);
    assert_eq!(metrics.outstanding, 0);
}

#[test]
fn priority_then_submission_order_is_stable() {
    let pool = JobPool::manual();
    let order = Arc::new(Mutex::new(Vec::new()));
    for (priority, label) in [
        (JobPriority::LOW, "low"),
        (JobPriority::NORMAL, "normal-1"),
        (JobPriority::HIGH, "high-1"),
        (JobPriority::NORMAL, "normal-2"),
    ] {
        let order = Arc::clone(&order);
        pool.spawn_with_priority(priority, move |_| {
            order.lock().unwrap().push(label);
            infallible(())
        })
        .unwrap();
    }
    pool.run_all();
    assert_eq!(
        *order.lock().unwrap(),
        ["high-1", "normal-1", "normal-2", "low"]
    );
}

#[test]
fn queue_is_bounded_and_spawn_wait_is_manual_safe() {
    let pool = JobPool::new(JobPoolConfig::new(0, 2)).unwrap();
    let a = pool.spawn(|_| infallible(())).unwrap();
    let b = pool.spawn(|_| infallible(())).unwrap();
    let full = pool.spawn(|_| infallible(()));
    assert_eq!(full.unwrap_err().kind(), JobErrorKind::QueueFull);
    assert_eq!(
        pool.spawn_wait(|_| infallible(())).unwrap_err().kind(),
        JobErrorKind::QueueFull
    );
    pool.run_next().unwrap();
    let c = pool.spawn(|_| infallible(())).unwrap();
    pool.run_all();
    a.wait().unwrap();
    b.wait().unwrap();
    c.wait().unwrap();
    assert_eq!(pool.metrics().peak_outstanding, 2);
}

#[test]
fn nested_spawn_wait_on_the_same_full_pool_fails_instead_of_deadlocking() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(1, 1)).unwrap());
    let task_pool = Arc::clone(&pool);
    let handle = pool
        .spawn(move |_| {
            let error = task_pool
                .spawn_wait(|_| Ok::<_, std::convert::Infallible>(()))
                .unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::NestedBlocking);
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
    handle
        .wait_timeout(Duration::from_secs(5))
        .expect("nested submission must return instead of deadlocking")
        .unwrap();
    pool.shutdown().unwrap();
}

#[test]
fn nested_spawn_wait_succeeds_when_capacity_is_available() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(2, 2)).unwrap());
    let task_pool = Arc::clone(&pool);
    let handle = pool
        .spawn(move |_| {
            let child = task_pool
                .spawn_wait(|_| Ok::<_, std::convert::Infallible>(42_u32))
                .unwrap();
            Ok::<_, std::convert::Infallible>(child)
        })
        .unwrap();
    let child = handle
        .wait_timeout(Duration::from_secs(5))
        .expect("nested submission with capacity must complete")
        .unwrap();
    assert_eq!(child.wait().unwrap(), 42);
    pool.shutdown().unwrap();
}

#[test]
fn nested_child_wait_from_a_worker_is_rejected_instead_of_deadlocking() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(1, 4)).unwrap());
    let task_pool = Arc::clone(&pool);
    let handle = pool
        .spawn(move |_| {
            let child = task_pool
                .spawn_wait(|_| Ok::<_, std::convert::Infallible>(7_u32))
                .unwrap();
            let error = child.wait().unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::NestedBlocking);
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
    handle
        .wait_timeout(Duration::from_secs(5))
        .expect("nested wait must return instead of deadlocking")
        .unwrap();
    pool.shutdown().unwrap();
}

#[test]
fn shutdown_from_a_worker_is_rejected_instead_of_self_joining() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(1, 4)).unwrap());
    let task_pool = Arc::clone(&pool);
    let handle = pool
        .spawn(move |_| {
            let error = task_pool.shutdown().unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::NestedBlocking);
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
    handle
        .wait_timeout(Duration::from_secs(5))
        .expect("worker shutdown attempt must return")
        .unwrap();
    pool.shutdown().unwrap();
}

#[test]
fn spawn_wait_blocks_until_capacity_is_available() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(1, 1)).unwrap());
    let (started_sender, started_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let first = pool
        .spawn(move |_| {
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            Ok::<_, std::convert::Infallible>(1_u8)
        })
        .unwrap();
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let (spawned_sender, spawned_receiver) = mpsc::channel();
    let submit_pool = Arc::clone(&pool);
    let submitter = thread::spawn(move || {
        let handle = submit_pool
            .spawn_wait(|_| Ok::<_, std::convert::Infallible>(2_u8))
            .unwrap();
        spawned_sender.send(()).unwrap();
        handle.wait().unwrap()
    });
    assert!(
        spawned_receiver
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    release_sender.send(()).unwrap();
    assert_eq!(first.wait().unwrap(), 1);
    spawned_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(submitter.join().unwrap(), 2);
    pool.shutdown().unwrap();
}

#[test]
fn cancelling_a_queued_job_finishes_it_without_running_it() {
    let pool = JobPool::manual();
    let ran = Arc::new(AtomicUsize::new(0));
    let first = {
        let ran = Arc::clone(&ran);
        pool.spawn(move |_| {
            ran.fetch_add(1, Ordering::SeqCst);
            infallible(1)
        })
        .unwrap()
    };
    let second = {
        let ran = Arc::clone(&ran);
        pool.spawn(move |_| {
            ran.fetch_add(1, Ordering::SeqCst);
            infallible(2)
        })
        .unwrap()
    };
    first.cancel();
    assert_eq!(first.status(), JobStatus::Cancelled);
    assert_eq!(pool.run_all(), 1);
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    let error = first.wait().unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::Cancelled);
    assert_eq!(second.wait().unwrap(), 2);
}

#[test]
fn cancelling_a_running_job_is_cooperative() {
    let pool = JobPool::new(JobPoolConfig::new(1, 8)).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let handle = pool
        .spawn(move |context| {
            started_sender.send(()).unwrap();
            for _ in 0..10_000_000 {
                if context.is_cancelled() {
                    return Ok::<_, std::convert::Infallible>(());
                }
                thread::yield_now();
            }
            panic!("cancellation was not observed");
        })
        .unwrap();
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    handle.cancel();
    let error = handle
        .wait_timeout(Duration::from_secs(5))
        .expect("cancelled job should finish")
        .unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::Cancelled);
    assert_eq!(handle.status(), JobStatus::Cancelled);
    pool.shutdown().unwrap();
}

#[test]
fn panic_is_contained_and_later_jobs_still_run() {
    let pool = JobPool::new(JobPoolConfig::new(1, 8)).unwrap();
    let panic = pool
        .spawn(|_| -> Result<(), std::convert::Infallible> {
            panic!("expected test panic");
        })
        .unwrap();
    assert_eq!(panic.wait().unwrap_err().kind(), JobErrorKind::Panicked);
    let recovered = pool.spawn(|_| infallible(7_u32)).unwrap();
    assert_eq!(recovered.wait().unwrap(), 7);
    let metrics = pool.metrics();
    assert_eq!(metrics.panicked, 1);
    assert_eq!(metrics.failed, 1);
    assert_eq!(metrics.completed, 1);
    pool.shutdown().unwrap();
}

#[derive(Debug)]
struct SourceError;
impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("source")
    }
}
impl Error for SourceError {}

#[derive(Debug)]
struct OuterError(SourceError);
impl fmt::Display for OuterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("outer")
    }
}
impl Error for OuterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn task_errors_keep_their_source_chain() {
    let pool = JobPool::manual();
    let handle = pool
        .spawn(|_| -> Result<(), OuterError> { Err(OuterError(SourceError)) })
        .unwrap();
    pool.run_all();
    let error = handle.wait().unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::Task);
    assert_eq!(error.to_string(), "outer");
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("outer")
    );
    assert_eq!(
        error
            .source()
            .and_then(Error::source)
            .map(ToString::to_string)
            .as_deref(),
        Some("source")
    );
}

#[test]
fn dependency_failure_propagates_without_deadlock() {
    let pool = JobPool::manual();
    let a = pool
        .spawn(|_| -> Result<(), io::Error> { Err(io::Error::other("boom")) })
        .unwrap();
    let b = pool
        .spawn_with_dependencies(&[a.dependency()], |_| infallible(2_u32))
        .unwrap();
    let c = pool
        .spawn_with_dependencies(&[b.dependency()], |_| infallible(3_u32))
        .unwrap();
    pool.run_all();
    assert_eq!(a.wait().unwrap_err().kind(), JobErrorKind::Task);
    assert_eq!(b.wait().unwrap_err().kind(), JobErrorKind::DependencyFailed);
    assert_eq!(c.wait().unwrap_err().kind(), JobErrorKind::DependencyFailed);
    assert_eq!(pool.metrics().failed, 3);
}

#[test]
fn graceful_shutdown_finishes_queued_work_and_rejects_new_jobs() {
    let pool = JobPool::new(JobPoolConfig::new(2, 64)).unwrap();
    let handles: Vec<_> = (0..32)
        .map(|value| pool.spawn(move |_| infallible(value)).unwrap())
        .collect();
    pool.shutdown().unwrap();
    for (expected, handle) in handles.iter().enumerate() {
        assert_eq!(handle.wait().unwrap(), expected);
        assert_eq!(handle.status(), JobStatus::Completed);
    }
    assert_eq!(pool.metrics().completed, 32);
    assert_eq!(
        pool.spawn(|_| infallible(())).unwrap_err().kind(),
        JobErrorKind::Shutdown
    );
}

#[test]
fn graceful_shutdown_waits_for_running_work() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(1, 8)).unwrap());
    let (started_sender, started_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let handle = pool
        .spawn(move |_| {
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            Ok::<_, std::convert::Infallible>(9_u8)
        })
        .unwrap();
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let (done_sender, done_receiver) = mpsc::channel();
    let shutdown_pool = Arc::clone(&pool);
    let shutdown = thread::spawn(move || {
        shutdown_pool.shutdown().unwrap();
        done_sender.send(()).unwrap();
    });
    assert!(
        done_receiver
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    release_sender.send(()).unwrap();
    done_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    shutdown.join().unwrap();
    assert_eq!(handle.wait().unwrap(), 9);
}

#[test]
fn manual_shutdown_cancels_pending_work() {
    let pool = JobPool::manual();
    let handle = pool.spawn(|_| infallible(1_u8)).unwrap();
    pool.shutdown().unwrap();
    assert_eq!(handle.wait().unwrap_err().kind(), JobErrorKind::Shutdown);
}

#[test]
fn cancelling_shutdown_finishes_running_and_queued_jobs() {
    let pool = JobPool::new(JobPoolConfig::new(1, 32)).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let running = pool
        .spawn(move |context| {
            started_sender.send(()).unwrap();
            while !context.is_cancelled() {
                thread::yield_now();
            }
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let queued: Vec<_> = (0..8)
        .map(|_| pool.spawn(|_| infallible(())).unwrap())
        .collect();
    pool.shutdown_cancelling().unwrap();
    assert_eq!(running.wait().unwrap_err().kind(), JobErrorKind::Cancelled);
    for handle in queued {
        assert_eq!(handle.wait().unwrap_err().kind(), JobErrorKind::Cancelled);
    }
}

#[test]
fn threaded_dependency_chain_completes_in_order() {
    let pool = JobPool::new(JobPoolConfig::new(4, 256)).unwrap();
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut previous = None;
    let mut handles = Vec::new();
    for index in 0..100_u32 {
        let order = Arc::clone(&order);
        let handle = if let Some(dependency) = previous {
            pool.spawn_with_dependencies(&[dependency], move |_| {
                order.lock().unwrap().push(index);
                infallible(index)
            })
            .unwrap()
        } else {
            pool.spawn(move |_| {
                order.lock().unwrap().push(index);
                infallible(index)
            })
            .unwrap()
        };
        previous = Some(handle.dependency());
        handles.push(handle);
    }
    for (index, handle) in handles.iter().enumerate() {
        assert_eq!(handle.wait().unwrap(), index as u32);
    }
    let order = order.lock().unwrap().clone();
    assert_eq!(order, (0..100).collect::<Vec<_>>());
    pool.shutdown().unwrap();
}

#[test]
fn concurrent_submission_and_metrics_are_consistent() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(4, 32)).unwrap());
    let executed = Arc::new(AtomicUsize::new(0));
    let mut submitters = Vec::new();
    for _ in 0..8 {
        let pool = Arc::clone(&pool);
        let executed = Arc::clone(&executed);
        submitters.push(thread::spawn(move || {
            for _ in 0..50 {
                let executed = Arc::clone(&executed);
                let handle = pool
                    .spawn_wait(move |_| {
                        executed.fetch_add(1, Ordering::SeqCst);
                        infallible(())
                    })
                    .unwrap();
                handle.wait().unwrap();
            }
        }));
    }
    for submitter in submitters {
        submitter.join().unwrap();
    }
    assert_eq!(executed.load(Ordering::SeqCst), 400);
    let metrics = pool.metrics();
    assert_eq!(metrics.submitted, 400);
    assert_eq!(metrics.completed, 400);
    assert_eq!(metrics.outstanding, 0);
    assert!(metrics.peak_outstanding <= 32);
    pool.shutdown().unwrap();
}

#[test]
fn dependencies_cannot_cross_pool_boundaries() {
    let first = JobPool::manual();
    let second = JobPool::manual();
    let handle = first.spawn(|_| infallible(())).unwrap();
    let error = second
        .spawn_with_dependencies(&[handle.dependency()], |_| infallible(()))
        .unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::UnknownDependency);
    first.run_all();
    handle.wait().unwrap();
}

#[test]
fn a_task_holding_the_last_pool_reference_does_not_self_deadlock() {
    let pool = Arc::new(JobPool::new(JobPoolConfig::new(1, 4)).unwrap());
    let task_pool = Arc::clone(&pool);
    let (started_sender, started_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let handle = pool
        .spawn(move |_| {
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            drop(task_pool);
            Ok::<_, std::convert::Infallible>(3_u8)
        })
        .unwrap();
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    drop(pool);
    release_sender.send(()).unwrap();
    assert_eq!(
        handle
            .wait_timeout(Duration::from_secs(5))
            .expect("dropping the last pool reference must not self-join")
            .unwrap(),
        3
    );
}

#[test]
fn waiting_for_an_exhausted_handle_is_not_a_deadlock() {
    let pool = JobPool::manual();
    let handle = pool.spawn(|_| infallible(1_u8)).unwrap();
    pool.run_all();
    assert_eq!(handle.try_take().map(Result::unwrap), Some(1));
    assert!(handle.try_take().is_none());
    assert!(handle.wait_timeout(Duration::from_millis(1)).is_none());
}

#[cfg(feature = "diagnostics")]
#[test]
fn diagnostics_receive_job_events() {
    use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, MemorySink};
    let sink = Arc::new(MemorySink::new(32));
    let configuration = DiagnosticsConfig {
        min_level: Level::Debug,
        ..DiagnosticsConfig::default()
    };
    let pool = JobPool::with_diagnostics(
        JobPoolConfig::manual(),
        Diagnostics::with_config(sink.clone(), configuration),
    )
    .unwrap();
    let handle = pool.spawn(|_| infallible(1)).unwrap();
    pool.run_all();
    handle.wait().unwrap();
    let records = sink.records();
    assert!(records.iter().any(|record| record.name == "submitted"));
    assert!(records.iter().any(|record| record.name == "finished"));
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn priority_and_sequence_order_is_total(
            priorities in prop::collection::vec(-20_i32..=20_i32, 0..40)
        ) {
            let pool = JobPool::manual();
            let order = Arc::new(Mutex::new(Vec::new()));
            let mut expected: Vec<(i32, usize)> = priorities
                .iter()
                .copied()
                .enumerate()
                .map(|(index, priority)| (priority, index))
                .collect();
            expected.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
            for (index, priority) in priorities.iter().copied().enumerate() {
                let order = Arc::clone(&order);
                pool.spawn_with_priority(JobPriority::new(priority), move |_| {
                    order.lock().unwrap().push(index);
                    infallible(())
                })
                .unwrap();
            }
            pool.run_all();
            let actual = order.lock().unwrap().clone();
            let expected: Vec<usize> = expected.into_iter().map(|(_, index)| index).collect();
            prop_assert_eq!(actual, expected);
        }
    }
}

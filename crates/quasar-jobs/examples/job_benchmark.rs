//! Local scheduling benchmark; not a statistically rigorous benchmark.
//!
//! ```sh
//! cargo run --release -p quasar-jobs --example job_benchmark
//! ```
use quasar_jobs::{JobPool, JobPoolConfig};
use std::{
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const TASKS: usize = 2_000;
const WORK: u64 = 40_000;

fn work() -> u64 {
    let mut value: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..WORK {
        value = value.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
    black_box(value)
}

fn run(workers: usize) {
    let pool = JobPool::new(JobPoolConfig::new(workers, TASKS)).unwrap();
    let start = Instant::now();
    let latency_total = Arc::new(AtomicU64::new(0));
    let latency_max = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::with_capacity(TASKS);
    for _ in 0..TASKS {
        let submitted = Instant::now();
        let latency_total = Arc::clone(&latency_total);
        let latency_max = Arc::clone(&latency_max);
        handles.push(
            pool.spawn(move |_| {
                let latency = submitted.elapsed().as_nanos() as u64;
                latency_total.fetch_add(latency, Ordering::Relaxed);
                latency_max.fetch_max(latency, Ordering::Relaxed);
                Ok::<_, std::convert::Infallible>(work())
            })
            .unwrap(),
        );
    }
    let mut total = 0u64;
    for handle in &handles {
        total = total.wrapping_add(handle.wait().unwrap());
    }
    let elapsed = start.elapsed();
    println!(
        "workers={workers:<2} tasks={TASKS} elapsed={:>8.3} ms  throughput={:>8.1} tasks/s  queue_avg={:>7.1} us  queue_max={:>7.1} us  checksum={total:016x}",
        elapsed.as_secs_f64() * 1_000.0,
        TASKS as f64 / elapsed.as_secs_f64(),
        latency_total.load(Ordering::Relaxed) as f64 / TASKS as f64 / 1_000.0,
        latency_max.load(Ordering::Relaxed) as f64 / 1_000.0,
    );
    pool.shutdown().unwrap();
}

fn main() {
    println!("work units/task={WORK}");
    for workers in [1, 2, 4, 8] {
        run(workers);
    }
    let manual = JobPool::new(JobPoolConfig::new(0, TASKS)).unwrap();
    let start = Instant::now();
    let mut handles = Vec::with_capacity(TASKS);
    for _ in 0..TASKS {
        handles.push(
            manual
                .spawn(|_| Ok::<_, std::convert::Infallible>(work()))
                .unwrap(),
        );
    }
    manual.run_all();
    let elapsed = start.elapsed();
    println!(
        "manual  tasks={TASKS} elapsed={:>8.3} ms  throughput={:>8.1} tasks/s",
        elapsed.as_secs_f64() * 1_000.0,
        TASKS as f64 / elapsed.as_secs_f64(),
    );
    for handle in handles {
        handle.wait().unwrap();
    }
    manual.shutdown().unwrap();
    black_box(Duration::ZERO);
}

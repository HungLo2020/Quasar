//! Local record/replay throughput probe; not a statistically rigorous benchmark.
//!
//! ```sh
//! cargo run --release -p quasar-sim --example sim_benchmark
//! ```
use quasar_sim::{Recorder, Replay, Simulation, SimulationConfig, StateHash, StateHasher, Tick};
use std::{
    hint::black_box,
    mem::size_of,
    time::{Duration, Instant},
};

const TICKS: u64 = 500_000;

#[derive(Clone, Copy)]
struct World {
    value: u64,
    ticks: u64,
}
impl Default for World {
    fn default() -> Self {
        Self {
            value: 0x9e37_79b9_7f4a_7c15,
            ticks: 0,
        }
    }
}
impl Simulation for World {
    type Input = u64;
    type Snapshot = (u64, u64);
    type Error = std::convert::Infallible;

    fn update(&mut self, _: Tick, input: &u64) -> Result<(), Self::Error> {
        self.value = self
            .value
            .wrapping_mul(6364136223846793005)
            .wrapping_add(*input);
        self.ticks += 1;
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        let mut hasher = StateHasher::new();
        hasher.write_u64(self.value);
        hasher.write_u64(self.ticks);
        hasher.finish()
    }
    fn snapshot(&self) -> Self::Snapshot {
        (self.value, self.ticks)
    }
    fn restore(&mut self, snapshot: &Self::Snapshot) -> Result<(), Self::Error> {
        (self.value, self.ticks) = *snapshot;
        Ok(())
    }
}

fn main() {
    let config = SimulationConfig::new(1, 7, "bench");
    let start = Instant::now();
    let mut recorder = Recorder::new(World::default(), config);
    for tick in 0..TICKS {
        recorder.tick(tick).unwrap();
    }
    let record = start.elapsed();
    println!(
        "record {:>9.3} ms  {:>9.0} ticks/s  journal={} records",
        record.as_secs_f64() * 1_000.0,
        TICKS as f64 / record.as_secs_f64(),
        recorder.journal().len()
    );

    println!(
        "record size={} bytes  approximate journal payload={} KiB",
        size_of::<quasar_sim::TickRecord<u64>>(),
        recorder.journal().len() * size_of::<quasar_sim::TickRecord<u64>>() / 1024
    );
    let journal = recorder.journal().clone();
    let start = Instant::now();
    let mut replay = Replay::new(World::default(), journal).unwrap();
    let summary = replay.run_to_end().unwrap();
    let replayed = start.elapsed();
    println!(
        "replay {:>9.3} ms  {:>9.0} ticks/s  ticks={} hash={}",
        replayed.as_secs_f64() * 1_000.0,
        TICKS as f64 / replayed.as_secs_f64(),
        summary.ticks,
        summary.last_hash
    );

    let snapshot = recorder.snapshot();
    let start = Instant::now();
    for _ in 0..100_000 {
        recorder.restore(&snapshot).unwrap();
        black_box(recorder.current_hash());
    }
    let restore = start.elapsed();
    println!(
        "restore {:>9.3} ms  {:>9.0} restores/s",
        restore.as_secs_f64() * 1_000.0,
        100_000.0 / restore.as_secs_f64()
    );
    black_box(Duration::ZERO);
}

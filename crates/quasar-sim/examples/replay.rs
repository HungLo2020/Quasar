//! Headless record, replay, seek and divergence demonstration.
//!
//! ```sh
//! cargo run -p quasar-sim --example replay
//! ```
use quasar_sim::{
    Journal, Recorder, Replay, ReplayError, Simulation, SimulationConfig, StateHash, StateHasher,
    Tick,
};

#[derive(Clone, Copy, Default)]
struct Counter {
    value: i64,
    ticks: u64,
}
impl Simulation for Counter {
    type Input = i64;
    type Snapshot = (i64, u64);
    type Error = std::convert::Infallible;

    fn update(&mut self, _: Tick, input: &i64) -> Result<(), Self::Error> {
        self.value = self.value.wrapping_add(*input);
        self.ticks += 1;
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        let mut hasher = StateHasher::new();
        hasher.write_i64(self.value);
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SimulationConfig::new(1, 0x5eed, "counter-demo");
    let mut recorder = Recorder::new(Counter::default(), config.clone());
    for tick in 1..=12 {
        recorder.tick(i64::from(tick % 5) + 1)?;
    }
    println!(
        "recorded {} ticks, final tick {}, hash {}",
        recorder.journal().len(),
        recorder.current_tick(),
        recorder.current_hash()
    );

    let mut replay = Replay::new(Counter::default(), recorder.journal().clone())?;
    let summary = replay.run_to_end()?;
    println!(
        "replayed {} ticks exactly, final hash {}",
        summary.ticks, summary.last_hash
    );

    replay.seek(Tick::new(4))?;
    println!(
        "seek(4) -> tick {}, hash {}; {} records remain",
        replay.current_tick(),
        replay.current_hash(),
        replay.remaining()
    );

    let mut tampered = recorder.journal().records().to_vec();
    tampered[2].hash = StateHash::new(0xdead_beef);
    let tampered = Journal::new(config, tampered)?;
    let mut replay = Replay::new(Counter::default(), tampered)?;
    match replay.run_to_end() {
        Err(ReplayError::Divergence(divergence)) => {
            println!("injected divergence: {divergence}");
            assert_eq!(divergence.tick, Tick::new(3));
            assert_eq!(divergence.input_index, 2);
        }
        other => panic!("expected divergence, got {other:?}"),
    }
    Ok(())
}

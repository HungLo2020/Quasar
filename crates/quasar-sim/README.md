# quasar-sim

Optional deterministic simulation coordination for Quasar: tick numbering,
recording, replay verification, snapshots/rollback and divergence diagnostics.
It owns no game state, ECS, renderer, window, network stack or application loop.

```toml
quasar-sim = { path = "../Quasar/crates/quasar-sim" }
```

A game implements `Simulation` with its own input, snapshot and hash types:

```rust
use quasar_sim::{Recorder, Simulation, SimulationConfig, StateHash, Tick};

struct Game { score: u64 }
impl Simulation for Game {
    type Input = u8;
    type Snapshot = u64;
    type Error = std::convert::Infallible;
    fn update(&mut self, _: Tick, input: &u8) -> Result<(), Self::Error> {
        self.score += u64::from(*input);
        Ok(())
    }
    fn state_hash(&self) -> StateHash { StateHash::new(self.score) }
    fn snapshot(&self) -> u64 { self.score }
    fn restore(&mut self, state: &u64) -> Result<(), Self::Error> {
        self.score = *state;
        Ok(())
    }
}

let mut recorder = Recorder::new(
    Game { score: 0 },
    SimulationConfig::new(1, 0x5eed, "level-1"),
);
recorder.tick(3)?;
let journal = recorder.journal().clone();
let mut replay = quasar_sim::Replay::new(Game { score: 0 }, journal)?;
let summary = replay.run_to_end()?;
assert_eq!(summary.ticks, 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## APIs

- `Simulation`: game-owned `update`, `state_hash`, `snapshot`, `restore`.
- `Recorder`: applies and records inputs with per-tick hashes.
- `ScheduledSimulation`: reuses `quasar_core::FixedStep` for time-to-tick
  scheduling; `advance_ticks` is the exact headless path.
- `Journal`: validated consecutive input/hash records.
- `Replay`: exact reapplication, hash comparison, seeking, exhaustion errors and
  first-divergence reports.
- `Snapshot`: caller-owned game snapshot plus tick, hash and simulation config.
- `StateHasher`: small platform-stable FNV-1a helper.
- `Divergence`: first mismatch with tick, input index, expected/actual hashes and
  timeline metadata; serializable with the `serde` feature.

## Features

- `serde`: structured journal, summary and divergence serialization.
- `jobs`: optional `quasar-jobs` replay offload (`Replay::into_jobs`).

## Verification

```sh
cargo test -p quasar-sim
cargo test -p quasar-sim --features serde,jobs
cargo clippy -p quasar-sim --all-targets -- -D warnings
cargo clippy -p quasar-sim --all-targets --features serde,jobs -- -D warnings
cargo run -p quasar-sim --example replay
cargo run --release -p quasar-sim --example sim_benchmark
```

Tests cover exact record/replay, empty and exhausted journals, first-tick
divergence injection, snapshot restoration and journal truncation, snapshot
lineage/config mismatch, version/timeline/seed mismatch, seeking, invalid
ordering, simulation error propagation, FixedStep scheduling, serializer
round-trips and property-based random inputs/seeks. The headless example records,
replays and reports an injected divergence.

## Limitations

- Determinism depends entirely on the game's state, input ordering, RNG seed and
  `state_hash` implementation. Cross-CPU floating-point bitwise parity is not
  claimed.
- Seeking replays forward from the initial snapshot; keyframes/indexes are not
  built automatically.
- Journal records clone inputs per replayed tick. Large inputs should be handles
  or compact event types.
- `StateHasher` is non-cryptographic; use the game's own stable hash if needed.
- Snapshots are game-owned and not automatically portable; serialization and
  versioning of the snapshot representation are caller responsibilities.
- The scheduler supplies tick coordination only. Render pacing and window/input
  policy remain independent (`quasar-app`, `quasar-core`, or the game).

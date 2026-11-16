//! The shared deterministic RNG inside recorded, replayed and restored
//! simulations.
use quasar_sim::{
    Recorder, Replay, ReplayError, Rng, Simulation, SimulationConfig, StateHash, StateHasher, Tick,
};
use std::convert::Infallible;

/// Spawns loot from a seeded stream; the RNG lives in the game state.
#[derive(Clone)]
struct Loot {
    rng: Rng,
    gold: u64,
    drops: Vec<u32>,
}

impl Loot {
    fn new(config: &SimulationConfig) -> Self {
        Self {
            rng: config.rng_for("loot"),
            gold: 0,
            drops: Vec::new(),
        }
    }
}

impl Simulation for Loot {
    type Input = u8;
    type Snapshot = Loot;
    type Error = Infallible;

    fn update(&mut self, _: Tick, kills: &u8) -> Result<(), Self::Error> {
        for _ in 0..*kills {
            self.gold += self.rng.range_u64(1..20);
            if self.rng.chance(0.3) {
                let item = *self
                    .rng
                    .weighted(&[1u32, 2, 3], |id| f64::from(4 - id))
                    .unwrap();
                self.drops.push(item);
            }
        }
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        let mut hasher = StateHasher::new();
        hasher.write_rng(&self.rng);
        hasher.write_u64(self.gold);
        for drop in &self.drops {
            hasher.write_u32(*drop);
        }
        hasher.finish()
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.clone()
    }
    fn restore(&mut self, snapshot: &Self::Snapshot) -> Result<(), Self::Error> {
        *self = snapshot.clone();
        Ok(())
    }
}

fn config() -> SimulationConfig {
    SimulationConfig::new(1, 0xDEAD_BEEF, "loot")
}

#[test]
fn replays_with_rng_state_are_exact() {
    let mut recorder = Recorder::new(Loot::new(&config()), config());
    for tick in 0..200u32 {
        recorder.tick((tick % 4) as u8).unwrap();
    }
    let mut replay = Replay::new(Loot::new(&config()), recorder.journal().clone()).unwrap();
    let summary = replay.run_to_end().unwrap();
    assert_eq!(summary.last_hash, recorder.current_hash());
    assert_eq!(replay.simulation().gold, recorder.simulation().gold);
    assert_eq!(replay.simulation().drops, recorder.simulation().drops);
}

#[test]
fn a_different_seed_diverges_and_is_reported() {
    let mut recorder = Recorder::new(Loot::new(&config()), config());
    for _ in 0..20 {
        recorder.tick(3).unwrap();
    }
    let other = SimulationConfig::new(1, 1, "loot");
    let mut replay = Replay::new(Loot::new(&other), recorder.journal().clone()).unwrap();
    assert!(matches!(
        replay.run_to_end(),
        Err(ReplayError::Divergence(divergence)) if divergence.tick == Tick::new(1)
    ));
}

#[test]
fn snapshots_restore_the_stream_position() {
    let mut recorder = Recorder::new(Loot::new(&config()), config());
    for _ in 0..10 {
        recorder.tick(2).unwrap();
    }
    let snapshot = recorder.snapshot();
    let mut continued = recorder.simulation().clone();
    continued.update(Tick::new(11), &5).unwrap();
    let mut restored = Loot::new(&SimulationConfig::new(1, 99, "other"));
    restored.restore(&snapshot.state).unwrap();
    restored.update(Tick::new(11), &5).unwrap();
    assert_eq!(restored.state_hash(), continued.state_hash());
}

#[test]
fn config_streams_are_stable_and_named() {
    assert_eq!(config().rng(), Rng::seed_from_u64(0xDEAD_BEEF));
    assert_ne!(config().rng_for("loot"), config().rng_for("ai"));
    let mut hasher_a = StateHasher::new();
    hasher_a.write_rng(&config().rng());
    let mut rng = config().rng();
    rng.next_u64();
    let mut hasher_b = StateHasher::new();
    hasher_b.write_rng(&rng);
    assert_ne!(hasher_a.finish(), hasher_b.finish());
}

#[cfg(feature = "serde")]
#[test]
fn rng_serialises_as_its_state_words() {
    let mut rng = config().rng();
    rng.next_u64();
    let json = serde_json::to_string(&rng).unwrap();
    assert_eq!(json, serde_json::to_string(&rng.state()).unwrap());
    let back: Rng = serde_json::from_str(&json).unwrap();
    assert_eq!(back, rng);
    assert!(serde_json::from_str::<Rng>("[0,0,0,0]").is_err());
}

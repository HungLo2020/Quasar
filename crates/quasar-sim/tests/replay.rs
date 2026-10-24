use quasar_core::{FixedStep, FixedStepConfig};
use quasar_sim::{
    Journal, JournalError, Recorder, Replay, ReplayError, ScheduledSimulation, Simulation,
    SimulationConfig, Snapshot, SnapshotError, StateHash, StateHasher, Tick,
};
use std::{convert::Infallible, error::Error, time::Duration};

#[derive(Clone, Default)]
struct Counter {
    value: i64,
    ticks: u64,
}
impl Simulation for Counter {
    type Input = i64;
    type Snapshot = (i64, u64);
    type Error = Infallible;

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

fn config() -> SimulationConfig {
    SimulationConfig::new(3, 0x1234_5678, "test-timeline")
}

fn recorded(inputs: impl IntoIterator<Item = i64>) -> Recorder<Counter> {
    let mut recorder = Recorder::new(Counter::default(), config());
    for input in inputs {
        recorder.tick(input).unwrap();
    }
    recorder
}

#[test]
fn records_and_replays_exactly() {
    let recorder = recorded((1..=100).map(|value| value as i64));
    assert_eq!(recorder.journal().len(), 100);
    assert_eq!(recorder.current_tick(), Tick::new(100));
    let mut replay = Replay::new(Counter::default(), recorder.journal().clone()).unwrap();
    let summary = replay.run_to_end().unwrap();
    assert_eq!(summary.ticks, 100);
    assert_eq!(summary.last_tick, Tick::new(100));
    assert_eq!(summary.last_hash, recorder.current_hash());
    assert_eq!(summary.config, *recorder.config());
    assert!(replay.is_exhausted());
    assert_eq!(replay.remaining(), 0);
    assert_eq!(
        replay.step().unwrap_err().to_string(),
        "replay exhausted before tick 101"
    );
}

#[test]
fn empty_replay_is_valid_and_exhausts_immediately() {
    let journal = Journal::<i64>::empty(config());
    let mut replay = Replay::new(Counter::default(), journal).unwrap();
    assert!(replay.is_exhausted());
    let summary = replay.run_to_end().unwrap();
    assert_eq!(summary.ticks, 0);
    assert_eq!(summary.last_tick, Tick::ZERO);
    assert!(matches!(
        replay.step(),
        Err(ReplayError::Exhausted { next_tick }) if next_tick == Tick::new(1)
    ));
}

#[test]
fn version_timeline_and_seed_mismatches_are_explicit() {
    let recorder = recorded([1]);
    let journal = recorder.journal().clone();
    let version = SimulationConfig::new(4, 0x1234_5678, "test-timeline");
    let error = Replay::with_expected_config(Counter::default(), journal.clone(), &version)
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ReplayError::VersionMismatch {
            expected: 4,
            found: 3
        }
    ));

    let timeline = SimulationConfig::new(3, 0x1234_5678, "other");
    let error = Replay::with_expected_config(Counter::default(), journal.clone(), &timeline)
        .err()
        .unwrap();
    assert!(matches!(error, ReplayError::TimelineMismatch { .. }));

    let seed = SimulationConfig::new(3, 1, "test-timeline");
    let error = Replay::with_expected_config(Counter::default(), journal, &seed)
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ReplayError::SeedMismatch {
            expected: 1,
            found: 0x1234_5678
        }
    ));
}

#[test]
fn injected_divergence_reports_the_first_tick() {
    let recorder = recorded([1, 2, 3, 4]);
    let mut records = recorder.journal().records().to_vec();
    records[0].hash = StateHash::new(0xdead_beef);
    let journal = Journal::new(config(), records).unwrap();
    let mut replay = Replay::new(Counter::default(), journal).unwrap();
    let error = replay.step().unwrap_err();
    let ReplayError::Divergence(divergence) = &error else {
        panic!("expected divergence, got {error}");
    };
    assert_eq!(divergence.tick, Tick::new(1));
    assert_eq!(divergence.input_index, 0);
    assert_eq!(divergence.expected, StateHash::new(0xdead_beef));
    assert_ne!(divergence.actual, divergence.expected);
    assert_eq!(replay.divergence(), Some(divergence));
    assert!(error.to_string().contains("first divergence at tick 1"));
    assert!(matches!(replay.step(), Err(ReplayError::Divergence(_))));
}

#[test]
fn snapshot_restore_truncates_the_journal_and_resumes_sequentially() {
    let mut recorder = recorded([1, 2, 3, 4, 5]);
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.tick, Tick::new(5));
    assert_eq!(snapshot.hash, recorder.current_hash());
    recorder.tick(6).unwrap();
    recorder.tick(7).unwrap();
    assert_eq!(recorder.journal().len(), 7);

    recorder.restore(&snapshot).unwrap();
    assert_eq!(recorder.current_tick(), Tick::new(5));
    assert_eq!(recorder.journal().len(), 5);
    assert_eq!(recorder.current_hash(), snapshot.hash);
    assert_eq!(recorder.next_tick(), Tick::new(6));
    recorder.tick(8).unwrap();
    assert_eq!(recorder.current_tick(), Tick::new(6));
    assert_eq!(recorder.journal().records().last().unwrap().input, 8);

    let journal = recorder.journal().clone();
    let mut replay = Replay::new(Counter::default(), journal).unwrap();
    assert_eq!(replay.run_to_end().unwrap().ticks, 6);
}

#[test]
fn restoring_an_ahead_snapshot_is_rejected() {
    let mut recorder = recorded([1, 2, 3, 4]);
    let ahead = recorder.snapshot();
    let earlier = {
        recorder
            .restore(&Snapshot::new(
                Tick::new(2),
                recorder.journal().records()[1].hash,
                config(),
                (3, 2),
            ))
            .unwrap();
        recorder.snapshot()
    };
    let error = recorder.restore(&ahead).unwrap_err();
    assert!(matches!(
        error,
        SnapshotError::SnapshotAhead { snapshot, current }
            if snapshot == Tick::new(4) && current == Tick::new(2)
    ));
    assert_eq!(earlier.tick, Tick::new(2));
}

#[test]
fn tampered_snapshot_hashes_are_rejected() {
    let mut recorder = recorded([1, 2]);
    let snapshot = Snapshot::new(Tick::new(2), StateHash::new(999), config(), (3, 2));
    let error = recorder.restore(&snapshot).unwrap_err();
    assert!(matches!(error, SnapshotError::HashMismatch { .. }));
    assert!(error.to_string().contains("snapshot hash mismatch"));
}

#[test]
fn snapshots_from_another_timeline_are_rejected() {
    let recorder = recorded([1, 2]);
    let snapshot = recorder.snapshot();
    let mut other = Recorder::new(
        Counter::default(),
        SimulationConfig::new(3, 99, "other-timeline"),
    );
    let error = other.restore(&snapshot).unwrap_err();
    assert!(matches!(error, SnapshotError::ConfigMismatch { .. }));
    assert!(error.to_string().contains("other-timeline"));
}

#[test]
fn seeking_restores_and_replays_forward() {
    let recorder = recorded((1..=20).map(|value| value as i64));
    let mut replay = Replay::new(Counter::default(), recorder.journal().clone()).unwrap();
    let step = replay.seek(Tick::new(5)).unwrap();
    assert_eq!(step.tick, Tick::new(5));
    assert_eq!(replay.remaining(), 15);
    assert_eq!(replay.step().unwrap().tick, Tick::new(6));

    replay.seek(Tick::ZERO).unwrap();
    assert_eq!(replay.current_tick(), Tick::ZERO);
    assert_eq!(replay.remaining(), 20);
    assert_eq!(replay.step().unwrap().tick, Tick::new(1));

    replay.seek(Tick::new(20)).unwrap();
    assert!(replay.is_exhausted());
    let error = replay.seek(Tick::new(21)).unwrap_err();
    assert!(matches!(
        error,
        ReplayError::SeekOutOfRange { requested, available }
            if requested == Tick::new(21) && available == Tick::new(20)
    ));
}

#[test]
fn invalid_journal_order_is_rejected() {
    let journal = Journal::new(
        config(),
        vec![quasar_sim::TickRecord::new(
            Tick::new(2),
            1,
            StateHash::new(1),
        )],
    );
    assert!(matches!(
        journal.unwrap_err(),
        JournalError::NonSequential { index: 0, expected, found }
            if expected == Tick::new(1) && found == Tick::new(2)
    ));
}

#[derive(Debug)]
struct FailAt(u64);
impl std::fmt::Display for FailAt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "failed at tick {}", self.0)
    }
}
impl std::error::Error for FailAt {}

struct Failing {
    tick: u64,
}
impl Simulation for Failing {
    type Input = ();
    type Snapshot = ();
    type Error = FailAt;

    fn update(&mut self, tick: Tick, _: &()) -> Result<(), Self::Error> {
        if tick.value() == self.tick {
            return Err(FailAt(self.tick));
        }
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        StateHash::new(self.tick)
    }
    fn snapshot(&self) {}
    fn restore(&mut self, _: &()) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn recorder_and_replay_propagate_simulation_errors() {
    let mut recorder = Recorder::new(Failing { tick: 3 }, config());
    recorder.tick(()).unwrap();
    recorder.tick(()).unwrap();
    let error = recorder.tick(()).unwrap_err();
    assert_eq!(error.to_string(), "failed at tick 3");

    let mut recorder = Recorder::new(Failing { tick: 999 }, config());
    recorder.tick(()).unwrap();
    let mut journal_records = recorder.journal().records().to_vec();
    journal_records.push(quasar_sim::TickRecord::new(
        Tick::new(2),
        (),
        StateHash::new(0),
    ));
    let journal = Journal::new(config(), journal_records).unwrap();
    // A fresh simulation failing on tick 1 proves errors are mapped without
    // being mistaken for divergence.
    let mut replay = Replay::new(Failing { tick: 1 }, journal).unwrap();
    let error = replay.step().unwrap_err();
    assert!(matches!(error, ReplayError::Simulation { .. }));
    assert_eq!(error.to_string(), "simulation failed: failed at tick 1");
    assert!(error.source().is_some());
}

#[test]
fn scheduled_simulation_reuses_fixed_step_and_records_exact_ticks() {
    let mut scheduled = ScheduledSimulation::from_config(
        Counter::default(),
        config(),
        FixedStepConfig::for_rate(60.0, 8),
    )
    .unwrap();
    assert_eq!(scheduled.advance_ticks(10, |_| 1).unwrap(), 10);
    assert_eq!(scheduled.recorder().current_tick(), Tick::new(10));
    let before = scheduled.recorder().current_tick();
    let ticks = scheduled.advance(Duration::from_millis(50), |_| 1).unwrap();
    assert!((2..=3).contains(&ticks), "ticks={ticks}");
    assert_eq!(
        scheduled.recorder().current_tick(),
        Tick::new(before.value() + u64::from(ticks))
    );
    scheduled.clock_mut().reset();
    assert_eq!(
        scheduled.advance(Duration::from_secs(10), |_| 1).unwrap(),
        8
    );
    let total = scheduled.recorder().current_tick().value();
    let journal = scheduled.recorder().journal().clone();
    let mut replay = Replay::new(Counter::default(), journal).unwrap();
    assert_eq!(replay.run_to_end().unwrap().ticks, total);
}

#[test]
fn scheduled_simulation_snapshot_round_trips() {
    let mut scheduled = ScheduledSimulation::new(
        Counter::default(),
        config(),
        FixedStep::new(60.0, 8).unwrap(),
    );
    scheduled
        .advance_ticks(5, |tick| tick.value() as i64)
        .unwrap();
    let snapshot = scheduled.snapshot();
    scheduled.advance_ticks(5, |_| 1).unwrap();
    scheduled.restore(&snapshot).unwrap();
    assert_eq!(scheduled.recorder().current_tick(), Tick::new(5));
    scheduled.advance_ticks(1, |_| 7).unwrap();
    assert_eq!(scheduled.recorder().journal().len(), 6);
}

#[cfg(feature = "serde")]
#[test]
fn journals_and_reports_serialize_structurally() {
    let recorder = recorded([1, 2, 3]);
    let json = serde_json::to_string(recorder.journal()).unwrap();
    let decoded: Journal<i64> = serde_json::from_str(&json).unwrap();
    assert_eq!(&decoded, recorder.journal());
    let mut replay = Replay::new(Counter::default(), decoded).unwrap();
    assert_eq!(replay.run_to_end().unwrap().ticks, 3);

    let invalid_json = r#"{
        "config": {"version": 1, "seed": 0, "timeline": "bad"},
        "records": [
            {"tick": 2, "input": 1, "hash": 1}
        ]
    }"#;
    let invalid: Journal<i64> = serde_json::from_str(invalid_json).unwrap();
    let error = Replay::new(Counter::default(), invalid).err().unwrap();
    assert!(matches!(error, ReplayError::InvalidJournal(_)));

    let divergence = quasar_sim::Divergence {
        tick: Tick::new(2),
        input_index: 1,
        expected: StateHash::new(1),
        actual: StateHash::new(2),
        config: config(),
    };
    let json = serde_json::to_string(&divergence).unwrap();
    assert_eq!(
        serde_json::from_str::<quasar_sim::Divergence>(&json).unwrap(),
        divergence
    );
}

#[cfg(feature = "jobs")]
#[test]
fn optional_jobs_integration_replays_off_thread() {
    use quasar_jobs::JobPool;
    let recorder = recorded((1..=50).map(|value| value as i64));
    let replay = Replay::new(Counter::default(), recorder.journal().clone()).unwrap();
    let pool = JobPool::manual();
    let handle = replay.into_jobs(&pool).unwrap();
    pool.run_all();
    let summary = handle.wait().unwrap().unwrap();
    assert_eq!(summary.ticks, 50);
    assert_eq!(summary.last_hash, recorder.current_hash());
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn recording_then_replaying_always_matches(inputs in prop::collection::vec(-1000_i64..=1000_i64, 0..200)) {
            let recorder = recorded(inputs.iter().copied());
            let mut replay = Replay::new(Counter::default(), recorder.journal().clone()).unwrap();
            let summary = replay.run_to_end().unwrap();
            prop_assert_eq!(summary.ticks, inputs.len() as u64);
            prop_assert_eq!(summary.last_hash, recorder.current_hash());
            prop_assert_eq!(replay.current_tick(), Tick::new(inputs.len() as u64));
            prop_assert!(replay.is_exhausted());
        }

        #[test]
        fn seeking_matches_prefix_replays(
            inputs in prop::collection::vec(-50_i64..=50_i64, 0..100),
            requested in 0_usize..100,
        ) {
            let recorder = recorded(inputs.iter().copied());
            let requested = requested.min(inputs.len());
            let mut prefix = Recorder::new(Counter::default(), config());
            for input in inputs.iter().take(requested) {
                prefix.tick(*input).unwrap();
            }
            let mut replay = Replay::new(Counter::default(), recorder.journal().clone()).unwrap();
            let step = replay.seek(Tick::new(requested as u64)).unwrap();
            prop_assert_eq!(step.hash, prefix.current_hash());
            prop_assert_eq!(replay.current_hash(), prefix.current_hash());
            prop_assert_eq!(replay.remaining(), inputs.len() - requested);
        }
    }
}

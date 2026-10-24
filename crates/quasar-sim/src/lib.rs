//! Deterministic simulation coordination, snapshots and replay verification.
//!
//! `quasar-sim` deliberately does not own game state. A game implements
//! [`Simulation`] with its own input, snapshot and state-hashing types; this
//! crate supplies tick numbering, recording, replay comparison, divergence
//! diagnostics, snapshot restoration and an optional `quasar-core::FixedStep`
//! scheduler.
//!
//! ```
//! use quasar_sim::{Recorder, Simulation, SimulationConfig, StateHash, Tick};
//!
//! #[derive(Default)]
//! struct Counter(u64);
//! impl Simulation for Counter {
//!     type Input = u64;
//!     type Snapshot = u64;
//!     type Error = std::convert::Infallible;
//!     fn update(&mut self, _: Tick, input: &u64) -> Result<(), Self::Error> {
//!         self.0 += *input;
//!         Ok(())
//!     }
//!     fn state_hash(&self) -> StateHash {
//!         StateHash::new(self.0)
//!     }
//!     fn snapshot(&self) -> u64 {
//!         self.0
//!     }
//!     fn restore(&mut self, snapshot: &u64) -> Result<(), Self::Error> {
//!         self.0 = *snapshot;
//!         Ok(())
//!     }
//! }
//!
//! let mut recorder = Recorder::new(Counter::default(), SimulationConfig::new(1, 42, "demo"));
//! recorder.tick(2).unwrap();
//! recorder.tick(3).unwrap();
//! assert_eq!(recorder.current_tick(), Tick::new(2));
//! ```
//!
//! Deterministic guarantees are limited to the state/input/hash the game
//! supplies. Cross-CPU floating-point bitwise parity is not claimed.

use quasar_core::{FixedStep, FixedStepConfig};
use std::{error::Error, fmt, time::Duration};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Monotonic simulation tick. The first applied tick is `1`; `0` means the
/// initial state before any update.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Tick(u64);
impl Tick {
    pub const ZERO: Self = Self(0);
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn value(self) -> u64 {
        self.0
    }
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
    pub const fn previous(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}
impl fmt::Display for Tick {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Stable identifier for a recorded timeline.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TimelineId(String);
impl TimelineId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<&str> for TimelineId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}
impl From<String> for TimelineId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}
impl fmt::Display for TimelineId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Timeline metadata recorded with a journal.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SimulationConfig {
    pub version: u64,
    pub seed: u64,
    pub timeline: TimelineId,
}
impl SimulationConfig {
    pub fn new(version: u64, seed: u64, timeline: impl Into<TimelineId>) -> Self {
        Self {
            version,
            seed,
            timeline: timeline.into(),
        }
    }
}

/// Stable state fingerprint. The included [`StateHasher`] is FNV-1a based; a
/// game may supply any stable hash through [`Simulation::state_hash`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct StateHash(u64);
impl StateHash {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}
impl fmt::Display for StateHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:016x}", self.0)
    }
}

/// Platform-stable FNV-1a hasher for game-provided state fields.
///
/// This is not cryptographic and does not make floating-point results portable;
/// games should hash quantized/bitwise values and test their own platforms.
#[derive(Clone, Debug)]
pub struct StateHasher(u64);
impl StateHasher {
    pub const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    pub fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    pub fn write_u8(&mut self, value: u8) {
        self.write(&[value]);
    }
    pub fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }
    pub fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }
    pub fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }
    pub fn write_f32_bits(&mut self, value: f32) {
        self.write_u32(value.to_bits());
    }
    pub fn write_f64_bits(&mut self, value: f64) {
        self.write_u64(value.to_bits());
    }
    pub fn finish(self) -> StateHash {
        StateHash(self.0)
    }
    pub fn hash(bytes: &[u8]) -> StateHash {
        let mut hasher = Self::new();
        hasher.write(bytes);
        hasher.finish()
    }
}
impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Game-owned deterministic simulation boundary.
pub trait Simulation {
    type Input: Clone;
    type Snapshot;
    type Error: Error + Send + Sync + 'static;

    /// Applies exactly one input at `tick`. The game owns all state and may use
    /// its own RNG, seeded externally from [`SimulationConfig::seed`].
    fn update(&mut self, tick: Tick, input: &Self::Input) -> Result<(), Self::Error>;
    /// Stable fingerprint after the last applied tick.
    fn state_hash(&self) -> StateHash;
    /// Caller-owned snapshot representation, for example a clone, bytes, or a
    /// game-defined keyframe.
    fn snapshot(&self) -> Self::Snapshot;
    fn restore(&mut self, snapshot: &Self::Snapshot) -> Result<(), Self::Error>;
}

/// One recorded input and the resulting state hash.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TickRecord<I> {
    pub tick: Tick,
    pub input: I,
    pub hash: StateHash,
}
impl<I> TickRecord<I> {
    pub fn new(tick: Tick, input: I, hash: StateHash) -> Self {
        Self { tick, input, hash }
    }
}

/// Ordered journal of inputs and expected hashes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Journal<I> {
    config: SimulationConfig,
    records: Vec<TickRecord<I>>,
}
impl<I> Journal<I> {
    pub fn empty(config: SimulationConfig) -> Self {
        Self {
            config,
            records: Vec::new(),
        }
    }
    /// Builds and validates a journal. Ticks must be consecutive starting at 1.
    pub fn new(
        config: SimulationConfig,
        records: Vec<TickRecord<I>>,
    ) -> Result<Self, JournalError> {
        let journal = Self { config, records };
        journal.validate()?;
        Ok(journal)
    }
    pub fn config(&self) -> &SimulationConfig {
        &self.config
    }
    pub fn records(&self) -> &[TickRecord<I>] {
        &self.records
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn last_tick(&self) -> Tick {
        self.records.last().map_or(Tick::ZERO, |record| record.tick)
    }
    pub fn next_tick(&self) -> Tick {
        self.last_tick().next()
    }
    pub fn validate(&self) -> Result<(), JournalError> {
        let mut expected = Tick::new(1);
        for (index, record) in self.records.iter().enumerate() {
            if record.tick != expected {
                return Err(JournalError::NonSequential {
                    index,
                    expected,
                    found: record.tick,
                });
            }
            expected = expected.next();
        }
        Ok(())
    }
    pub(crate) fn push(&mut self, record: TickRecord<I>) {
        self.records.push(record);
    }
    pub(crate) fn truncate_after(&mut self, tick: Tick) {
        self.records.truncate(tick.value() as usize);
    }
}

/// Invalid journal ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalError {
    NonSequential {
        index: usize,
        expected: Tick,
        found: Tick,
    },
}
impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonSequential {
                index,
                expected,
                found,
            } => write!(
                formatter,
                "journal record {index} has tick {found}, expected {expected}"
            ),
        }
    }
}
impl Error for JournalError {}

/// Snapshot returned by [`Recorder::snapshot`].
pub struct Snapshot<S> {
    pub tick: Tick,
    pub hash: StateHash,
    pub config: SimulationConfig,
    pub state: S,
}
impl<S> Snapshot<S> {
    pub fn new(tick: Tick, hash: StateHash, config: SimulationConfig, state: S) -> Self {
        Self {
            tick,
            hash,
            config,
            state,
        }
    }
}
impl<S: fmt::Debug> fmt::Debug for Snapshot<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("tick", &self.tick)
            .field("hash", &self.hash)
            .field("config", &self.config)
            .field("state", &self.state)
            .finish()
    }
}

/// Failure restoring a snapshot into a recorder.
#[derive(Debug)]
pub enum SnapshotError<E> {
    SnapshotAhead {
        snapshot: Tick,
        current: Tick,
    },
    ConfigMismatch {
        snapshot: SimulationConfig,
        recorder: SimulationConfig,
    },
    HashMismatch {
        expected: StateHash,
        actual: StateHash,
    },
    Simulation(E),
}
impl<E: fmt::Display> fmt::Display for SnapshotError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SnapshotAhead { snapshot, current } => write!(
                formatter,
                "snapshot tick {snapshot} is ahead of recorder tick {current}"
            ),
            Self::ConfigMismatch { snapshot, recorder } => write!(
                formatter,
                "snapshot timeline '{}' (version {}, seed {}) does not match recorder timeline '{}' (version {}, seed {})",
                snapshot.timeline,
                snapshot.version,
                snapshot.seed,
                recorder.timeline,
                recorder.version,
                recorder.seed
            ),
            Self::HashMismatch { expected, actual } => write!(
                formatter,
                "snapshot hash mismatch after restore: expected {expected}, got {actual}"
            ),
            Self::Simulation(error) => write!(formatter, "snapshot restore failed: {error}"),
        }
    }
}
impl<E: Error + 'static> Error for SnapshotError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Simulation(error) => Some(error),
            Self::SnapshotAhead { .. }
            | Self::ConfigMismatch { .. }
            | Self::HashMismatch { .. } => None,
        }
    }
}

/// Records inputs and per-tick hashes for a game-owned simulation.
pub struct Recorder<S: Simulation> {
    simulation: S,
    config: SimulationConfig,
    journal: Journal<S::Input>,
    next_tick: Tick,
    current_hash: StateHash,
}
impl<S: Simulation> Recorder<S> {
    pub fn new(simulation: S, config: SimulationConfig) -> Self {
        let current_hash = simulation.state_hash();
        Self {
            simulation,
            journal: Journal::empty(config.clone()),
            config,
            next_tick: Tick::new(1),
            current_hash,
        }
    }
    pub fn config(&self) -> &SimulationConfig {
        &self.config
    }
    /// Read-only access. Mutate state only through [`Self::tick`] so journal
    /// hashes stay consistent.
    pub fn simulation(&self) -> &S {
        &self.simulation
    }
    pub fn journal(&self) -> &Journal<S::Input> {
        &self.journal
    }
    pub fn next_tick(&self) -> Tick {
        self.next_tick
    }
    pub fn current_tick(&self) -> Tick {
        self.next_tick.previous()
    }
    pub fn current_hash(&self) -> StateHash {
        self.current_hash
    }
    /// Applies, records and hashes exactly one tick.
    pub fn tick(&mut self, input: S::Input) -> Result<Tick, S::Error> {
        let tick = self.next_tick;
        self.simulation.update(tick, &input)?;
        let hash = self.simulation.state_hash();
        self.journal.push(TickRecord { tick, input, hash });
        self.next_tick = tick.next();
        self.current_hash = hash;
        Ok(tick)
    }
    pub fn tick_ref(&mut self, input: &S::Input) -> Result<Tick, S::Error> {
        self.tick(input.clone())
    }
    /// Captures the state after the last applied tick.
    pub fn snapshot(&self) -> Snapshot<S::Snapshot> {
        Snapshot {
            tick: self.current_tick(),
            hash: self.current_hash,
            config: self.config.clone(),
            state: self.simulation.snapshot(),
        }
    }
    /// Restores a snapshot and truncates the journal to the same tick.
    pub fn restore(
        &mut self,
        snapshot: &Snapshot<S::Snapshot>,
    ) -> Result<(), SnapshotError<S::Error>> {
        if snapshot.config != self.config {
            return Err(SnapshotError::ConfigMismatch {
                snapshot: snapshot.config.clone(),
                recorder: self.config.clone(),
            });
        }
        if snapshot.tick > self.current_tick() {
            return Err(SnapshotError::SnapshotAhead {
                snapshot: snapshot.tick,
                current: self.current_tick(),
            });
        }
        self.simulation
            .restore(&snapshot.state)
            .map_err(SnapshotError::Simulation)?;
        let actual = self.simulation.state_hash();
        if actual != snapshot.hash {
            return Err(SnapshotError::HashMismatch {
                expected: snapshot.hash,
                actual,
            });
        }
        self.next_tick = snapshot.tick.next();
        self.current_hash = actual;
        self.journal.truncate_after(snapshot.tick);
        Ok(())
    }
    pub fn into_parts(self) -> (S, Journal<S::Input>) {
        (self.simulation, self.journal)
    }
}

/// Result of one successful replay update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ReplayStep {
    pub tick: Tick,
    pub hash: StateHash,
}

/// Summary after replaying all remaining records.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ReplaySummary {
    pub ticks: u64,
    pub last_tick: Tick,
    pub last_hash: StateHash,
    pub config: SimulationConfig,
}

/// First mismatch between recorded and replayed state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Divergence {
    pub tick: Tick,
    pub input_index: usize,
    pub expected: StateHash,
    pub actual: StateHash,
    pub config: SimulationConfig,
}
impl fmt::Display for Divergence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "first divergence at tick {} of timeline '{}' (version {}, seed {}): expected {}, got {}",
            self.tick,
            self.config.timeline,
            self.config.version,
            self.config.seed,
            self.expected,
            self.actual
        )
    }
}

/// Replay failure with machine-readable structure.
#[derive(Debug)]
pub enum ReplayError {
    InvalidJournal(JournalError),
    Exhausted {
        next_tick: Tick,
    },
    SeekOutOfRange {
        requested: Tick,
        available: Tick,
    },
    VersionMismatch {
        expected: u64,
        found: u64,
    },
    TimelineMismatch {
        expected: TimelineId,
        found: TimelineId,
    },
    SeedMismatch {
        expected: u64,
        found: u64,
    },
    Divergence(Divergence),
    Simulation {
        message: String,
        source: Box<dyn Error + Send + Sync>,
    },
}
impl ReplayError {
    pub fn divergence(&self) -> Option<&Divergence> {
        match self {
            Self::Divergence(divergence) => Some(divergence),
            _ => None,
        }
    }
    fn simulation(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Simulation {
            message: error.to_string(),
            source: Box::new(error),
        }
    }
}
impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJournal(error) => write!(formatter, "invalid journal: {error}"),
            Self::Exhausted { next_tick } => {
                write!(formatter, "replay exhausted before tick {next_tick}")
            }
            Self::SeekOutOfRange {
                requested,
                available,
            } => write!(
                formatter,
                "cannot seek to tick {requested}; replay has {available} ticks"
            ),
            Self::VersionMismatch { expected, found } => {
                write!(
                    formatter,
                    "simulation version mismatch: expected {expected}, found {found}"
                )
            }
            Self::TimelineMismatch { expected, found } => {
                write!(
                    formatter,
                    "timeline mismatch: expected '{expected}', found '{found}'"
                )
            }
            Self::SeedMismatch { expected, found } => {
                write!(
                    formatter,
                    "seed mismatch: expected {expected}, found {found}"
                )
            }
            Self::Divergence(divergence) => divergence.fmt(formatter),
            Self::Simulation { message, .. } => write!(formatter, "simulation failed: {message}"),
        }
    }
}
impl Error for ReplayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJournal(error) => Some(error),
            Self::Simulation { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// Replays a journal against a fresh simulation and compares every hash.
pub struct Replay<S: Simulation> {
    simulation: S,
    journal: Journal<S::Input>,
    initial_snapshot: S::Snapshot,
    initial_hash: StateHash,
    index: usize,
    current_tick: Tick,
    current_hash: StateHash,
    divergence: Option<Divergence>,
}
impl<S: Simulation> Replay<S> {
    pub fn new(simulation: S, journal: Journal<S::Input>) -> Result<Self, ReplayError> {
        journal.validate().map_err(ReplayError::InvalidJournal)?;
        let initial_hash = simulation.state_hash();
        let initial_snapshot = simulation.snapshot();
        Ok(Self {
            simulation,
            journal,
            initial_snapshot,
            initial_hash,
            index: 0,
            current_tick: Tick::ZERO,
            current_hash: initial_hash,
            divergence: None,
        })
    }
    /// Rejects version, timeline or seed mismatches before replay begins.
    pub fn with_expected_config(
        simulation: S,
        journal: Journal<S::Input>,
        expected: &SimulationConfig,
    ) -> Result<Self, ReplayError> {
        let found = journal.config();
        if expected.version != found.version {
            return Err(ReplayError::VersionMismatch {
                expected: expected.version,
                found: found.version,
            });
        }
        if expected.timeline != found.timeline {
            return Err(ReplayError::TimelineMismatch {
                expected: expected.timeline.clone(),
                found: found.timeline.clone(),
            });
        }
        if expected.seed != found.seed {
            return Err(ReplayError::SeedMismatch {
                expected: expected.seed,
                found: found.seed,
            });
        }
        Self::new(simulation, journal)
    }
    /// Read-only access. Mutate state only through [`Self::step`] so replay
    /// checks remain meaningful.
    pub fn simulation(&self) -> &S {
        &self.simulation
    }
    pub fn journal(&self) -> &Journal<S::Input> {
        &self.journal
    }
    pub fn current_tick(&self) -> Tick {
        self.current_tick
    }
    pub fn current_hash(&self) -> StateHash {
        self.current_hash
    }
    pub fn remaining(&self) -> usize {
        self.journal.records.len().saturating_sub(self.index)
    }
    pub fn is_exhausted(&self) -> bool {
        self.remaining() == 0
    }
    pub fn divergence(&self) -> Option<&Divergence> {
        self.divergence.as_ref()
    }
    /// Applies the next exact input and verifies its recorded hash.
    pub fn step(&mut self) -> Result<ReplayStep, ReplayError> {
        if let Some(divergence) = &self.divergence {
            return Err(ReplayError::Divergence(divergence.clone()));
        }
        let Some(record) = self.journal.records.get(self.index) else {
            return Err(ReplayError::Exhausted {
                next_tick: self.current_tick.next(),
            });
        };
        let expected_tick = self.current_tick.next();
        if record.tick != expected_tick {
            return Err(ReplayError::InvalidJournal(JournalError::NonSequential {
                index: self.index,
                expected: expected_tick,
                found: record.tick,
            }));
        }
        let tick = record.tick;
        let input = record.input.clone();
        let expected = record.hash;
        self.simulation
            .update(tick, &input)
            .map_err(ReplayError::simulation)?;
        let actual = self.simulation.state_hash();
        if actual != expected {
            let divergence = Divergence {
                tick,
                input_index: self.index,
                expected,
                actual,
                config: self.journal.config.clone(),
            };
            self.divergence = Some(divergence.clone());
            return Err(ReplayError::Divergence(divergence));
        }
        self.index += 1;
        self.current_tick = tick;
        self.current_hash = actual;
        Ok(ReplayStep { tick, hash: actual })
    }
    /// Replays all remaining records. Empty journals return an empty summary.
    pub fn run_to_end(&mut self) -> Result<ReplaySummary, ReplayError> {
        loop {
            match self.step() {
                Ok(_) => {}
                Err(ReplayError::Exhausted { .. }) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(self.summary())
    }
    /// Restores the initial state and replays forward to `requested`.
    ///
    /// Seeking is O(n) and verifies every intermediate hash. `Tick::ZERO`
    /// restores the initial state without applying an input.
    pub fn seek(&mut self, requested: Tick) -> Result<ReplayStep, ReplayError> {
        let available = Tick::new(self.journal.records.len() as u64);
        if requested > available {
            return Err(ReplayError::SeekOutOfRange {
                requested,
                available,
            });
        }
        self.simulation
            .restore(&self.initial_snapshot)
            .map_err(ReplayError::simulation)?;
        self.index = 0;
        self.current_tick = Tick::ZERO;
        self.current_hash = self.simulation.state_hash();
        self.divergence = None;
        if self.current_hash != self.initial_hash {
            let divergence = Divergence {
                tick: Tick::ZERO,
                input_index: 0,
                expected: self.initial_hash,
                actual: self.current_hash,
                config: self.journal.config.clone(),
            };
            self.divergence = Some(divergence.clone());
            return Err(ReplayError::Divergence(divergence));
        }
        while self.current_tick < requested {
            self.step()?;
        }
        Ok(ReplayStep {
            tick: self.current_tick,
            hash: self.current_hash,
        })
    }
    pub fn summary(&self) -> ReplaySummary {
        ReplaySummary {
            ticks: self.current_tick.value(),
            last_tick: self.current_tick,
            last_hash: self.current_hash,
            config: self.journal.config.clone(),
        }
    }
    pub fn into_simulation(self) -> S {
        self.simulation
    }
}

/// Fixed-step scheduler layered over [`Recorder`].
///
/// `advance` converts caller-supplied elapsed time with
/// [`quasar_core::FixedStep`]; `advance_ticks` is the deterministic headless
/// path. Rendering remains independent and may run at a different rate.
pub struct ScheduledSimulation<S: Simulation> {
    recorder: Recorder<S>,
    clock: FixedStep,
}
impl<S: Simulation> ScheduledSimulation<S> {
    pub fn new(simulation: S, config: SimulationConfig, clock: FixedStep) -> Self {
        Self {
            recorder: Recorder::new(simulation, config),
            clock,
        }
    }
    pub fn from_config(
        simulation: S,
        config: SimulationConfig,
        clock: FixedStepConfig,
    ) -> Result<Self, &'static str> {
        Ok(Self::new(
            simulation,
            config,
            FixedStep::with_config(clock)?,
        ))
    }
    pub fn recorder(&self) -> &Recorder<S> {
        &self.recorder
    }
    pub fn recorder_mut(&mut self) -> &mut Recorder<S> {
        &mut self.recorder
    }
    pub fn clock(&self) -> &FixedStep {
        &self.clock
    }
    pub fn clock_mut(&mut self) -> &mut FixedStep {
        &mut self.clock
    }
    pub fn interpolation(&self) -> f64 {
        self.clock.interpolation()
    }
    /// Advances by wall-clock time and applies every tick due. The provider is
    /// called only for ticks that are actually executed.
    pub fn advance<F>(&mut self, elapsed: Duration, mut input: F) -> Result<u32, S::Error>
    where
        F: FnMut(Tick) -> S::Input,
    {
        self.clock.advance(elapsed);
        let mut ticks = 0;
        while self.clock.take_tick() {
            let tick = self.recorder.next_tick();
            let value = input(tick);
            self.recorder.tick(value)?;
            ticks += 1;
        }
        Ok(ticks)
    }
    /// Applies exactly `ticks` updates without consulting the clock.
    pub fn advance_ticks<F>(&mut self, ticks: u64, mut input: F) -> Result<u64, S::Error>
    where
        F: FnMut(Tick) -> S::Input,
    {
        for _ in 0..ticks {
            let tick = self.recorder.next_tick();
            let value = input(tick);
            self.recorder.tick(value)?;
        }
        Ok(ticks)
    }
    pub fn snapshot(&self) -> Snapshot<S::Snapshot> {
        self.recorder.snapshot()
    }
    pub fn restore(
        &mut self,
        snapshot: &Snapshot<S::Snapshot>,
    ) -> Result<(), SnapshotError<S::Error>> {
        self.recorder.restore(snapshot)
    }
    pub fn into_parts(self) -> (S, Journal<S::Input>) {
        self.recorder.into_parts()
    }
}

#[cfg(feature = "jobs")]
impl<S> Replay<S>
where
    S: Simulation + Send + 'static,
    S::Input: Send + 'static,
    S::Snapshot: Send + 'static,
{
    /// Replays on a caller-owned `quasar-jobs` worker. The simulation and
    /// journal move to the worker; the handle resolves to a summary or a boxed
    /// replay error. This is an optional integration, not a required runtime.
    pub fn into_jobs(
        self,
        pool: &quasar_jobs::JobPool,
    ) -> Result<
        quasar_jobs::JobHandle<Result<ReplaySummary, Box<ReplayError>>>,
        quasar_jobs::JobError,
    > {
        pool.spawn(move |_| {
            let mut replay = self;
            Ok::<_, quasar_jobs::JobError>(replay.run_to_end().map_err(Box::new))
        })
    }
}

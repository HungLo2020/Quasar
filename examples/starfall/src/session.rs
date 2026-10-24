//! Recording/replay session around the game world.

use crate::settings::GameSettings;
use crate::world::{GameError, GameInput, GameWorld};
use quasar_sim::{Journal, Recorder, Replay, ReplayError, Tick};
use std::path::Path;

pub enum SessionMode {
    Live(Box<Recorder<GameWorld>>),
    Replay(Box<Replay<GameWorld>>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepResult {
    Ticked(Tick),
    Exhausted,
}

pub struct GameSession {
    mode: SessionMode,
}
impl GameSession {
    pub fn live(world: GameWorld, settings: &GameSettings) -> Self {
        Self {
            mode: SessionMode::Live(Box::new(Recorder::new(world, settings.simulation_config()))),
        }
    }
    pub fn replay(
        world: GameWorld,
        journal: Journal<GameInput>,
        settings: &GameSettings,
    ) -> Result<Self, GameError> {
        let replay = Replay::with_expected_config(world, journal, &settings.simulation_config())
            .map_err(|error| GameError::invalid(format!("replay rejected: {error}")))?;
        Ok(Self {
            mode: SessionMode::Replay(Box::new(replay)),
        })
    }
    pub fn from_file(
        world: GameWorld,
        path: &Path,
        settings: &GameSettings,
    ) -> Result<Self, GameError> {
        let bytes = std::fs::read(path).map_err(|error| {
            GameError::invalid(format!("read replay {}: {error}", path.display()))
        })?;
        let journal: Journal<GameInput> = serde_json::from_slice(&bytes)
            .map_err(|error| GameError::invalid(format!("parse replay: {error}")))?;
        Self::replay(world, journal, settings)
    }
    pub fn is_replay(&self) -> bool {
        matches!(self.mode, SessionMode::Replay(_))
    }
    pub fn step(&mut self, input: GameInput) -> Result<StepResult, GameError> {
        match &mut self.mode {
            SessionMode::Live(recorder) => recorder
                .tick(input)
                .map(StepResult::Ticked)
                .map_err(|never| match never {}),
            SessionMode::Replay(replay) => match replay.step() {
                Ok(step) => Ok(StepResult::Ticked(step.tick)),
                Err(ReplayError::Exhausted { .. }) => Ok(StepResult::Exhausted),
                Err(error) => Err(GameError::invalid(format!("replay: {error}"))),
            },
        }
    }
    pub fn world(&self) -> &GameWorld {
        match &self.mode {
            SessionMode::Live(recorder) => recorder.simulation(),
            SessionMode::Replay(replay) => replay.simulation(),
        }
    }
    pub fn current_tick(&self) -> Tick {
        match &self.mode {
            SessionMode::Live(recorder) => recorder.current_tick(),
            SessionMode::Replay(replay) => replay.current_tick(),
        }
    }
    pub fn current_hash(&self) -> quasar_sim::StateHash {
        match &self.mode {
            SessionMode::Live(recorder) => recorder.current_hash(),
            SessionMode::Replay(replay) => replay.current_hash(),
        }
    }
    pub fn journal(&self) -> Option<&Journal<GameInput>> {
        match &self.mode {
            SessionMode::Live(recorder) => Some(recorder.journal()),
            SessionMode::Replay(_) => None,
        }
    }
    pub fn write_record(&self, path: &Path) -> Result<(), GameError> {
        let Some(journal) = self.journal() else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(journal)
            .map_err(|error| GameError::invalid(format!("serialize replay: {error}")))?;
        std::fs::write(path, bytes).map_err(|error| {
            GameError::invalid(format!("write replay {}: {error}", path.display()))
        })
    }
    pub fn summary(&self) -> SessionSummary {
        let world = self.world();
        SessionSummary {
            ticks: self.current_tick().value(),
            hash: self.current_hash(),
            score: world.score,
            crystals_remaining: world.crystals_remaining(),
            drones_remaining: world.drones_remaining(),
            health: world.health,
            won: world.won,
            lost: world.lost,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub ticks: u64,
    pub hash: quasar_sim::StateHash,
    pub score: u32,
    pub crystals_remaining: usize,
    pub drones_remaining: usize,
    pub health: i32,
    pub won: bool,
    pub lost: bool,
}
impl std::fmt::Display for SessionSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "ticks={} hash={} score={} crystals={} drones={} health={} won={} lost={}",
            self.ticks,
            self.hash,
            self.score,
            self.crystals_remaining,
            self.drones_remaining,
            self.health,
            self.won,
            self.lost
        )
    }
}

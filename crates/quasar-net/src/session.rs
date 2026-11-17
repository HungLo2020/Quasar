//! Lockstep and rollback sessions driving a `quasar-sim` simulation.
use crate::{NetInput, Transport, transport::MAX_DATAGRAM};
use quasar_sim::{Journal, Simulation, SimulationConfig, StateHash, Tick, TickRecord};
use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

const MAGIC: [u8; 2] = *b"QN";
const VERSION: u8 = 1;
/// Finalized hashes repeated in every packet.
const HASHES_PER_PACKET: usize = 8;

/// How frames advance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// Every frame waits for every player's input. Simple and exact; local
    /// input lag is `input_delay` frames.
    Lockstep,
    /// Runs ahead on predicted remote inputs (each player's last input) and
    /// rolls back to resimulate when a prediction was wrong. At most
    /// `max_prediction` frames run ahead of the confirmed inputs.
    Rollback { max_prediction: u32 },
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub players: u8,
    /// This machine's player index.
    pub local: u8,
    /// Frames between reading local input and simulating it. Hides latency;
    /// lockstep needs at least the round trip in frames to never wait.
    pub input_delay: u32,
    pub mode: SyncMode,
    /// A peer silent for this long is reported once as disconnected.
    pub timeout: Duration,
    /// Identifies the simulation in the confirmed-input [`Journal`].
    pub simulation: SimulationConfig,
}

impl SessionConfig {
    pub fn new(players: u8, local: u8, mode: SyncMode, simulation: SimulationConfig) -> Self {
        Self {
            players,
            local,
            input_delay: 2,
            mode,
            timeout: Duration::from_secs(5),
            simulation,
        }
    }
}

/// Something a session noticed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetEvent {
    /// A peer's state hash differs from ours on a confirmed frame.
    Desync {
        tick: Tick,
        player: u8,
        local: StateHash,
        remote: StateHash,
    },
    Disconnected {
        player: u8,
    },
    /// A previously silent peer is heard again.
    Reconnected {
        player: u8,
    },
}

/// Result of one [`Session::advance`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advance {
    /// The frame simulated, or `None` while waiting for inputs.
    pub tick: Option<Tick>,
    /// Frames rolled back and resimulated before it (rollback mode).
    pub resimulated: u32,
}

/// Running totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub frames: u64,
    pub waits: u64,
    pub rollbacks: u64,
    pub resimulated: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_rejected: u64,
}

#[derive(Debug)]
pub enum NetError<E> {
    Transport(std::io::Error),
    Simulation(E),
    Config(String),
}

impl<E: fmt::Display> fmt::Display for NetError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Simulation(e) => write!(f, "simulation: {e}"),
            Self::Config(e) => write!(f, "configuration: {e}"),
        }
    }
}
impl<E: fmt::Debug + fmt::Display> std::error::Error for NetError<E> {}

struct Peer<A> {
    player: u8,
    addr: A,
    /// Highest of our local frames this peer has (contiguously).
    acked: u64,
    last_heard: Instant,
    connected: bool,
    remote_hashes: BTreeMap<u64, u64>,
}

/// Synchronises one simulation across peers. The simulation's input is
/// every player's input for the frame, indexed by player.
pub struct Session<I: NetInput, S: Simulation<Input = Vec<I>>, T: Transport> {
    config: SessionConfig,
    sim: S,
    transport: T,
    peers: Vec<Peer<T::Addr>>,
    /// Real inputs per player by frame.
    inputs: Vec<BTreeMap<u64, I>>,
    /// Highest frame up to which each player's inputs are all known.
    contiguous: Vec<u64>,
    local_next: u64,
    current: u64,
    used: BTreeMap<u64, Vec<I>>,
    snapshots: BTreeMap<u64, S::Snapshot>,
    hashes: BTreeMap<u64, StateHash>,
    finalized: u64,
    journal: Vec<TickRecord<Vec<I>>>,
    events: Vec<NetEvent>,
    stats: SessionStats,
    packet: Vec<u8>,
}

impl<I: NetInput, S: Simulation<Input = Vec<I>>, T: Transport> Session<I, S, T> {
    /// `peers` maps every remote player to its address.
    pub fn new(
        config: SessionConfig,
        simulation: S,
        transport: T,
        peers: Vec<(u8, T::Addr)>,
    ) -> Result<Self, NetError<S::Error>> {
        let players = usize::from(config.players);
        if players == 0 || config.local >= config.players {
            return Err(NetError::Config("local player must be < players".into()));
        }
        let mut remote: Vec<u8> = peers.iter().map(|(p, _)| *p).collect();
        remote.sort_unstable();
        let expected: Vec<u8> = (0..config.players).filter(|p| *p != config.local).collect();
        if remote != expected {
            return Err(NetError::Config(
                "peers must list every other player exactly once".into(),
            ));
        }
        if let SyncMode::Rollback { max_prediction } = config.mode
            && max_prediction == 0
        {
            return Err(NetError::Config(
                "rollback needs max_prediction >= 1".into(),
            ));
        }
        if I::SIZE == 0 || I::SIZE > 256 {
            return Err(NetError::Config(
                "inputs must encode to 1..=256 bytes".into(),
            ));
        }
        let delay = u64::from(config.input_delay);
        // Frames inside the input delay use the default input everywhere.
        let mut inputs = vec![BTreeMap::new(); players];
        for player in &mut inputs {
            for frame in 1..=delay {
                player.insert(frame, I::default());
            }
        }
        let now = Instant::now();
        let mut snapshots = BTreeMap::new();
        if matches!(config.mode, SyncMode::Rollback { .. }) {
            snapshots.insert(0, simulation.snapshot());
        }
        let mut hashes = BTreeMap::new();
        hashes.insert(0, simulation.state_hash());
        Ok(Self {
            peers: peers
                .into_iter()
                .map(|(player, addr)| Peer {
                    player,
                    addr,
                    acked: delay,
                    last_heard: now,
                    connected: true,
                    remote_hashes: BTreeMap::new(),
                })
                .collect(),
            inputs,
            contiguous: vec![delay; players],
            local_next: delay + 1,
            current: 0,
            used: BTreeMap::new(),
            snapshots,
            hashes,
            finalized: 0,
            journal: Vec::new(),
            events: Vec::new(),
            stats: SessionStats::default(),
            packet: Vec::with_capacity(MAX_DATAGRAM),
            config,
            sim: simulation,
            transport,
        })
    }

    pub fn simulation(&self) -> &S {
        &self.sim
    }
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }
    /// The last simulated frame (possibly predicted in rollback mode).
    pub fn current_tick(&self) -> Tick {
        Tick::new(self.current)
    }
    /// The last frame simulated with every player's real input.
    pub fn confirmed_tick(&self) -> Tick {
        Tick::new(self.finalized)
    }
    pub fn stats(&self) -> SessionStats {
        self.stats
    }
    pub fn take_events(&mut self) -> Vec<NetEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn transport(&self) -> &T {
        &self.transport
    }
    /// Confirmed frames with their inputs and hashes, replayable with
    /// `quasar_sim::Replayer`.
    pub fn journal(&self) -> Journal<Vec<I>> {
        Journal::new(self.config.simulation.clone(), self.journal.clone())
            .expect("confirmed frames are consecutive")
    }

    /// Exchanges datagrams without advancing (call while paused or
    /// waiting to keep peers informed).
    pub fn poll(&mut self) -> Result<(), NetError<S::Error>> {
        self.receive()?;
        self.check_timeouts();
        self.send()
    }

    /// Feeds this frame's local input and advances one frame when possible.
    /// Call once per fixed tick.
    pub fn advance(&mut self, local_input: I) -> Result<Advance, NetError<S::Error>> {
        self.receive()?;
        self.check_timeouts();
        let local = usize::from(self.config.local);
        let delay = u64::from(self.config.input_delay);
        // Queue the input unless we are already a full delay ahead (when
        // waiting, the input for that frame is already queued).
        if self.local_next <= self.current + 1 + delay {
            self.inputs[local].insert(self.local_next, local_input);
            self.local_next += 1;
            self.update_contiguous(local);
        }
        let mut result = Advance::default();
        match self.config.mode {
            SyncMode::Lockstep => {
                let next = self.current + 1;
                if self.contiguous.iter().all(|&c| c >= next) {
                    let inputs = self.real_inputs(next);
                    self.simulate(next, inputs, false)?;
                    self.current = next;
                    result.tick = Some(Tick::new(next));
                }
            }
            SyncMode::Rollback { max_prediction } => {
                result.resimulated = self.correct()?;
                let confirmed = self.contiguous.iter().copied().min().unwrap_or(0);
                let next = self.current + 1;
                if next <= confirmed + u64::from(max_prediction) {
                    let inputs = self.best_inputs(next);
                    self.simulate(next, inputs, true)?;
                    self.current = next;
                    result.tick = Some(Tick::new(next));
                }
            }
        }
        if result.tick.is_some() {
            self.stats.frames += 1;
        } else {
            self.stats.waits += 1;
        }
        self.finalize();
        self.prune();
        self.send()?;
        Ok(result)
    }

    fn update_contiguous(&mut self, player: usize) {
        let map = &self.inputs[player];
        let mut c = self.contiguous[player];
        while map.contains_key(&(c + 1)) {
            c += 1;
        }
        self.contiguous[player] = c;
    }

    fn real_inputs(&self, frame: u64) -> Vec<I> {
        self.inputs
            .iter()
            .map(|m| m.get(&frame).cloned().unwrap_or_default())
            .collect()
    }

    /// Real inputs where known, otherwise each player's latest known input.
    fn best_inputs(&self, frame: u64) -> Vec<I> {
        self.inputs
            .iter()
            .map(|m| {
                m.get(&frame)
                    .or_else(|| m.range(..frame).next_back().map(|(_, i)| i))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    }

    fn simulate(
        &mut self,
        frame: u64,
        inputs: Vec<I>,
        keep: bool,
    ) -> Result<(), NetError<S::Error>> {
        self.sim
            .update(Tick::new(frame), &inputs)
            .map_err(NetError::Simulation)?;
        self.hashes.insert(frame, self.sim.state_hash());
        if keep {
            self.snapshots.insert(frame, self.sim.snapshot());
        }
        self.used.insert(frame, inputs);
        Ok(())
    }

    /// Rolls back to the first frame whose used inputs differ from what is
    /// known now and resimulates up to the current frame.
    fn correct(&mut self) -> Result<u32, NetError<S::Error>> {
        let first_wrong = (self.finalized + 1..=self.current).find(|frame| {
            let used = &self.used[frame];
            self.inputs
                .iter()
                .zip(used)
                .any(|(m, u)| m.get(frame).is_some_and(|real| real != u))
        });
        let Some(start) = first_wrong else {
            return Ok(0);
        };
        let snapshot = self
            .snapshots
            .get(&(start - 1))
            .expect("snapshots kept since the last confirmed frame");
        self.sim.restore(snapshot).map_err(NetError::Simulation)?;
        let end = self.current;
        for frame in start..=end {
            let inputs = self.best_inputs(frame);
            self.simulate(frame, inputs, true)?;
        }
        let count = (end - start + 1) as u32;
        self.stats.rollbacks += 1;
        self.stats.resimulated += u64::from(count);
        Ok(count)
    }

    /// Moves frames simulated with all-real inputs into the journal and
    /// compares their hashes with peers'.
    fn finalize(&mut self) {
        let confirmed = self.contiguous.iter().copied().min().unwrap_or(0);
        while self.finalized < confirmed.min(self.current) {
            let frame = self.finalized + 1;
            let inputs = self.used[&frame].clone();
            debug_assert_eq!(inputs, self.real_inputs(frame));
            let hash = self.hashes[&frame];
            self.journal
                .push(TickRecord::new(Tick::new(frame), inputs, hash));
            self.finalized = frame;
            for peer in &mut self.peers {
                if let Some(remote) = peer.remote_hashes.remove(&frame)
                    && remote != hash.value()
                {
                    self.events.push(NetEvent::Desync {
                        tick: Tick::new(frame),
                        player: peer.player,
                        local: hash,
                        remote: StateHash::new(remote),
                    });
                }
            }
        }
    }

    fn prune(&mut self) {
        let keep_from = self.finalized.saturating_sub(1);
        self.snapshots = self.snapshots.split_off(&keep_from);
        self.used = self.used.split_off(&keep_from);
        // Hashes are resent for a while after finalizing.
        let hashes_from = self.finalized.saturating_sub(HASHES_PER_PACKET as u64);
        self.hashes = self.hashes.split_off(&hashes_from);
        let local = usize::from(self.config.local);
        let acked = self.peers.iter().map(|p| p.acked).min().unwrap_or(u64::MAX);
        for (player, map) in self.inputs.iter_mut().enumerate() {
            // Local inputs stay until every peer has them; keep the last
            // known input of each player for prediction.
            let limit = if player == local {
                acked.min(self.finalized)
            } else {
                self.finalized
            };
            let cut = limit.saturating_sub(1);
            *map = map.split_off(&cut);
        }
    }

    fn check_timeouts(&mut self) {
        let timeout = self.config.timeout;
        for peer in &mut self.peers {
            if peer.connected && peer.last_heard.elapsed() > timeout {
                peer.connected = false;
                self.events.push(NetEvent::Disconnected {
                    player: peer.player,
                });
            }
        }
    }

    fn send(&mut self) -> Result<(), NetError<S::Error>> {
        let local = usize::from(self.config.local);
        let latest = self.local_next - 1;
        for index in 0..self.peers.len() {
            let peer = &self.peers[index];
            let player = usize::from(peer.player);
            let first = peer.acked + 1;
            let room = (MAX_DATAGRAM - 32 - HASHES_PER_PACKET * 16) / I::SIZE;
            let count = latest.saturating_sub(first - 1).min(room as u64) as u16;
            self.packet.clear();
            self.packet.extend(MAGIC);
            self.packet.push(VERSION);
            self.packet.push(self.config.local);
            self.packet.extend(self.contiguous[player].to_le_bytes());
            self.packet.extend(first.to_le_bytes());
            self.packet.extend(count.to_le_bytes());
            let mut buffer = [0u8; 256];
            for frame in first..first + u64::from(count) {
                let input = self.inputs[local].get(&frame).cloned().unwrap_or_default();
                input.write(&mut buffer[..I::SIZE]);
                self.packet.extend_from_slice(&buffer[..I::SIZE]);
            }
            let recent: Vec<(u64, u64)> = self
                .hashes
                .range(..=self.finalized)
                .rev()
                .take(HASHES_PER_PACKET)
                .filter(|(frame, _)| **frame > 0)
                .map(|(f, h)| (*f, h.value()))
                .collect();
            self.packet.push(recent.len() as u8);
            for (frame, hash) in recent {
                self.packet.extend(frame.to_le_bytes());
                self.packet.extend(hash.to_le_bytes());
            }
            let addr = self.peers[index].addr.clone();
            self.transport
                .send(&addr, &self.packet)
                .map_err(NetError::Transport)?;
            self.stats.packets_sent += 1;
        }
        Ok(())
    }

    fn receive(&mut self) -> Result<(), NetError<S::Error>> {
        while let Some((from, bytes)) = self.transport.recv().map_err(NetError::Transport)? {
            if !self.accept(&from, &bytes) {
                self.stats.packets_rejected += 1;
            } else {
                self.stats.packets_received += 1;
            }
        }
        Ok(())
    }

    fn accept(&mut self, from: &T::Addr, bytes: &[u8]) -> bool {
        let mut reader = Reader(bytes);
        let Some(header) = reader.take(4) else {
            return false;
        };
        if header[..2] != MAGIC || header[2] != VERSION {
            return false;
        }
        let player = header[3];
        let Some(index) = self
            .peers
            .iter()
            .position(|p| p.player == player && p.addr == *from)
        else {
            return false;
        };
        let (Some(ack), Some(first), Some(count)) = (reader.u64(), reader.u64(), reader.u16())
        else {
            return false;
        };
        let Some(payload) = reader.take(usize::from(count) * I::SIZE) else {
            return false;
        };
        let Some(hash_count) = reader.take(1) else {
            return false;
        };
        let mut hashes = Vec::new();
        for _ in 0..hash_count[0] {
            let (Some(frame), Some(hash)) = (reader.u64(), reader.u64()) else {
                return false;
            };
            hashes.push((frame, hash));
        }
        let slot = usize::from(player);
        let known = self.contiguous[slot];
        for (i, chunk) in payload.chunks_exact(I::SIZE).enumerate() {
            let frame = first + i as u64;
            if frame > known && !self.inputs[slot].contains_key(&frame) {
                self.inputs[slot].insert(frame, I::read(chunk));
            }
        }
        self.update_contiguous(slot);
        let peer = &mut self.peers[index];
        peer.acked = peer.acked.max(ack.min(self.local_next - 1));
        peer.last_heard = Instant::now();
        if !peer.connected {
            peer.connected = true;
            self.events.push(NetEvent::Reconnected { player });
        }
        for (frame, hash) in hashes {
            if frame <= self.finalized {
                if let Some(local) = self.journal.iter().rev().find(|r| r.tick.value() == frame)
                    && local.hash.value() != hash
                    && !self.events.iter().any(|e| {
                        matches!(e, NetEvent::Desync { tick, player: p, .. } if tick.value() == frame && *p == player)
                    })
                {
                    self.events.push(NetEvent::Desync {
                        tick: Tick::new(frame),
                        player,
                        local: local.hash,
                        remote: StateHash::new(hash),
                    });
                }
            } else {
                self.peers[index].remote_hashes.insert(frame, hash);
            }
        }
        true
    }
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
}

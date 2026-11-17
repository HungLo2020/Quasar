//! Lockstep and rollback over lossy links, desync detection, timeouts and UDP.
use quasar_net::*;
use quasar_sim::{Simulation, SimulationConfig, StateHash, Tick};
use std::time::Duration;

/// Order-sensitive state so any input or ordering difference shows up.
#[derive(Clone, Default)]
struct Game {
    state: u64,
    /// Adds noise to one peer's simulation to provoke a desync.
    noise_at: Option<u64>,
}

impl Simulation for Game {
    type Input = Vec<u16>;
    type Snapshot = u64;
    type Error = std::convert::Infallible;
    fn update(&mut self, tick: Tick, inputs: &Vec<u16>) -> Result<(), Self::Error> {
        for (player, input) in inputs.iter().enumerate() {
            self.state = self
                .state
                .wrapping_mul(0x100_0000_01b3)
                .wrapping_add(u64::from(*input) << (player * 8))
                ^ tick.value();
        }
        if self.noise_at == Some(tick.value()) {
            self.state ^= 1;
        }
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        StateHash::new(self.state)
    }
    fn snapshot(&self) -> u64 {
        self.state
    }
    fn restore(&mut self, snapshot: &u64) -> Result<(), Self::Error> {
        self.state = *snapshot;
        Ok(())
    }
}

/// A player's scripted input: changes every few frames so predictions miss.
fn input(player: u8, frame: u64) -> u16 {
    ((frame / (3 + u64::from(player))) * (7 + u64::from(player)) % 17) as u16
}

fn sessions(
    mode: SyncMode,
    players: u8,
    delay: u32,
    conditions: LinkConditions,
    games: Vec<Game>,
) -> (MemoryNetwork, Vec<Session<u16, Game, MemoryTransport>>) {
    let network = MemoryNetwork::new(conditions, 42);
    let endpoints: Vec<_> = (0..players).map(|_| network.endpoint()).collect();
    let sessions = endpoints
        .into_iter()
        .zip(games)
        .enumerate()
        .map(|(local, (transport, game))| {
            let mut config = SessionConfig::new(
                players,
                local as u8,
                mode,
                SimulationConfig::new(1, 7, "net-test"),
            );
            config.input_delay = delay;
            let peers = (0..players)
                .filter(|p| *p != local as u8)
                .map(|p| (p, usize::from(p)))
                .collect();
            Session::new(config, game, transport, peers).unwrap()
        })
        .collect();
    (network, sessions)
}

/// Runs every session for `frames` local frames, then lets them drain.
fn run(network: &MemoryNetwork, sessions: &mut [Session<u16, Game, MemoryTransport>], frames: u64) {
    let mut fed = vec![0u64; sessions.len()];
    for _ in 0..frames * 6 {
        for (index, session) in sessions.iter_mut().enumerate() {
            let player = index as u8;
            let next = fed[index] + 1;
            let before = session.current_tick();
            if next <= frames {
                session.advance(input(player, next)).unwrap();
                if session.current_tick() > before
                    || fed[index] < u64::from(session.config().input_delay)
                {
                    fed[index] = next;
                }
            } else {
                session.advance(0).unwrap();
            }
        }
        network.step();
        if sessions
            .iter()
            .all(|s| s.confirmed_tick().value() >= frames)
        {
            break;
        }
    }
}

/// The reference: the confirmed inputs replayed without any network.
fn replay(journal: &quasar_sim::Journal<Vec<u16>>) -> u64 {
    let mut game = Game::default();
    for record in journal.records() {
        game.update(record.tick, &record.input).unwrap();
        assert_eq!(
            game.state_hash(),
            record.hash,
            "journal hash at {}",
            record.tick
        );
    }
    game.state
}

fn lossy() -> LinkConditions {
    LinkConditions {
        latency: 3,
        jitter: 3,
        loss: 0.2,
        duplicate: 0.1,
    }
}

#[test]
fn lockstep_peers_agree_over_a_lossy_reordering_link() {
    let (network, mut sessions) =
        sessions(SyncMode::Lockstep, 3, 4, lossy(), vec![Game::default(); 3]);
    run(&network, &mut sessions, 240);
    let journals: Vec<_> = sessions.iter().map(|s| s.journal()).collect();
    assert!(journals[0].len() >= 240);
    let n = journals.iter().map(|j| j.len()).min().unwrap();
    for j in &journals[1..] {
        assert_eq!(
            j.records()[..n],
            journals[0].records()[..n],
            "same frames everywhere"
        );
    }
    replay(&journals[0]);
    for s in &mut sessions {
        assert!(s.take_events().is_empty(), "no desync");
        assert_eq!(s.stats().rollbacks, 0);
    }
    let (sent, dropped) = network.stats();
    assert!(dropped > sent / 10, "the link really lost packets");
    // Recorded inputs are each player's real inputs.
    let record = &journals[0].records()[100];
    for player in 0..3u8 {
        assert_eq!(record.input[usize::from(player)], input(player, 100 - 4));
    }
}

#[test]
fn rollback_resimulates_mispredictions_and_matches_lockstep() {
    let (network, mut sessions) = sessions(
        SyncMode::Rollback { max_prediction: 12 },
        2,
        1,
        lossy(),
        vec![Game::default(); 2],
    );
    run(&network, &mut sessions, 300);
    let stats = sessions[0].stats();
    assert!(
        stats.rollbacks > 10,
        "predictions were corrected: {stats:?}"
    );
    assert!(stats.resimulated >= stats.rollbacks);
    let a = sessions[0].journal();
    let b = sessions[1].journal();
    let n = a.len().min(b.len());
    assert!(n >= 300);
    assert_eq!(a.records()[..n], b.records()[..n]);
    // The rolled-back state equals a plain replay of the confirmed inputs.
    let reference = replay(&a);
    for session in &mut sessions {
        if session.confirmed_tick() == session.current_tick() && session.journal().len() == a.len()
        {
            assert_eq!(session.simulation().state, reference);
        }
        assert!(session.take_events().is_empty());
    }
}

#[test]
fn rollback_never_runs_further_ahead_than_allowed() {
    // Peer 1 never advances: peer 0 may predict 5 frames, then waits.
    let (network, mut sessions) = sessions(
        SyncMode::Rollback { max_prediction: 5 },
        2,
        0,
        LinkConditions::default(),
        vec![Game::default(); 2],
    );
    for frame in 1..=20 {
        sessions[0].advance(input(0, frame)).unwrap();
        network.step();
    }
    assert_eq!(sessions[0].current_tick(), Tick::new(5));
    assert_eq!(sessions[0].confirmed_tick(), Tick::ZERO);
    assert!(sessions[0].stats().waits >= 15);
}

#[test]
fn desyncs_name_the_first_differing_tick() {
    let mut games = vec![Game::default(); 2];
    games[1].noise_at = Some(57);
    let (network, mut sessions) =
        sessions(SyncMode::Lockstep, 2, 2, LinkConditions::default(), games);
    run(&network, &mut sessions, 100);
    for (index, session) in sessions.iter_mut().enumerate() {
        let events = session.take_events();
        let first = events
            .iter()
            .find_map(|e| match e {
                NetEvent::Desync { tick, player, .. } => Some((*tick, *player)),
                _ => None,
            })
            .expect("desync reported");
        assert_eq!(first.0, Tick::new(57), "peer {index}");
        assert_eq!(first.1, 1 - index as u8);
    }
}

#[test]
fn silent_peers_are_reported_once_and_garbage_is_rejected() {
    let (network, mut sessions) = sessions(
        SyncMode::Lockstep,
        2,
        2,
        LinkConditions::default(),
        vec![Game::default(); 2],
    );
    let mut alone = sessions.remove(0);
    let mut config = alone.config().clone();
    config.timeout = Duration::from_millis(20);
    drop(alone);
    let mut transport = network.endpoint();
    let address = transport.address();
    let mut stray = network.endpoint();
    alone = Session::new(
        config,
        Game::default(),
        network.endpoint(),
        vec![(1, address)],
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(40));
    alone.poll().unwrap();
    alone.poll().unwrap();
    let events = alone.take_events();
    assert_eq!(events, vec![NetEvent::Disconnected { player: 1 }]);
    // Packets from unknown addresses or with a bad header are ignored.
    let target = alone.transport().address();
    stray.send(&target, b"QN\x01\x01garbage").unwrap();
    transport.send(&target, b"nonsense").unwrap();
    network.step();
    alone.poll().unwrap();
    assert_eq!(alone.stats().packets_rejected, 2);
    assert_eq!(alone.stats().packets_received, 0);
    assert!(
        Session::new(
            SessionConfig::new(2, 2, SyncMode::Lockstep, SimulationConfig::new(1, 0, "x")),
            Game::default(),
            network.endpoint(),
            vec![],
        )
        .is_err()
    );
}

#[test]
fn udp_peers_on_localhost_stay_in_lockstep() {
    let a = UdpTransport::bind("127.0.0.1:0").unwrap();
    let b = UdpTransport::bind("127.0.0.1:0").unwrap();
    let (addr_a, addr_b) = (a.local_addr().unwrap(), b.local_addr().unwrap());
    let config = |local| {
        SessionConfig::new(
            2,
            local,
            SyncMode::Lockstep,
            SimulationConfig::new(1, 0, "udp"),
        )
    };
    let mut one = Session::new(config(0), Game::default(), a, vec![(1, addr_b)]).unwrap();
    let mut two = Session::new(config(1), Game::default(), b, vec![(0, addr_a)]).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let (mut fed_one, mut fed_two) = (1u64, 1u64);
    while one.confirmed_tick().value() < 120 || two.confirmed_tick().value() < 120 {
        assert!(std::time::Instant::now() < deadline, "UDP sessions stalled");
        if one.advance(input(0, fed_one)).unwrap().tick.is_some() {
            fed_one += 1;
        }
        if two.advance(input(1, fed_two)).unwrap().tick.is_some() {
            fed_two += 1;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    let n = 120;
    assert_eq!(one.journal().records()[..n], two.journal().records()[..n]);
}

//! Two pilots on a 1D track, synchronised by rollback over UDP.
//!
//! ```sh
//! # Both peers in one process on localhost:
//! cargo run -p quasar-net --example net_duel
//! # Or across two machines (player 0 hosts):
//! cargo run -p quasar-net --example net_duel -- --local 0 --bind 0.0.0.0:7700 --peer OTHER:7701
//! cargo run -p quasar-net --example net_duel -- --local 1 --bind 0.0.0.0:7701 --peer HOST:7700
//! ```
use quasar_net::{NetInput, Session, SessionConfig, SyncMode, UdpTransport};
use quasar_sim::{Simulation, SimulationConfig, StateHash, StateHasher, Tick};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// One frame of one pilot's controls: -1 / 0 / +1 thrust and a fire flag.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Controls {
    thrust: i8,
    fire: bool,
}

impl NetInput for Controls {
    const SIZE: usize = 2;
    fn write(&self, out: &mut [u8]) {
        out[0] = self.thrust as u8;
        out[1] = u8::from(self.fire);
    }
    fn read(bytes: &[u8]) -> Self {
        Self {
            thrust: bytes[0] as i8,
            fire: bytes[1] != 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Duel {
    /// Positions and velocities in fixed point (1/256 units).
    position: [i32; 2],
    velocity: [i32; 2],
    hits: [u32; 2],
}

impl Simulation for Duel {
    type Input = Vec<Controls>;
    type Snapshot = Duel;
    type Error = std::convert::Infallible;
    fn update(&mut self, _: Tick, inputs: &Vec<Controls>) -> Result<(), Self::Error> {
        for (p, input) in inputs.iter().enumerate() {
            self.velocity[p] = (self.velocity[p] + i32::from(input.thrust) * 8) * 15 / 16;
            self.position[p] = (self.position[p] + self.velocity[p]).clamp(-25_600, 25_600);
        }
        for (p, input) in inputs.iter().enumerate() {
            if input.fire && (self.position[p] - self.position[1 - p]).abs() < 2_560 {
                self.hits[p] += 1;
            }
        }
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        let mut h = StateHasher::new();
        for v in self.position.iter().chain(&self.velocity) {
            h.write_i64(i64::from(*v));
        }
        for v in self.hits {
            h.write_u32(v);
        }
        h.finish()
    }
    fn snapshot(&self) -> Duel {
        self.clone()
    }
    fn restore(&mut self, snapshot: &Duel) -> Result<(), Self::Error> {
        *self = snapshot.clone();
        Ok(())
    }
}

/// A scripted pilot standing in for real input.
fn pilot(player: u8, frame: u64) -> Controls {
    let phase = (frame / (20 + u64::from(player) * 7)) % 3;
    Controls {
        thrust: phase as i8 - 1,
        fire: frame.is_multiple_of(11 + u64::from(player)),
    }
}

fn config(local: u8) -> SessionConfig {
    let mut config = SessionConfig::new(
        2,
        local,
        SyncMode::Rollback { max_prediction: 8 },
        SimulationConfig::new(1, 0, "net-duel"),
    );
    config.input_delay = 1;
    config
}

fn run(sessions: &mut [Session<Controls, Duel, UdpTransport>], frames: u64) {
    let started = Instant::now();
    let mut next = Instant::now();
    let mut fed = vec![1u64; sessions.len()];
    while sessions.iter().any(|s| s.confirmed_tick().value() < frames) {
        for session in sessions.iter_mut() {
            let local = session.config().local;
            let i = usize::from(local).min(fed.len() - 1);
            if session
                .advance(pilot(local, fed[i]))
                .expect("session")
                .tick
                .is_some()
            {
                fed[i] += 1;
            }
            for event in session.take_events() {
                println!("player {local}: {event:?}");
            }
        }
        next += Duration::from_micros(16_667);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        if started.elapsed() > Duration::from_secs(60) {
            eprintln!("timed out waiting for the peer");
            std::process::exit(1);
        }
    }
    for session in sessions {
        let duel = session.simulation();
        let stats = session.stats();
        println!(
            "player {} confirmed={} positions={:?} hits={:?} hash={} rollbacks={} resimulated={}",
            session.config().local,
            session.confirmed_tick(),
            duel.position,
            duel.hits,
            session.journal().records()[frames as usize - 1].hash,
            stats.rollbacks,
            stats.resimulated,
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let frames = 600;
    if let (Some(local), Some(bind), Some(peer)) =
        (value("--local"), value("--bind"), value("--peer"))
    {
        let local: u8 = local.parse().expect("--local 0 or 1");
        let peer: SocketAddr = peer.parse().expect("--peer HOST:PORT");
        let transport = UdpTransport::bind(bind).expect("bind");
        let session = Session::new(
            config(local),
            Duel::default(),
            transport,
            vec![(1 - local, peer)],
        )
        .expect("session");
        run(&mut [session], frames);
        return;
    }
    let a = UdpTransport::bind("127.0.0.1:0").expect("bind");
    let b = UdpTransport::bind("127.0.0.1:0").expect("bind");
    let (addr_a, addr_b) = (a.local_addr().unwrap(), b.local_addr().unwrap());
    let one = Session::new(config(0), Duel::default(), a, vec![(1, addr_b)]).expect("session");
    let two = Session::new(config(1), Duel::default(), b, vec![(0, addr_a)]).expect("session");
    run(&mut [one, two], frames);
}

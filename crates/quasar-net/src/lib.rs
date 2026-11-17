//! Peer-to-peer input synchronisation for deterministic `quasar-sim` games.
//!
//! A [`Session`] runs one game simulation per machine and exchanges only
//! player inputs, never state: every peer feeds the same inputs to the same
//! deterministic [`Simulation`](quasar_sim::Simulation), so every peer
//! computes the same frames. Two modes:
//!
//! * [`SyncMode::Lockstep`]: a frame runs once every player's input for it
//!   has arrived. Exact and simple; with an input delay at least as long as
//!   the round trip it never waits.
//! * [`SyncMode::Rollback`]: frames run immediately on predicted remote
//!   inputs (each player's last input); when a real input differs, the
//!   session restores the snapshot before that frame and resimulates.
//!   Local input feels instant; the simulation must restore cheaply.
//!
//! Every packet repeats all inputs the peer has not acknowledged, so lost,
//! duplicated and reordered datagrams need no separate reliability layer.
//! Peers also exchange the state hash of each confirmed frame; a mismatch is
//! reported as [`NetEvent::Desync`] with the first differing tick, and the
//! session's [`Session::journal`] replays exactly the confirmed frames for
//! debugging.
//!
//! Transports are pluggable ([`Transport`]): [`UdpTransport`] for real
//! networks and [`MemoryNetwork`] for tests and same-process play, with
//! seeded latency, jitter, loss and duplication. No threads are created;
//! the session does its I/O inside [`Session::advance`] and
//! [`Session::poll`].
//!
//! ```
//! use quasar_net::{LinkConditions, MemoryNetwork, Session, SessionConfig, SyncMode};
//! use quasar_sim::{Simulation, SimulationConfig, StateHash, Tick};
//!
//! #[derive(Default)]
//! struct Sum(u64);
//! impl Simulation for Sum {
//!     type Input = Vec<u8>;
//!     type Snapshot = u64;
//!     type Error = std::convert::Infallible;
//!     fn update(&mut self, _: Tick, inputs: &Vec<u8>) -> Result<(), Self::Error> {
//!         self.0 = self.0.wrapping_mul(31) + inputs.iter().map(|&i| u64::from(i)).sum::<u64>();
//!         Ok(())
//!     }
//!     fn state_hash(&self) -> StateHash { StateHash::new(self.0) }
//!     fn snapshot(&self) -> u64 { self.0 }
//!     fn restore(&mut self, s: &u64) -> Result<(), Self::Error> { self.0 = *s; Ok(()) }
//! }
//!
//! let network = MemoryNetwork::new(LinkConditions::default(), 1);
//! let (a, b) = (network.endpoint(), network.endpoint());
//! let config = |local| SessionConfig::new(2, local, SyncMode::Lockstep, SimulationConfig::new(1, 0, "doc"));
//! let mut one = Session::new(config(0), Sum::default(), a, vec![(1, 1)]).unwrap();
//! let mut two = Session::new(config(1), Sum::default(), b, vec![(0, 0)]).unwrap();
//! for frame in 0..30u8 {
//!     one.advance(frame).unwrap();
//!     two.advance(frame * 2).unwrap();
//!     network.step();
//! }
//! assert_eq!(one.simulation().0, two.simulation().0);
//! ```
mod session;
mod transport;

pub use session::{Advance, NetError, NetEvent, Session, SessionConfig, SessionStats, SyncMode};
pub use transport::{
    LinkConditions, MAX_DATAGRAM, MemoryNetwork, MemoryTransport, Transport, UdpTransport,
};

/// A player's input for one frame, with a fixed-size wire encoding.
///
/// `Default` is the input of frames before the input delay and of players
/// whose input is unknown. Equality decides whether a prediction was right,
/// so keep inputs quantised (buttons, stick values as integers).
pub trait NetInput: Clone + PartialEq + Default + std::fmt::Debug {
    /// Encoded size in bytes (1..=256).
    const SIZE: usize;
    /// Writes exactly [`Self::SIZE`] bytes.
    fn write(&self, out: &mut [u8]);
    /// Reads from exactly [`Self::SIZE`] bytes.
    fn read(bytes: &[u8]) -> Self;
}

macro_rules! int_input {
    ($($t:ty),*) => {$(
        impl NetInput for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn write(&self, out: &mut [u8]) {
                out.copy_from_slice(&self.to_le_bytes());
            }
            fn read(bytes: &[u8]) -> Self {
                <$t>::from_le_bytes(bytes.try_into().expect("input size"))
            }
        }
    )*};
}
int_input!(u8, u16, u32, u64, i8, i16, i32, i64);

impl<const N: usize> NetInput for [u8; N]
where
    [u8; N]: Default,
{
    const SIZE: usize = N;
    fn write(&self, out: &mut [u8]) {
        out.copy_from_slice(self);
    }
    fn read(bytes: &[u8]) -> Self {
        bytes.try_into().expect("input size")
    }
}

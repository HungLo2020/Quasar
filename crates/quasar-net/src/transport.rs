//! Datagram transports: UDP and an in-memory network for tests and local
//! play.
use std::collections::VecDeque;
use std::fmt::Debug;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};

/// Largest datagram sessions send; fits common MTUs without fragmentation.
pub const MAX_DATAGRAM: usize = 1200;

/// Unreliable, unordered datagrams to addressed peers. Sessions tolerate
/// loss, duplication and reordering.
pub trait Transport {
    type Addr: Clone + Eq + Debug;
    fn send(&mut self, to: &Self::Addr, bytes: &[u8]) -> io::Result<()>;
    /// The next received datagram, or `None` when nothing is waiting.
    /// Never blocks.
    fn recv(&mut self) -> io::Result<Option<(Self::Addr, Vec<u8>)>>;
}

/// Non-blocking UDP.
pub struct UdpTransport {
    socket: UdpSocket,
    buffer: Vec<u8>,
}

impl UdpTransport {
    /// Binds a non-blocking socket, for example to `"0.0.0.0:7777"` or
    /// `"127.0.0.1:0"` (any free port).
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            buffer: vec![0; 64 * 1024],
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Transport for UdpTransport {
    type Addr = SocketAddr;
    fn send(&mut self, to: &SocketAddr, bytes: &[u8]) -> io::Result<()> {
        match self.socket.send_to(bytes, to) {
            Ok(_) => Ok(()),
            // A full send buffer drops the datagram, as the network may.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }
    fn recv(&mut self) -> io::Result<Option<(SocketAddr, Vec<u8>)>> {
        loop {
            match self.socket.recv_from(&mut self.buffer) {
                Ok((len, from)) => return Ok(Some((from, self.buffer[..len].to_vec()))),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                // ICMP "port unreachable" from a peer that left: ignore.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

/// Simulated link behaviour for [`MemoryNetwork`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkConditions {
    /// Network steps ([`MemoryNetwork::step`]) before a datagram arrives.
    pub latency: u32,
    /// Extra random delay of up to this many steps (reorders datagrams).
    pub jitter: u32,
    /// Probability of dropping each datagram.
    pub loss: f32,
    /// Probability of delivering a datagram twice.
    pub duplicate: f32,
}

impl Default for LinkConditions {
    fn default() -> Self {
        Self {
            latency: 0,
            jitter: 0,
            loss: 0.,
            duplicate: 0.,
        }
    }
}

struct InFlight {
    deliver_at: u64,
    from: usize,
    to: usize,
    bytes: Vec<u8>,
}

struct NetworkState {
    now: u64,
    conditions: LinkConditions,
    random: u64,
    in_flight: Vec<InFlight>,
    inboxes: Vec<VecDeque<(usize, Vec<u8>)>>,
    sent: u64,
    dropped: u64,
}

impl NetworkState {
    fn chance(&mut self) -> f32 {
        // xorshift64*
        self.random ^= self.random >> 12;
        self.random ^= self.random << 25;
        self.random ^= self.random >> 27;
        (self.random.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    }
    fn deliver_due(&mut self) {
        let now = self.now;
        let mut index = 0;
        while index < self.in_flight.len() {
            if self.in_flight[index].deliver_at <= now {
                let packet = self.in_flight.swap_remove(index);
                self.inboxes[packet.to].push_back((packet.from, packet.bytes));
            } else {
                index += 1;
            }
        }
    }
}

/// A deterministic in-process network: endpoints exchange datagrams with
/// configurable latency, jitter, loss and duplication (seeded, so failures
/// reproduce).
#[derive(Clone)]
pub struct MemoryNetwork {
    state: Arc<Mutex<NetworkState>>,
}

impl MemoryNetwork {
    pub fn new(conditions: LinkConditions, seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(NetworkState {
                now: 0,
                conditions,
                random: seed | 1,
                in_flight: Vec::new(),
                inboxes: Vec::new(),
                sent: 0,
                dropped: 0,
            })),
        }
    }
    /// A new endpoint; its address is its index.
    pub fn endpoint(&self) -> MemoryTransport {
        let mut state = self.state.lock().expect("network lock");
        state.inboxes.push(VecDeque::new());
        MemoryTransport {
            network: self.clone(),
            address: state.inboxes.len() - 1,
        }
    }
    /// Advances simulated time by one step and delivers due datagrams.
    pub fn step(&self) {
        let mut state = self.state.lock().expect("network lock");
        state.now += 1;
        state.deliver_due();
    }
    pub fn set_conditions(&self, conditions: LinkConditions) {
        self.state.lock().expect("network lock").conditions = conditions;
    }
    /// `(sent, dropped)` datagram counts.
    pub fn stats(&self) -> (u64, u64) {
        let state = self.state.lock().expect("network lock");
        (state.sent, state.dropped)
    }
}

/// One endpoint of a [`MemoryNetwork`].
pub struct MemoryTransport {
    network: MemoryNetwork,
    address: usize,
}

impl MemoryTransport {
    pub fn address(&self) -> usize {
        self.address
    }
}

impl Transport for MemoryTransport {
    type Addr = usize;
    fn send(&mut self, to: &usize, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.network.state.lock().expect("network lock");
        if *to >= state.inboxes.len() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "unknown endpoint"));
        }
        state.sent += 1;
        let conditions = state.conditions;
        if state.chance() < conditions.loss {
            state.dropped += 1;
            return Ok(());
        }
        let copies = if state.chance() < conditions.duplicate {
            2
        } else {
            1
        };
        for _ in 0..copies {
            let jitter = if conditions.jitter > 0 {
                (state.chance() * (conditions.jitter + 1) as f32) as u64
            } else {
                0
            };
            let deliver_at = state.now + u64::from(conditions.latency) + jitter;
            state.in_flight.push(InFlight {
                deliver_at,
                from: self.address,
                to: *to,
                bytes: bytes.to_vec(),
            });
        }
        state.deliver_due();
        Ok(())
    }
    fn recv(&mut self) -> io::Result<Option<(usize, Vec<u8>)>> {
        let mut state = self.network.state.lock().expect("network lock");
        Ok(state.inboxes[self.address].pop_front())
    }
}

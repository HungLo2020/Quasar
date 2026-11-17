# Networking

`quasar-net` synchronises a deterministic `quasar-sim` game between peers by
exchanging player inputs only. Every machine runs the same
[`Simulation`](../crates/quasar-sim/src/lib.rs) on the same inputs, so they
compute the same frames without sending state.

## Sessions

```rust
let mut config = SessionConfig::new(2, local_player, SyncMode::Lockstep, sim_config);
config.input_delay = 3;
let mut session = Session::new(config, game, UdpTransport::bind("0.0.0.0:7700")?, peers)?;
// once per fixed tick:
let advance = session.advance(my_input)?;
for event in session.take_events() { /* Desync, Disconnected, Reconnected */ }
```

The simulation's input is `Vec<I>`: every player's input for the frame,
indexed by player. `I` implements `NetInput` (a fixed-size wire encoding;
integers and byte arrays already do).

- **Lockstep** runs a frame once every player's input for it has arrived.
  With an `input_delay` of at least the round trip in frames it never
  waits; local input lags by `input_delay` frames.
- **Rollback** runs frames immediately on predicted remote inputs (each
  player's last input). When a real input arrives that differs, the session
  restores the snapshot before that frame and resimulates up to the present.
  `max_prediction` caps how far it may run ahead of the confirmed inputs.
  `Simulation::snapshot` and `restore` must be cheap.

`Session::current_tick` is the latest simulated frame (possibly predicted);
`Session::confirmed_tick` is the latest frame simulated with every real
input.

## Reliability and desyncs

Datagrams carry every input the receiver has not acknowledged, so loss,
duplication and reordering need no separate reliability layer. Each packet
also carries the state hashes of recent confirmed frames; a mismatch raises
`NetEvent::Desync` with the first differing tick. `Session::journal`
returns the confirmed frames (inputs plus hashes) as a `quasar_sim::Journal`
for replay and divergence reports.

A peer silent for `SessionConfig::timeout` is reported once as
`Disconnected`, and as `Reconnected` when it is heard again. No threads are
created: all I/O happens inside `advance` and `poll` (call `poll` while
paused to keep peers informed).

## Transports

- `UdpTransport`: non-blocking UDP; datagrams stay under `MAX_DATAGRAM`
  (1200 bytes).
- `MemoryNetwork`: in-process endpoints with seeded latency, jitter, loss
  and duplication. Tests use it to prove that peers agree over bad links.

Implement `Transport` for other carriers (relays, platform sockets).

## Example and tests

`cargo run -p quasar-net --example net_duel` runs a two-player rollback duel
over localhost UDP (or between machines with `--local`, `--bind`,
`--peer`). The crate tests cover lockstep and rollback over a 20%-loss,
reordering, duplicating link, prediction limits, desync reporting, timeouts,
malformed packets and real UDP sockets.

Distributing a networked game through Basalt needs a Basalt rule that
allows network access; that lives in Basalt, not in Quasar.

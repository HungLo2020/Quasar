# Deterministic simulation, snapshots and replay

Issue #41 is implemented by the independently usable `quasar-sim` crate.
It coordinates caller-owned simulations; it does not define game state, rules,
an ECS, networking or a renderer.

## Existing replay audit

Read-only audit of the current games:

- MissileCommand records JSON frames and a seed, checks replay version `1`,
  applies exactly one recorded input per simulation tick, and explicitly stops
  on replay exhaustion (`replay_exhaustion_does_not_apply_an_extra_tick`). It
  retains deterministic action-edge tests.
- MicroMages records per-frame button masks with stage/player/difficulty context,
  rejects empty replays, limits playback to recorded frames, and can run the same
  session headlessly.
- Neither game records expected state hashes or a timeline identity. `quasar-sim`
  adds those while leaving input representation and state ownership to the game.

`quasar-sim` follows the same exhaustion rule: `Replay::step` returns
`ReplayError::Exhausted` rather than applying a synthetic extra tick.

## Architecture

### Simulation boundary

`Simulation` has four game-owned responsibilities:

1. apply one input at one `Tick`;
2. return a stable `StateHash`;
3. produce an associated `Snapshot`;
4. restore that snapshot.

The input, snapshot and error types are associated, so no serialization format,
struct layout or ECS is imposed. A game can use raw structs, byte buffers or
engine-specific snapshots.

### Recording

`Recorder` applies inputs through the simulation, hashes after each update and
appends `TickRecord { tick, input, hash }`. Journals validate consecutive ticks
starting at `1`. `Recorder::snapshot` captures tick/hash/config plus the game
snapshot; `restore` rejects snapshots from another timeline/config or with a
tampered hash, restores the game value and truncates the journal so later ticks
continue sequentially.

### Replay and divergence

`Replay` applies journal inputs in order and compares each expected hash with the
actual hash after the update. The first mismatch becomes `Divergence`, containing
the tick, input index, expected/actual hashes and timeline metadata. Replaying a
diverged journal again returns the same report. `seek(tick)` restores the initial
snapshot and replays forward, verifying every intermediate hash.

`Replay::with_expected_config` rejects version, timeline or seed mismatches before
playback. `Replay::new` is available when those checks are intentionally
bypassed.

### Tick coordination and render schedules

`ScheduledSimulation` wraps the existing `quasar_core::FixedStep`; it does not
implement a second accumulator. `advance(elapsed, input_provider)` applies every
due fixed tick, while `advance_ticks(n, ...)` is the deterministic headless
path. Rendering can run at any independent rate because it is not part of this
crate. Pause/resume uses `clock_mut().reset()`.

### Seeds, versions and timelines

`SimulationConfig` records a numeric simulation version, a seed and a timeline
id. The game initializes its RNG from the seed; `quasar-sim` does not create or
own randomness. Version/timeline/seed checks are explicit replay errors. Journals
can be serialized with the optional `serde` feature; the format is otherwise
caller-selected.

### State hashing

`StateHasher` provides FNV-1a over bytes and common scalar encodings. Games can
instead supply any stable `StateHash`. Determinism is only as strong as the
game's state/input/RNG and hash implementation. This crate does not claim
cross-CPU floating-point bitwise parity; games should quantize or test platform
behavior explicitly.

## Benchmarks

A local Linux x86_64 release run on an Intel i7-10750H (rustc 1.94.0) with the
sample 500,000-tick `u64` counter simulation produced:

| Operation | Time | Throughput |
|---|---:|---:|
| Record + hash per tick | 7.36 ms | ~67.9M ticks/s |
| Replay + compare per tick | 4.18 ms | ~119.7M ticks/s |
| Restore fixed snapshot | 1.15 ms / 100k | ~86.6M restores/s |

The restore figure measures repeated restoration of one already-current
snapshot and is an overhead probe, not realistic rollback cost with journal
truncation or large game states. The record/replay figures include a tiny hash;
game hashing/snapshotting dominates real workloads. A 500,000-record
`TickRecord<u64>` journal is about 24 bytes/record (~11.7 MiB payload), plus
vector capacity and any retained snapshots/keyframes. Memory grows linearly with
the journal.

Run:

```sh
cargo run --release -p quasar-sim --example sim_benchmark
cargo run -p quasar-sim --example replay
```

## Optional integrations

- `serde`: journal, summary and divergence serialization.
- `jobs`: `Replay::into_jobs` moves a replay to a caller-owned `quasar-jobs`
  worker. Networking/rollback and an ECS remain optional and external.
- `quasar-core`: only the existing `FixedStep` timing type is reused.
- No window, renderer, audio, asset or app-runner dependency.
- No `quasar-net` crate exists in this workspace; rollback networking is not
  forced or implemented here.

## Acceptance criteria

- Headless record of N ticks and exact N-update replay with matching final hash.
- First divergence identifies its tick and expected/actual hashes.
- Empty and exhausted replays return explicit outcomes without extra updates.
- Snapshot restore returns to the exact tick/hash and truncates later records.
- Cross-timeline/config snapshots and tampered hashes are rejected.
- Version, timeline and seed mismatches are explicit errors.
- Seeking restores and replays forward with hash verification.
- Invalid/duplicate/non-sequential journal ordering is rejected.
- Seed/version/timeline metadata is recorded and serializable.
- Fixed-step scheduling reuses `quasar-core::FixedStep`; headless tick advancement
  is deterministic.
- Optional serde and jobs integrations build independently.
- No mandatory ECS, renderer, window, network or runner dependency.

## Limitations

- Determinism is game-owned; cross-CPU floating-point bitwise parity is not
  claimed or tested.
- Seeking is O(n) from the initial snapshot; no automatic keyframe index.
- Journal replay clones inputs each tick. Use compact IDs/handles for large data.
- Snapshots are not automatically serialized or versioned; that is a game codec
  concern.
- The sample restore benchmark is an overhead probe, not a large-state rollback
  benchmark.
- `ReplayError` itself is not serde-serializable because it can retain a boxed
  error; `Divergence` and `ReplaySummary` are the machine-readable reports.
- No rollback networking implementation; only optional job-based replay offload
  is included.

# Fixed-step timing

`quasar-core::FixedStep` remains the small, device-independent accumulator used by
Pong, Tilequest, MissileCommand, MicroMages and Quasar-Snake. No application
runner, renderer, ECS or global clock is required.

## API and compatibility

`FixedStep::new(hz, max_steps)` keeps its historical behavior:

- step derived from `hz`;
- `advance` clamps accumulation to `step * max_steps`;
- excess elapsed time is dropped;
- `take_tick`, `step`, `until_next_tick`, `interpolation` and `reset` retain
  their existing signatures.

New opt-in policy:

```rust
let mut clock = FixedStep::with_config(FixedStepConfig {
    hz: 60.0,
    max_catch_up_ticks: 4,
    max_elapsed: Some(Duration::from_millis(250)),
    backlog: BacklogPolicy::Preserve,
})?;
let report = clock.advance_with_report(elapsed);
while clock.take_tick() {
    // at most four ticks after this advance
}
```

- `max_elapsed` clamps one input interval before accumulation and reports the
  clamped duration. This is useful after suspend/debugger pauses.
- `max_catch_up_ticks` bounds ticks returned after one `advance` call.
- `BacklogPolicy::Drop` reproduces the historical discard behavior.
- `BacklogPolicy::Preserve` retains excess time beyond the catch-up budget for
  later advances; time remains exact in the `Duration` accumulator.
- `AdvanceReport` exposes accepted/clamped/dropped time and retained backlog.
- `dropped_total` and `clamped_total` provide cumulative accounting.
- `from_step(Duration, max)` allows an exact integer-nanosecond step, avoiding
  float-rate rounding when reproducing a legacy frame duration.

`advance` still returns `()` for source compatibility. Use
`advance_with_report` when accounting is needed.

## MicroMages comparison

The reference MicroMages loop uses:

```rust
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 60);
```

It schedules the next `about_to_wait` deadline at `start + FRAME`, runs one
simulation frame when the deadline is reached, and does not catch up missed
frames. The current MicroMages tree also has a 250 ms stall clamp and eight-step
limit around `quasar-core::FixedStep`.

The `quasar-core` regression
`micromages_steady_60hz_deadline_sequence_matches_fixed_step` compares a modeled
deadline loop and `FixedStep::from_step(FRAME, 1)` over 600 identical steady/slow
frame durations. Both produce 600 ticks. Exact 60 Hz behavior is therefore
reproducible with the exact truncated `FRAME` step and one-catch-up budget.
Under faster-than-step or highly jittered frames the two models intentionally
diverge: MicroMages follows its wall-clock deadline and can slip a frame, while
Quasar's fixed-step accumulator preserves fractional remainder and catches up
within its configured budget. Use `FixedStep` for simulation determinism and the
application's frame deadline for presentation pacing.

## Relationship to issue #41

Issue #41's simulation/replay coordination can reuse this timing type and its
reports. This change deliberately does not add a `quasar-sim` crate, tick journal,
state hashing, snapshots or replay infrastructure; those belong to that separate
coordination layer so timing is not duplicated.

## Local overhead measurement

`cargo run --release -p quasar-core --example timing_overhead` exercises the
hot path for 2,000,000 frames. On this Linux x86_64 host (12-thread Intel
i7-10750H, rustc 1.94.0), one local run reported:

| Path | Time | Notes |
|---|---:|---|
| `advance` + consume ticks | 12.74 ns/frame | one 60 Hz tick, no report |
| `advance_with_report` | 17.82 ns/frame | report struct returned and read |
| 250 ms clamp + drop | 28.60 ns/frame | 400 ms input clamped, eight ticks |
| `Preserve` backlog | 15.94 ns/frame | quarter-step frames |

These are single-host probe observations, not a portable guarantee. `Duration`
arithmetic dominates; there is no allocation, lock, clock read or platform call
in the hot path. A frame budget of 16.7 ms leaves this below 0.0002% of budget.

## Acceptance criteria

- `FixedStep::new` and its default policy remain behaviorally identical to the
  pre-change accumulator for valid rates and arbitrary elapsed sequences.
- Fractional rates retain sub-step remainder without drift over long runs.
- `max_elapsed` bounds a single suspended interval and is reported separately
  from catch-up drops.
- `max_catch_up_ticks` bounds work per `advance` under sustained overload.
- `Drop` discards and reports excess; `Preserve` retains it without exceeding
  the per-advance tick budget.
- `reset` clears backlog and restores the catch-up budget after pause/resume.
- `from_step` reproduces an exact legacy step such as MicroMages' truncated
  60 Hz frame.
- No window, renderer, ECS, global clock or background thread is required.

## Tests and limitations

```sh
cargo test -p quasar-core
cargo run --release -p quasar-core --example timing_overhead
cargo clippy -p quasar-core --all-targets -- -D warnings
```

Tests cover fractional rates, backward-compatible defaults, catch-up bounds,
elapsed clamping, preserved backlog, per-advance budget, reports, interpolation,
reset and the MicroMages comparison. A deterministic xorshift driver feeds
20,000 randomized stalls (including 250 ms clamps) and sustained overload while
asserting no more than eight ticks per advance and bounded backlog. A 59.94 Hz,
one-hour, 120-frames-per-second run stays within two ticks of the expected
fractional total. A second randomized run interleaves 10,000 stalls with
pause/reset events and verifies that reset discards only paused time and that
every advance remains within the catch-up budget. Pause/resume resets accumulated
time and the catch-up budget.

Limitations:
- `Preserve` can retain an unbounded `Duration` backlog if the caller repeatedly
  adds time without consuming ticks; use `max_elapsed`/a drain policy in that
  case.
- `interpolation` can exceed `1.0` when a preserved backlog exists and no ticks
  have been consumed.
- `Duration` has nanosecond resolution; exact rational rates below one
  nanosecond per step are rejected or rounded by the existing constructor.
- No GPU/presentation timing or wall-clock sampling is performed by `FixedStep`.

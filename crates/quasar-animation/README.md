# quasar-animation

Renderer-neutral animation timelines, posing, blending, event markers and
layered state machines. It owns no entities, ECS, scene graph, GPU state or
global clock.

```toml
quasar-animation = { path = "crates/quasar-animation" }
# Optional adapters
# quasar-animation = { ..., features = ["serde", "sprite"} }
```

## Core model

- `Value`: float, vec2/3/4, quaternion, bool, u32.
- `Keyframe` + `Track`: step, linear or smooth interpolation.
- `Marker`: named event time.
- `Clip`: duration, tracks and markers. Sample returns a `Pose` map keyed by
  caller-defined property names.
- `Player`: caller-advanced playback with speed, once/loop/ping-pong, negative
  speed, events and large-timestep accounting.
- `Pose`: property map; `blend` preserves disjoint properties.
- `StateMachine`: layered states, ordered transitions, bool/float/trigger
  conditions, crossfades, state-time conditions and per-layer weights.
- `Skeleton`/`Bone`/`Transform`: renderer-independent skeletal pose adapter.
- Optional `sprite` feature: convert a `sprite.frame` step track to
  `quasar-render-2d::AnimationClip`.
- Optional `serde` feature: serializable clips, values, markers, transitions,
  conditions and parameter values.

## Example

```rust
use std::sync::Arc;
use quasar_animation::{Clip, Interpolation, Keyframe, LoopMode, Player, Track, Value};

let clip = Arc::new(Clip::new(
    "pulse", 1.0,
    vec![Track::new("scale", Interpolation::Linear, vec![
        Keyframe::new(0.0, Value::Float(1.0)),
        Keyframe::new(1.0, Value::Float(2.0)),
    ])?],
    vec![],
)?);
let mut player = Player::new(clip, LoopMode::Loop);
player.advance(0.25)?;
assert_eq!(player.sample().f32("scale"), Some(1.25));
# Ok::<(), quasar_animation::AnimationError>(())
```

## Event semantics

For forward playback, markers in `(from, to]` are emitted. Backward playback
uses `[to, from)`. At a loop/ping-pong boundary, the destination boundary marker
is included. A huge looped advance skips enumerating every repeated cycle:
`Playback::wraps` reports how many boundaries were crossed and
`skipped_loops` reports cycles whose markers were not individually emitted.
Ping-pong caps segment expansion at 1024 and sets `truncated` if a larger delta
was supplied. This keeps memory/event output bounded; use bounded caller ticks
when every event matters.

## State machines

States are named clips per layer. Transitions are evaluated in insertion order
and have an optional source (`None` = any), target, crossfade duration and
condition. Trigger parameters are cleared when a transition using them fires.
A transition to the current state is ignored. Crossfade poses blend the outgoing
and incoming players; layered poses are normalized by layer weight.

```rust
# use std::sync::Arc;
# use quasar_animation::*;
# let idle = Arc::new(Clip::new("idle", 1.0, vec![], vec![Marker::new(0.0, "x")]).unwrap());
# let run = Arc::new(Clip::new("run", 1.0, vec![], vec![Marker::new(0.0, "x")]).unwrap());
let mut machine = StateMachine::new();
let layer = machine.add_layer("base");
machine.add_state(layer, "idle", idle, LoopMode::Loop, 1.0)?;
machine.add_state(layer, "run", run, LoopMode::Loop, 1.2)?;
machine.add_transition(layer, Transition::new(
    Some("idle".into()), "run", 0.15,
    Condition::Float { parameter: "speed".into(), compare: Compare::Greater, value: 0.1 },
)?)?;
machine.set_parameter("speed", ParameterValue::Float(1.0));
machine.update(0.01)?;
assert_eq!(machine.current_state(layer), Some("run"));
# Ok::<(), quasar_animation::AnimationError>(())
```

## Examples

```sh
cargo run -p quasar-animation --example sprite_sequence
cargo run -p quasar-animation --features sprite --example sprite_sequence
cargo run -p quasar-animation --example skeletal_blend
cargo run --release -p quasar-animation --example animation_benchmark
```

`physics-playground` also uses the state machine for AI agent idle/move/alert
transitions and a sampled pulse used by the renderer.

## Measured baseline

Development host, release, 200,000 samples of a clip with 128 properties × 128
keys: ~1.41 s (≈7.0 µs per full 128-track sample); `sample_into` with a reused
pose measured ~0.86 s (≈4.3 µs). 200,000 layered state-machine updates:
~10.2 ms. These are single-host probes, not portable guarantees.

## Verification

```sh
cargo test -p quasar-animation
cargo test -p quasar-animation --all-features
cargo clippy -p quasar-animation --all-targets --all-features -- -D warnings
```

## Limitations

- Playback uses `f32`; cross-platform bitwise equality is not claimed.
- No motion matching, IK, animation compression or authoring UI.
- Skeletal support is a CPU pose adapter; Quasar's 3D renderer does not yet
  expose GPU skinning.
- State machines own clips per state and are not ECS components.
- Sprite conversion handles one `sprite.frame` step track.

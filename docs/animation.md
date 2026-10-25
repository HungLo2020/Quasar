# Animation architecture and state machines

`quasar-animation` addresses Quasar issue **#35**. It is a standalone,
renderer-neutral library for time-based properties. Games own entities, update
cadence and mapping from property names to sprites, bones, transforms or UI.

## Design decisions

- **Generic values, not entity types.** A track has a caller-defined property
  string and a typed `Value`; no ECS or game class hierarchy is assumed.
- **Clips are immutable timelines.** `Clip` is validated once; `Player` is the
  mutable cursor. Multiple players can share an `Arc<Clip>`.
- **Explicit time.** `Player::advance(dt)` and `StateMachine::update(dt)` are the
  only clocks; there is no wall-clock or global animation manager.
- **Bounded events.** Loop markers are deterministic, but a single enormous
  delta does not allocate an unbounded event list. `Playback::skipped_loops`
  and `truncated` make omitted work visible.
- **Layer blending.** A `StateMachine` can contain multiple layers with
  independent states/transitions; poses are combined by normalized weights.
- **Thin adapters.** Sprite frame conversion is an optional feature; skeletal
  pose sampling is dependency-free. No renderer is required for either core.

## Data flow

```text
Clip { tracks, markers, duration }
  ├─ Track { Keyframe<Value>, Interpolation } ── sample(time) ── Pose
  └─ Player { time, speed, loop mode } ── advance(dt) ── Playback { events }
                       │
                       └─ Pose
StateMachine { layers, parameters }
  ├─ Layer { states, transitions, current Player, fade }
  └─ Pose blend + transition/event results
```

## Playback and looping

Loop modes are `Once`, `Loop` and `PingPong`. Speed is signed; a negative speed
plays backward. `seek` clamps/wraps according to mode. `Player::sample` uses the
clip's tracks and holds the last value after the final keyframe.

Event rules are documented in the crate README. For a large looped delta,
`wraps` counts boundaries and `skipped_loops` reports repeated cycles whose
markers were deliberately not enumerated. Ping-pong has a 1,024-segment cap so
a pathological delta cannot stall a frame indefinitely.

## Blending and transitions

`Pose::blend` interpolates matching property kinds and preserves properties
present on only one side. `StateMachine` uses a smoothstep crossfade between the
outgoing and incoming state clips, then normalizes layer weights for final
composition. Transitions are ordered and evaluated after advancing the current
state. Trigger parameters are automatically cleared after a transition uses one.
Conditions support booleans, numeric comparisons, triggers, state time, and
`All`/`Any` composition.

A transition to the current state is ignored to avoid restarting clips every
tick. If a transition occurs during an existing crossfade, the current state
machine snaps to the active state and starts the next transition (one fade at a
time, bounded memory).

## Sprite and skeletal adapters

- A step track named `sprite.frame` can be converted to
  `quasar-render-2d::AnimationClip` with the optional `sprite` feature. Frame
  durations must round to at least one millisecond.
- `Skeleton` is an ordered list of named bones with earlier-only parents.
  `Skeleton::sample(pose)` reads `bone.<name>.translation|rotation|scale`.
  Missing properties use the rest transform. `SkeletonPose::blend` lerps
  translation/scale and slerps rotations.
- GPU skinning/inverse bind matrices are renderer concerns. Quasar's current 3D
  renderer has no skeletal pipeline; the adapter is ready for a future one.

## Determinism and cost

- No RNG, global time, thread-local state or unordered public output.
- Sampling a 128-track/128-key clip for 200,000 frames took ~1.41 s on the
  development host (release) after interning property names and using a compact
  sorted pose; `sample_into` with a reused pose took ~0.86 s (~4.3 µs/sample).
  A 200,000-update layered state machine took ~10.2 ms. Property names are
  shared `Arc<str>` values; `Clip::sample` allocates one pose vector and
  `sample_into` can reuse it.
- `Pose::blend` allocates a new map. Games can cache/reuse poses at a higher
  layer if allocation becomes measurable.
- Large keyframe counts are bounded by `u32`/usize vector limits; validation
  rejects non-finite times/values and unsorted keys.

## Acceptance criteria for #35

- [x] Clips/tracks/interpolation, event markers, looping, speed/direction and
  deterministic caller-driven advancement.
- [x] Blending/crossfades, layered state machines, parameter bindings,
  explicit transitions and versioned serializable definition types.
- [x] Generic properties for transforms/numerics/events and sprite/skeletal
  adapters.
- [x] Headless tests for boundaries, repeated/large steps, transitions,
  serialization, properties and skeletal/sprite adapters.
- [x] 2D sprite-sequence and skeletal blend examples using the same core APIs.
- [x] No mandatory ECS, scene graph, GPU, renderer or editor.
- [x] Allocation/determinism limitations documented.

## Integration

- **Physics Playground:** AI agents use layered state machines for idle/move/
  alert animation; sampled pulse drives sphere scale.
- **2D games:** use `sprite.frame` plus the optional render-2d adapter, or map
  a pose to any sprite/atlas index.
- **3D games:** sample bone properties and send the resulting transforms to a
  renderer-owned skinning path.
- **UI/gameplay:** tracks can target arbitrary numeric/vector/bool properties.

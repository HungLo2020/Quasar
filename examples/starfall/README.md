# Starfall

A deliberately small but complete 3D Quasar reference game. It combines the
window/runner, input, deterministic simulation, job pool, assets, diagnostics,
audio, 3D renderer, 2D HUD and profiling in one ordinary Cargo binary.

```sh
cargo run -p starfall
cargo run -p starfall -- --headless --ticks 600 --record /tmp/starfall.json
cargo run -p starfall -- --headless --replay /tmp/starfall.json
cargo run -p starfall -- --headless --ticks 600 --screenshot /tmp/starfall.png
cargo run -p starfall -- --config examples/starfall/config.json --headless --ticks 10
cargo run -p starfall -- --benchmark --benchmark-frames 120 --benchmark-file /tmp/starfall.json
cargo run -p starfall -- --help
```

## Game

Pilot a survey craft through a neon arena. Collect all five crystals, shoot the
orbiting drones, and return through the portal before the timer expires. Hull
damage, restart and deterministic autoplay are included.

Controls are configurable in the default `config.toml` or with `--set`.
JSON is supported by passing a `.json` config path; files are never
rewritten or converted:

| Action | Default |
|---|---|
| Move | `WASD` (camera-relative, fixed right/left) |
| Turn | Mouse horizontal motion, or `Q` / `E` |
| Shoot | Mouse left button or `Space` |
| Restart | `R` |
| Fullscreen | `F11` |
| Quit | `Escape` |

Mouse sensitivity is configured with `game.mouse_sensitivity` (default
`0.0026` radians/pixel). The cursor is captured while the window is focused and
is released automatically on focus loss. Gamepad bindings are registered in the
same action map (left stick, right stick, South, Start); a caller-provided
gamepad source can feed them without changing the game.

## Why it is a reference

- **One device, one frame:** `RenderState3D` paints the world, then
  `RenderState2D` loads the same target for the HUD. Both command lists are
  submitted in one `FrameSurface` transaction.
- **Deterministic core:** fixed 60 Hz ticks, `GameWorld` implements
  `quasar_sim::Simulation`, and every input is recorded with a state hash.
- **Cargo-native:** `quasar-app` owns the window/event loop; the game owns GAL,
  render states, audio and state. `--headless` uses the same runner and rules.
- **Assets and jobs:** level JSON, PNG texture and procedural sound clips are
  loaded on a caller-owned `quasar-jobs` pool through `quasar-assets`.
- **Diagnostics and profiling:** explicit `Diagnostics` records plus a
  `quasar-profile` artifact for CPU/GPU frame time, scopes, uploads, resource
  churn and synchronization waits.
- **Configuration:** typed defaults/file/environment/CLI precedence through
  `quasar-config`; controls and gameplay timings are game-defined keys.

## Verification

```sh
cargo test -p starfall
cargo test -p starfall --no-default-features
cargo test -p starfall --test render -- --ignored   # Vulkan-capable host
cargo clippy -p starfall --all-targets -- -D warnings
```

Tests cover level validation, autoplay completion, snapshot restore, exact
record/replay, asset/texture/sound loading, typed config precedence, errors and
an offscreen 3D+HUD pixel/content check.

## Benchmark sample

Release build, RTX 2070 / Linux X11, 120 frames (one local run; not a portable
guarantee):

| Metric | p50 | p95 | p99 |
|---|---:|---:|---:|
| CPU frame | 14.38 ms | 15.90 ms | 16.81 ms |
| GPU frame | 65.5 µs | 68.1 µs | 75.8 µs |
| CPU submit | 91.7 µs | 131.6 µs | 155.5 µs |
| Acquire | 4.4 µs | 6.9 µs | 18.5 µs |
| Present/retire | 14.22 ms | 15.73 ms | 16.67 ms |
| Draws/frame | 145 | 145 | 145 |
| Host writes/frame | 18,368 B | 18,368 B | 18,368 B |

This Xephyr run was presentation-bound at roughly 60 Hz; CPU submission and GPU
work remained sub-millisecond. GPU scopes report roughly 40.96 µs for the 3D pass
and 8.58 µs for the HUD at p50 (45.66/10.27 µs at p99). GPU timestamps are
backend/hardware dependent; unsupported backends report zero samples rather
than estimated timings.

## Limitations

- Simple sphere/AABB-free distance collision; no physics engine.
- One player and one arena; replay is input journal based, not rollback
  networking.
- The gamepad action map is wired but winit does not produce gamepad events;
  an external source would be needed for hardware controllers.
- Audio is best-effort; unavailable devices log a warning and gameplay continues.
- The asset root is the crate checkout; packaging would replace it with a
  Basalt-mounted path.

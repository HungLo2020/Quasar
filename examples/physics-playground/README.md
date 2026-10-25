# Physics Playground

First-person 3D playground built on Rapier 3D, `quasar-render-3d`,
`quasar-render-2d` and VulkanicGAL.

```sh
cargo run -p physics-playground
cargo run -p physics-playground -- --headless --ticks 300 --autoplay
cargo run -p physics-playground -- --headless --ticks 120 --screenshot /tmp/playground.png
cargo run -p physics-playground -- --benchmark --benchmark-frames 120 --benchmark-file /tmp/playground.json
```

## Controls

| Input | Action |
|---|---|
| `W/A/S/D` | Walk |
| Mouse | Look |
| `Space` | Jump |
| Left mouse | Kick the looked-at dynamic object |
| `F` | Spawn a bouncing ball |
| `N` | Toggle navigation debug paths |
| `R` | Reset |
| `F11` | Fullscreen |
| `Escape` | Quit |

## What it demonstrates

- Rapier 3D gravity, dynamic blocks/balls, restitution, friction, damping and
  sleeping.
- A kinematic capsule player driven by Rapier's `KinematicCharacterController`,
  including slope/step handling, jumping and impulse transfer to blocks.
- Collision groups for world/player/prop/trigger layers, a sensor trigger zone
  and collision-event counting.
- Raycasts for crosshair interaction and impulse kicks; the ray excludes the
  player's own collider/body.
- `quasar-spatial`: moving prop AABBs and the player are indexed each tick;
  frustum queries cull 3D draws and radius queries feed the HUD.
- `quasar-animation`: two AI agents have idle/move/alert layered state machines;
  a sampled pulse drives their scale.
- `quasar-nav`: a dynamic 32×32 grid is rebuilt from static and moving obstacle
  AABBs only when a prop pose changes (throttled to at most every five ticks);
  agents replan on version changes, follow smoothed paths and can display debug
  path cells with `N`.
- `quasar-app` fixed-step lifecycle, `quasar-input` mouse capture/look,
  diagnostics, `quasar-profile` frame metrics and shared 3D+2D rendering through
  one GAL/frame.
- Headless deterministic ticks with `--headless --autoplay`, offscreen
  screenshots and benchmark JSON.

The agents are deterministic visual/physics actors. They are intentionally
simple and do not implement combat, perception or steering beyond path
following.

## Measured on the development host

Release, Xephyr at 1280x720, 120 benchmark frames including two animated
agents and navigation updates. The X server was presentation-bound:

| Metric | p50 | p99 |
|---|---:|---:|
| CPU frame | 17.3 ms | 19.2 ms |
| GPU frame | 54.4 µs | 71.7 µs |
| 3D scope | 28.7 µs | 43.0 µs |
| HUD scope | 7.6 µs | 8.4 µs |
| Draws | 244 | 246 |
| Host writes | 23.2 KB | 23.3 KB |

Headless simulation was ~0.88 ms per tick with autoplay on this scene after
throttling navigation rebuilds. Single-host observations only.

## Design/limits

- The player is kinematic; enemies/ragdolls/vehicles would use Rapier bodies
  directly.
- Props are capped at 64. There is no editor, serialization format or ECS.
- The level is a bounded simple arena. Frustum culling is conservative.
- Audio is intentionally omitted; this demo focuses on physics/render/spatial
  integration.

## Verification

```sh
cargo test -p physics-playground
cargo test -p physics-playground --test playground -- --ignored
cargo clippy -p physics-playground --all-targets -- -D warnings
```

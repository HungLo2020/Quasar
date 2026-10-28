# Nightwatch

A playable, procedural first-person zombie survival example for Quasar's opt-in
3D lighting/material path. All gameplay is ordinary game-owned Rust data; there
is no ECS, scene graph, object registry or required authored scene format.

```sh
cargo run -p nightwatch
```

Survive waves in an industrial courtyard. Zombies navigate around crates and
barricades, chase the player and attack at close range. Aim for the head. Two
weapons have distinct firing cadence, magazine capacities and reload times.
The viewmodel animates recoil, slide movement, walking bob and reloads; zombies
have walking limbs, hit feedback and death poses. Muzzle flashes briefly light
the world and weapon. Procedural sounds are optional, and missing audio hardware
leaves the game playable.

| Input | Action |
|---|---|
| WASD / mouse | Move / look |
| Left mouse | Fire; hold for repeated shots |
| 1 / 2 | Pistol / rifle |
| R | Reload; ammunition reserves are unlimited |
| Shift / Space | Sprint / jump |
| F | Toggle flashlight |
| Escape | Pause/resume; quit after death |
| Click while paused / Q while paused | Resume / quit |
| Enter after death | Restart |
| F11 | Toggle fullscreen |

Focus loss pauses interactive play and releases the cursor. Resume deliberately
with Escape or a click. Mouse look is updated per frame; normal simulation uses
fixed 60 Hz ticks. Scripted autoplay continues without focus. In autoplay
benchmark mode, each application frame advances one scripted 1/60-second step,
so render measurements exercise a reproducible animated workload regardless of
host frame rate.

## Independent components

The rendering adapter owns one VulkanicGAL instance, a lit world renderer, a lit
first-person weapon renderer and a 2D HUD renderer. They share meshes, a texture,
targets and one frame submission. The example records direct GAL attachment
barriers between offscreen passes; native attachments use GAL-managed frame
transitions. The weapon clears depth after the world through an
explicit `LitPassOptions` dependency, so it stays visible near cover without
creating another device or copying a framebuffer.

Directional/colored point/spot lights, an unlit/emissive path, per-pixel
Lambert/Blinn–Phong shading and explicit linear/sRGB handling use public
`quasar-render-3d` APIs. `quasar-nav` supplies bounded A* queries; the game owns
agents and steering. Simple fixed-step axis movement against static boxes
belongs to this sample, rather than imposing a physics dependency on rendering.
Collision is a compact FPS sample model, not a general physics solver.

The library's `World` accepts explicit `Input` values and can be driven without
a window, GPU, audio device or `quasar-app`. The desktop adapter opts into the
runner. A future scene/ECS integration can generate the same draw/light values
without replacing this game-owned representation. See
[lighting architecture](../../docs/lighting-3d.md) for shader extension and
resource contracts.

## CLI and headless operation

```sh
# Simulation only: no graphics, window, audio or runner dependencies.
cargo run -p nightwatch --no-default-features -- --headless --autoplay --ticks 1200 --seed 42

# GPU screenshot without a window; optional graphics feature adds GAL/renderers.
cargo run -p nightwatch --no-default-features --features graphics -- \
  --headless --ticks 1 --autoplay --resolution 1280x720 --screenshot /tmp/nightwatch.png

# Finite native benchmark, with 8 warmup frames and bounded profile retention.
cargo run -p nightwatch -- --autoplay --no-audio --frames 180 \
  --resolution 960x540 --benchmark-file /tmp/nightwatch-profile.json

cargo test -p nightwatch --no-default-features
cargo test -p nightwatch --test render -- --ignored --test-threads=1
```

Additional options: `--fullscreen`, `--audio-device NAME`, `--help`, `--version`.
`--benchmark` disables requested vsync, enables GPU timestamps, and defaults to a
240-frame run; `--benchmark-file` also enables it. `--frames` counts native
rendered frames. GPU benchmarks require native mode. Configuration files and
`--set` are rejected because this example has no persisted configuration schema.
All assets and effect PCM are generated locally; no asset download is needed.

Simulation hashes support repeatability checks on the same floating-point
platform. They do not claim bitwise cross-platform replay compatibility.

## Scope and limitations

This is a low-poly lighting/gameplay example. Shadows, PBR/normal maps, baked
lighting, HDR tonemapping and general animation skinning remain future renderer
milestones. Humanoids and weapons use game-owned rigid-part animation. The
built-in forward path shades every supplied light; this example selects a small
bounded set rather than benchmarking thousands of lights. World and weapon
passes are opaque and explicitly encode sRGB; transparent/HDR consumers should
compose in linear space and encode separately.

No editor/runtime scene work, external game migration, GitHub workflow or
release operation is required to build or run the example.

## Verification and measured baseline

Local Linux x86-64 verification uses Vulkan on an RTX 2070. Automated X11 input
checks exercise movement, mouse fire, weapon switching/reload, jump, flashlight,
pause and window resize, with balanced GPU resource counts at shutdown. The
180-frame autoplay benchmark advances 180 gameplay ticks, including firing,
weapon animation and enemy movement (16 shots and 4 kills for seed 42).

A debug build with GAL validation, Xvfb, 960×540 and 8 warmup frames produced:

| Metric | p50 | p99 |
|---|---:|---:|
| CPU frame | 25.97 ms | 31.53 ms |
| CPU submission | 12.15 ms | 14.83 ms |
| GPU frame | 0.328 ms | 0.384 ms |
| Draw calls | 268 | 276 |
| Host writes | 82,752 B | 84,800 B |
| Blocking/device-idle waits | 0 / 0 | 0 / 0 |

These are one-machine debug/validation observations, not optimized performance
claims or cross-device thresholds. Late weapon/slot capacity growth can create
bounded resources after warmup (maximum two creates in a sampled frame).

The library's lighting pixel/composition/slot tests also pass through the
wgpu-backed GAL on Linux. No Windows/macOS parity is claimed. OpenGL-only builds
compile, but the current OpenGL shader conventions fail the renderer's explicit
binding/[0,1] depth requirements; construction returns `UnsupportedFeature`.
A separate capability-rejection test covers that boundary. No separate renderer
or silent backend fallback is introduced to hide it.

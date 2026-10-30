# Nightwatch

A playable first-person zombie survival example for Quasar's opt-in
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
| H / N | Toggle shadows / normal maps (status shown on HUD) |
| Escape | Pause/resume; quit after death |
| Click while paused / Q while paused | Resume / quit |
| Enter after death | Restart |
| F11 | Toggle fullscreen |
| F5 | Reload model/image assets; keep the previous generation if loading fails |

Focus loss pauses interactive play and releases the cursor. Resume deliberately
with Escape or a click. Mouse look is updated per frame; normal simulation uses
fixed 60 Hz ticks. Scripted autoplay continues without focus. In autoplay
benchmark mode, each application frame advances one scripted 1/60-second step,
so render measurements exercise a reproducible animated workload regardless of
host frame rate.

## Independent components

The rendering adapter owns one VulkanicGAL instance, a surface world renderer, a surface
first-person weapon renderer, an explicit shadow-map renderer and a 2D HUD renderer. They share meshes, a texture,
targets and one frame submission. The example records direct GAL attachment
barriers between offscreen passes; native attachments use GAL-managed frame
transitions. The weapon clears depth after the world through an
explicit `LitPassOptions` dependency, so it stays visible near cover without
creating another device or copying a framebuffer.

Directional/colored point/spot lights, an unlit/emissive path, per-pixel
Lambert/Blinn–Phong shading, directional sun shadows, perspective flashlight
shadows, procedural concrete/metal normal maps and explicit linear/sRGB handling use public
`quasar-render-3d` APIs. `quasar-nav` supplies bounded A* queries; the game owns
agents and steering. Collision-only `quasar-physics`/Rapier supplies swept
character capsules, round barrel cylinders, cuboids and compound props. Rendering,
bullets and navigation consume the same named prop definitions. Physics remains
independent of the render crates. See [3D collision](../../docs/collision-3d.md)
for ownership, coordinates, shape selection and reuse.

The library's `World` accepts explicit `Input` values and can be driven without
a window, GPU, audio device or `quasar-app`. The desktop adapter opts into the
runner. A future scene/ECS integration can generate the same draw/light values
without replacing this game-owned representation. See
[lighting architecture](../../docs/lighting-3d.md) for shader extension and
resource contracts. The [surface/shadow contract](../../docs/shadows-3d.md)
explains the independent caster/receiver API and map lifetime rules.

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

Additional options: `--fullscreen`, `--audio-device NAME`, `--no-shadows`,
`--no-normal-maps`, `--shadow-resolution N`, `--help`, `--version`.
Shadow resolution defaults to 1024 and accepts 32..=2048. Two maps across at most
three in-flight slots use at most 24 MiB at 1024 or 96 MiB at 2048. Slots allocate
lazily. Disabling shadows omits caster passes; normal maps remain independently
usable. H/N toggle effects live without changing gameplay state.
`--benchmark` disables requested vsync, enables GPU timestamps, and defaults to a
240-frame run; `--benchmark-file` also enables it. `--frames` counts native
rendered frames. GPU benchmarks require native mode. Configuration files and repeatable `--set` overrides use quasar-config
precedence; see [submission/configuration](../../docs/render-submission.md) for
the TOML/JSON schema, environment mappings and live benchmark thresholds.
Selected model/image assets are bundled under `assets`; floor, lights, hands,
enemies and effect PCM remain generated locally. No runtime download is needed.

Simulation hashes support repeatability checks on the same floating-point
platform. They do not claim bitwise cross-platform replay compatibility.

## Scope and limitations

This is a low-poly lighting/gameplay example. Directional shadows use one fixed
orthographic volume; flashlight shadows use one perspective map. Both use 3×3
PCF and explicit depth/normal bias. Arena cover and animated zombie parts cast
shadows; the first-person weapon receives local lighting/normal maps without
world shadows, avoiding a world/viewmodel coordinate mismatch. Normal textures
are generated from procedural concrete and metal height fields.

Cascaded/point-light shadows, PBR, baked lighting, HDR tonemapping and general
animation skinning remain future renderer milestones. Humanoids and weapons use game-owned rigid-part animation. The
built-in forward path shades every supplied light; this example selects a small
bounded set rather than benchmarking thousands of lights. World and weapon
passes are opaque and explicitly encode sRGB; transparent/HDR consumers should
compose in linear space and encode separately.

No editor/runtime scene work, external game migration, GitHub workflow or
release operation is required to build or run the example.

## Verification and measured baseline

The table below records the earlier lighting-only build before shadow mapping;
it is retained for comparison and does not describe the current default effects.

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

Before instancing/culling, with shadows and normal maps enabled, the same debug/validation/Xvfb setup
produces the following 180-frame results (172 retained samples):

| Metric | p50 | p99 |
|---|---:|---:|
| CPU frame | 66.66 ms | 74.23 ms |
| CPU submission | 49.91 ms | 55.83 ms |
| GPU frame | 0.888 ms | 0.999 ms |
| Draw calls | 750 | 758 |
| Host writes | 208,208 B | 210,256 B |
| Blocking/device-idle waits | 0 / 0 | 0 / 0 |

The two caster passes increased draw/validation work substantially. This earlier
debug build was CPU-bound in submission; these numbers establish functionality and cost,
not a real-time performance target. The optional submission path described below addresses that CPU work. Maximum resource creation after
warmup was three objects in a frame, with no leaks at teardown. Native tests also
pause the game, toggle H/N, verify world pixels change, then verify restoring both
effects restores the image exactly. The surface/shadow GPU tests pass through
both Vulkan and wgpu on this machine.

## Instanced submission and visibility

Nightwatch now enables instancing and independent camera/light-frustum culling.
Use `--no-instancing`, `--no-culling`, `--validation` and `--no-validation` for
controlled comparisons. `render.*` configuration controls the same policy.
Material/model inputs stay ordinary game data. Static cover, animated enemies,
weapon parts and both shadow passes share the library's bounded upload slots.

Vulkan and wgpu tests compare scalar, instanced and culled images exactly over
six animated gameplay frames, with balanced teardown. Additional GPU tests cover
off-camera shadow casters, per-instance materials, buffer growth, retained
commands and reversed submission order. Native tests exercise gameplay, H/N,
resize, teardown and a configured live threshold failure.

An optimized Linux/RTX 2070/Xvfb run at 960×540, seed 42, 180 frames with 8 warmup
frames, repeated three times per case, produced these medians of each run's p50:

| Policy | CPU frame | CPU submit | GPU frame | Draws | Host writes |
|---|---:|---:|---:|---:|---:|
| Previous commit, validation on | 17.52 ms | 4.838 ms | 0.886 ms | 750 | 208,208 B |
| Instancing + culling, validation on | 10.14 ms | 0.191 ms | 0.920 ms | 28 | 112,160 B |
| Previous commit, validation off | 11.22 ms | 0.313 ms | 0.839 ms | 750 | 208,208 B |
| Instancing + culling, validation off | 10.23 ms | 0.133 ms | 0.920 ms | 28 | 112,160 B |
| Current scalar path before model imports, validation off | 11.51 ms | 0.344 ms | 0.883 ms | 750 | 208,208 B |
| Instancing without culling, validation off | 10.62 ms | 0.143 ms | 0.857 ms | 32 | 143,376 B |

Validation and Vulkan command-label environment settings match between old/new
cases. The old executable was built from commit `1e6c0ed`. All runs produced
16 shots, 4 kills and game hash `c6ee52a2d7295b44`; resources balanced at shutdown.
The optimized path measured 557 visible and 158 culled pass instances at p50
(these are independent distributions). CPU preparation was about 0.27 ms.

CPU submission and upload costs improve substantially; this does not demonstrate
a GPU-time improvement. Whole-frame results are dominated by roughly 10 ms of
Xvfb presentation time, with run-to-run variation. Compare matching optimized
builds and validation settings; the earlier 67 ms debug number is not an
optimized baseline. See the reusable
[benchmark runner and configuration](../../docs/render-submission.md) for
reproducing cases and retaining raw measurements.

## Authored model assets

The scene now uses Kenney CC0 low-poly blasters as pistol/rifle, modular crates,
barrels, a workbench and wall panels. The source files, licenses, archive hashes
and per-file hashes are recorded in [assets/README.md](assets/README.md) and
[assets/manifest.json](assets/manifest.json). They are stylized guns with game
material tints, rather than replicas of real weapons.

The glTF loader and GPU adapter are public Quasar APIs. Nightwatch owns only its
asset selection, cover placement and animation policy. Recoil moves the loaded
weapon; named magazine parts move during reload. Crate clips are explicitly
imported at their default pose. `props.rs` owns named model assignments, placements
and explicit colliders: individual barrel cylinders, crate/wall cuboids, a
compound workbench and character capsules. Navigation validates both nodes and
edges against those same shapes.

`--asset-root PATH`, `assets.root` in TOML/JSON, and `NIGHTWATCH_ASSET_ROOT` select
an editable copy with the same pack layout. F5 stages replacements for the whole
selection; a broken/missing file leaves the previous models usable and reports
the error through diagnostics. Texture files participate in dependency tracking.
Run without graphics to retain the independent simulation-only dependency set.

The earlier 28-draw benchmark above predates these authored models. The new scene
has multiple meshes and image bindings; its draw count is a different workload.
See [3D asset loading](../../docs/model-assets-3d.md) for the implementation,
supported subset and reload/retirement tests.

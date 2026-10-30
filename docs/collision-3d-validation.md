# Nightwatch collision validation

Validated on Linux x86_64, Intel Core i7-10750H, NVIDIA RTX 2070,
NVIDIA 580.178.04. The baseline is workspace v0.2.18 commit
`5cc7517daa1dee8d18cc0cc8474099fc8cd12075`. The measurements capture the new collision implementation before the 0.2.19
release version bump. Validation used local commands; the collision audit did
not dispatch GitHub workflows or update issues.

## Findings and implementation

The old game used seven obstacle rectangles for movement, bullet boxes and
inflated grid occupancy. Rendering separately chose models by vector index
and obstacle aspect ratio. Each of two rectangles displayed **two** much
smaller barrels. This caused large invisible group corners, box-like bullet
hits and overly conservative navigation. Axis-by-axis endpoint movement could
also tunnel through geometry. Most visible walls, posts, lamps and gate pieces
had no individual collider; a global clamp approximated the perimeter.

The replacement uses explicit game-owned props and collision-only Rapier:
individual cylinders, cuboids, a compound workbench and shared character
capsules. Source bounds, structural fitting anchors, rigid transforms and
collider definitions are explicit, named and tested. Static bullets and swept
navigation use the same colliders as movement. Emissive lamp lights also read
the prop placements. There is no new collision solver, mandatory ECS, asset
mesh-collider generation or renderer physics dependency. See
[architecture and limitations](collision-3d.md).

The workspace audit additionally exposed Physics Playground's sensor checking
only the player center. Under Rapier's optional enhanced floating-point policy,
the player could stop with the capsule inside the trigger and its center
outside. The sample now queries its actual capsule, with a regression assertion
that surface entry activates the trigger before center entry. Nightwatch
itself does not force `enhanced-determinism` on other workspace users.

## Automated coverage

- Seven new physics fixtures: invalid/empty/deep/oversized shapes, nested
  compound flattening, rigid rotated bounds, invalid round scaling, capsule
  head corners, high-speed anti-tunneling, curved sliding, floor landing,
  ceiling sweeps and 513 analytic cylinder silhouette ray samples.
- Two CPU model-data fixtures: transformed instances, structural selection,
  containing radius, malformed geometry and non-affine transforms.
- Two navigation fixtures: circular clearance, real-position attachment,
  bounded allocation, thin obstacles between free nodes, swept valid edges,
  deterministic A* detours.
- Eleven Nightwatch integration fixtures with graphics enabled (ten headless):
  default barrel corners/grazes, compound top/leg gaps, transactional prop
  replacement, named model assignment after reordering, wall-obstructed melee,
  jumping/landing on props, perimeter/rotated geometry, NPC cylinder detours
  without penetration, bullets through former box corners, 1,800-tick replay
  equality and shipped-model fit/collider envelope calibration.
- Existing game/config tests remain passing. Asset alignment tests load the
  bundled GLBs through the existing asset service, without requiring a GPU.
- The final default workspace run passed **625 tests**, including doctests;
  54 existing hardware/ignored tests were not selected by that command.
  Separately, all three Nightwatch GPU tests passed on **native Vulkan** and
  again on **wgpu**. They check transactional asset reloads, scalar/instanced
  pixel parity, independent shadow culling, world/weapon/HUD composition,
  resize and GPU resource balance.

The feature matrix passed: workspace all-target/all-feature checks; physics
with no features, collision-only dim2, collision-only dim3, model-assets, and
all features (including dynamics/events/spatial/property tests); navigation
with no defaults, physics3d and all features; Nightwatch headless with ordinary
and enhanced determinism; Physics Playground headless and its enhanced sensor
regression. All-target Clippy with warnings denied passed for affected crates
in all-feature and headless configurations. Rustdoc with warnings denied,
formatting, Python check-runner syntax and whitespace checks passed.
Dependency-tree checks confirm ordinary navigation remains Rapier-free,
model fitting pulls in no GPU/decoder/window, and headless Nightwatch pulls in
no renderer, GAL, window, audio or glTF decoder.

## Native game exercise

A real Nightwatch X11 window under Xvfb used the NVIDIA Vulkan backend. Input
injection exercised walking, mouse firing, rifle selection, reload, jump,
flashlight, pause/exit, shadow/normal toggles and resize. Paused F5 success,
corrupted-model failure with the old frame retained, and restored-model
recovery were checked against captured pixels. Effect toggles changed the
frame and toggling back restored it. GPU resource counters balanced at
619 created / 619 destroyed. A deliberately failing benchmark threshold wrote
its result and released resources before returning failure.

This validates the native presentation/input integration, not subjective
mouse feel or every possible traversal. The long capsule-navigation and barrel
corner checks are deterministic simulation fixtures rather than visual guesses.

## CPU performance

The built-in probe uses 6,000 fixed ticks, a moving/jumping player and either
six or 24 living zombies. Health is reset outside the timer so the workload
does not stop on death; there is no firing or GPU work. Three runs are recorded
for each case. Values below are the median of each run's p50 or p95, in µs.
Raw runs and method are retained in
[measurements/nightwatch-collision-cpu.json](measurements/nightwatch-collision-cpu.json).

| Living zombies | Old boxes p50 | New shapes p50 | Old boxes p95 | New shapes p95 |
|---|---:|---:|---:|---:|
| 6 | 0.476 | 153.266 | 38.239 | 304.602 |
| 24 | 13.009 | 498.161 | 142.634 | 828.923 |

This is a substantial CPU cost increase for accuracy. The old scene has seven
boxes and a 32×32 point-blocked grid; the new scene has 63 static shaped
colliders and a 60×60 graph with validated capsule edges. Inputs match, while
resulting trajectories and collision work necessarily differ. At 24 agents,
the median tick uses about 3% of a 16.67 ms/60 Hz budget, excluding rendering.
The worst observed new tick in this series was 2.45 ms; timings are not latency
guarantees. No elapsed-time assertions make correctness tests flaky.

Median startup samples rose from roughly 0.005 ms to 32–43 ms: collider/index
construction and graph baking happen once per level preparation/replacement,
not each tick. 20,000 new world raycasts averaged 0.34–0.44 µs per query across
runs. The old implementation has no directly equivalent public scene-ray API;
no ray speedup is claimed. Repeated hashes match within each implementation
and agent count, while old/new hashes differ intentionally.

```sh
cargo run --locked --release -p nightwatch --no-default-features --example collision_benchmark -- 6
cargo run --locked --release -p nightwatch --no-default-features --example collision_benchmark -- 24
```

Future optimization targets include reducing repeated capsule clearance and
controller queries and sharing immutable level preparations across restarts.
The measured accuracy change does not justify introducing a second collision
engine or caching unvalidated geometry. Dense rebaking is intended for static
levels, not continuous large-world editing.

## Reproduction

```sh
cargo check --locked --workspace --all-targets --all-features
cargo test --locked --workspace
cargo test --locked -p quasar-physics --no-default-features --features dim3
cargo test --locked -p quasar-physics --no-default-features --features model-assets
cargo test --locked -p quasar-physics --all-features
cargo test --locked -p quasar-nav --no-default-features --features physics3d
cargo test --locked -p quasar-nav --all-features
cargo test --locked -p nightwatch --no-default-features
cargo test --locked -p nightwatch --no-default-features --features quasar-physics/enhanced-determinism
cargo test --locked -p physics-playground --features quasar-physics/enhanced-determinism
cargo clippy --locked -p quasar-physics -p quasar-nav -p nightwatch -p physics-playground --all-targets --all-features -- -D warnings
cargo clippy --locked -p quasar-physics -p quasar-nav -p nightwatch --all-targets --no-default-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked -p quasar-physics -p quasar-nav -p nightwatch --all-features --no-deps
cargo fmt --all -- --check
xvfb-run -a cargo test --locked -p nightwatch --test render -- --ignored --test-threads=1
xvfb-run -a env WGPU_BACKEND=vulkan cargo test --locked -p nightwatch --features vulkanic-gal/vulkan --test render -- --ignored --test-threads=1
cargo run --locked -p nightwatch --no-default-features -- --headless --ticks 600 --autoplay
```

`tools/full_checks.py` now retains the new feature/isolation checks and both
Nightwatch GPU runs. Optional tests require their platform drivers/libraries;
headless collision tests do not. This audit ran local commands, with no GitHub
workflow dispatch.

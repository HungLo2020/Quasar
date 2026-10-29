# Quasar

Composable Rust game infrastructure, with Pong as its first consumer. Games own
ordinary Cargo projects, their simulation, and their application loop. Each
Quasar crate can be used independently.

```sh
cargo run -p pong
cargo run -p pong -- --two-player
```

Pong starts against the computer. **W/S** move the left paddle; **Up/Down** move
the right paddle in two-player mode. **Space** serves, **P/Escape** pauses,
**R** restarts, and **Q** quits. First to seven wins. **F11/Alt+Enter** toggles
fullscreen; **F2** changes filtering; **F3** changes fit/integer scaling.
Losing focus releases held input and pauses an active human-controlled match.

## What is implemented

| Crate | Responsibility | Depends on other Quasar crates |
|---|---|---|
| `quasar-input` | Portable keyboard/mouse/gamepad state, player routing, hotplug, deadzones/calibration, named actions, axes, chords, serializable bindings | None |
| `quasar-window` | Native windows, desktop identity, fullscreen, DPI, cursor capture, native input adapter | quasar-input |
| `quasar-core` | Fixed-step timing and shared viewport/scaling math | None |
| `quasar-config` | Typed TOML/JSON settings, env/CLI precedence, validation, migrations and common arguments | Optional quasar-storage |
| `quasar-storage` | Basalt v1 paths and atomic JSON persistence | None |
| `quasar-assets` | Validated virtual paths, mounts, typed loaders, dependency graphs, schema checks, bounded cache, async worker and hot reload | Optional quasar-storage (Basalt mounts), diagnostics and notify |
| `quasar-diagnostics` | Explicit structured logs, spans, counters, bounded failure diagnostics and optional CPU frame timing | None |
| `quasar-app` | Optional Cargo-native winit/headless lifecycle runner with explicit frame/fixed-tick/input/render phases and suspend/resume hooks | quasar-core, quasar-input, optional quasar-window |
| `quasar-jobs` | Optional bounded worker pool with cancellation, dependencies, priorities, manual deterministic mode and metrics | None |
| `quasar-profile` | Bounded CPU/GPU frame, upload, resource, synchronization and renderer profiling with p99 summaries | Optional quasar-frame |
| `quasar-spatial` | Dependency-free 2D/3D AABB/ray/plane/frustum geometry and a deterministic spatial-hash index | None |
| `quasar-physics` | Optional Rapier 0.36 integration: collision-only worlds, bounded fixed-step dynamics, filters, events and optional spatial indexes | Optional quasar-spatial |
| `quasar-animation` | Renderer-neutral clips/tracks/interpolation, playback, blending, events and layered state machines; optional sprite/serde adapters | Optional quasar-render-2d |
| `quasar-nav` | Generic A*/Dijkstra, 2D/3D grids, dynamic costs/blocking, smoothing, optional tilemap/spatial/jobs/navmesh adapters | Optional quasar-tilemap, spatial, jobs, navmesh |
| `quasar-text` | Caller-owned TTF/OTF fonts, Harfrust shaping via cosmic-text, bidi/wrapping/alignment, caret/selection, glyph rasterization and cache-friendly CPU atlases | None |
| `quasar-ui` | Retained in-game menus/HUD: widget tree, stack/grid/absolute layout, DPI/safe areas, focus/modal scopes, text editing, rebinding, themes and consumed-input reporting; optional input/render2d/config/text adapters | Optional quasar-input, quasar-render-2d, quasar-config, quasar-text |
| `quasar-sim` | Optional fixed-step coordination, deterministic journals, snapshots, replay verification and divergence reports | quasar-core, optional quasar-jobs |
| `quasar-audio` | WAV/Ogg decode, deterministic mixing, buses, spatial audio, scheduling, streaming Ogg, offline WAV and CPAL output | Optional quasar-storage (settings) |
| `vulkanic-gal` | Independent extraction of MattMC's graphics abstraction layer | None |
| `quasar-render-2d` | Ordered instanced quads, sprites/atlases, bitmap text, optional shaped glyph-atlas pages, camera/transforms, clipping, GPU readback, persistent software-framebuffer presentation | VulkanicGAL, quasar-core, quasar-frame; optional quasar-text |
| `quasar-tilemap` | Versioned/Tiled maps, indexed tile/object queries, PNG atlases, animation, culled draw lists and CLI | Optional quasar-render-2d |
| `quasar-render-3d` | Indexed meshes, transforms, legacy and opt-in per-pixel directional/point/spot lighting, classic/unlit and normal-mapped materials, directional/spot shadows, depth and shared-GAL prepared passes | VulkanicGAL |
| `quasar-frame` | Optional recording transactions, target caching, acquire/present/cancel and completion polling | VulkanicGAL |
| `examples/tilequest` | Scrolling top-down quest, layered maps, collision, replays and headless/native verification | Selected crates |
| `examples/framebuffer` | CPU-rendered RGBA8 framebuffer presented through `quasar-render-2d`, with headless screenshots, window exercises and benchmarks | quasar-render-2d |
| `examples/audio` | Music, overlapping/spatial effects, buses, voice groups, procedural audio, offline WAV output and audio benchmarks | quasar-audio |
| `examples/pong` | Game rules, 2D draw lists, desktop application, headless CLI and replay | Selected crates |
| `examples/starfall` | Playable 3D reference game: shared 3D+2D frame, jobs/assets/audio, deterministic replay, config, in-game menus and profiling | quasar-app, input, sim, jobs, assets, diagnostics, audio, render-2d/3d, config, ui, profile |
| `examples/falling-sand` | Interactive cellular sand with Rapier 2D obstacles, dynamic ball, spatial queries and retro menus | quasar-app, input, physics, spatial, render-2d, ui, profile |
| `examples/physics-playground` | First-person Rapier 3D playground with character control, blocks, balls, triggers, raycasts, animated AI agents and navigation | quasar-app, input, physics, spatial, animation, nav, render-2d/3d, profile |
| `examples/nightwatch` | First-person zombie survival: directional/spotlight shadows, normal maps, per-pixel 3D lights/materials, animated weapons/enemies, obstacle navigation, shared GAL world/weapon/HUD passes and independent headless simulation | Selected rendering/input/nav/audio crates; optional app runner |
| `examples/app-stress` | Internal integration harness: frame/fixed phases, explicit input consumption, jobs/async assets, paused ticks, resize/fullscreen and deterministic headless hashes | quasar-app, input, jobs, render-2d, window |

Controller support lives in `quasar-input`: pure gamepad types, hotplug/player
routing, deadzones, calibration and combined keyboard/gamepad actions, plus an
optional `gilrs` adapter. See
[the input guide](crates/quasar-input/README.md). Generic asset paths, mounts,
typed loaders, dependency graphs and bounded caches live in
[`quasar-assets`](crates/quasar-assets/README.md); neither crate requires a
runner, editor or build tool. Optional observability uses the explicit,
feature-gated [`quasar-diagnostics`](docs/diagnostics.md) layer; async loading and
hot reload are documented in [async assets](docs/async-assets.md), timing policy
in [timing](docs/timing.md), the optional runner in [app runner](docs/app-runner.md),
its [v0.2.16 migration guide](docs/app-migration-v0.2.11.md) and
[adoption analysis](docs/app-adoption.md), background work in [jobs](docs/jobs.md), deterministic replay in
[simulation](docs/simulation.md), configuration in [configuration](docs/configuration.md),
graphics measurements in [profiling](docs/profiling.md), spatial indexing in
[spatial](docs/spatial.md), Rapier integration in [physics](docs/physics.md),
animation in [animation](docs/animation.md), navigation in [nav](docs/nav.md),
in-game menus/HUDs in [UI](docs/ui.md), fonts and shaped text in [text](docs/text.md),
and edge/focus behavior in [input hardening](docs/input-hardening.md). New
human-editable configuration defaults to TOML with JSON compatibility; the
[Starfall](examples/starfall/README.md) reference game demonstrates the combined
stack, while [Falling Sand](examples/falling-sand/README.md) and the
[Physics Playground](examples/physics-playground/README.md) demonstrate spatial
indexing, Rapier 2D/3D and headless/native verification.

### Desktop window identity

`quasar-window` owns native identity and compositor hints so games do not need
private Xlib/XCB code. `GameWindow::create` keeps the existing `WindowConfig`
contract; `create_with_desktop` adds distinct X11 instance/class names and an
explicit opaque or transparent policy. X11 metadata is applied before graphics
initialization, and `GameWindow::process` keeps the physical-size opaque region
in sync with resize, DPI and fullscreen changes. Wayland uses the winit
application id; Windows and macOS do not acquire X11 dependencies. See
[desktop window identity](docs/window-identity.md) for the MissileCommand,
Jackal, BlackOps and MicroMages audit and migration matrix.

The desktop example submits a 640×400 logical canvas to `quasar-render-2d`,
which draws GPU quads and font atlas sprites through VulkanicGAL. It uses
native X11/Wayland on Linux; window/surface integration also has Win32 and macOS
paths through winit/ash-window. Only Linux has been verified so far. Native
Vulkan requires a Vulkan 1.3 driver with dynamic rendering, synchronization2,
and timeline semaphores. macOS additionally needs a compatible Vulkan
portability driver; distribution of that runtime is future work. x86-64 is the
verified architecture; arm64 and riscv64 remain unverified targets.

There is no editor yet. The [requirements](REQUIREMENTS.md) call for an optional
native Rust editor built with the shared `quasar-ui`, Quasar window/input layers
and Quasar renderers on one caller-owned VulkanicGAL device/frame. Games and
tools use the same public widgets; reusable controls belong in `quasar-ui`.
The editor and tilemap tools will expose essential operations through public
Rust APIs and CLI/headless workflows as well as their UI. They remain optional
ordinary Cargo applications, with no Qt/KDE/GTK toolkit dependency. Advanced
widgets, embedded viewports and the editor shell are planned in issues
[#44](https://github.com/HungLo2020/Quasar/issues/44),
[#45](https://github.com/HungLo2020/Quasar/issues/45) and
[#46](https://github.com/HungLo2020/Quasar/issues/46).
Controller support is available through `quasar-input`'s optional
`gilrs` backend and pure event API; advanced 3D rendering and an editor remain
future additions. Audio is provided by `quasar-audio`; see
[the audio audit and feature matrix](docs/audio-audit.md) and
[the implementation report](docs/audio-implementation-report.md).

## Shared 3D and 2D rendering

Git tag `2.1` introduced caller-owned render state, shared targets and GPU textures,
bounded prepared-command lifetimes, and optional frame helpers. The current
release is `v0.2.16`, with workspace package versions set to `0.2.16`.
The published `v0.2.0` tag remains unchanged. Existing `Renderer2D` callers retain
their API and synchronous retirement by default; native targets are cached and
completion polling can be enabled with `set_frame_overlap(true)`.

```sh
cargo run -p quasar-composition
cargo run -p quasar-composition -- --frames 40 --exercise-window
cargo run -p quasar-composition -- --headless --screenshot /tmp/composition.png
```

See the [composition example](examples/composition/README.md),
[2D API](crates/quasar-render-2d/README.md),
[3D API](crates/quasar-render-3d/README.md), and
[optional frame helper](crates/quasar-frame/README.md) for ownership, cleanup,
GPU tests and reproducible synchronous/overlap measurements. Renderer crates
consume VulkanicGAL's public API. The windowed 2D presenter uses
`quasar-frame` for surface lifecycle; `RenderState2D` composition remains
usable directly on a caller-owned GAL.

### Graphics backend features

VulkanicGAL's backends are Cargo features. `vulkan` is the default and retains
the existing Vulkan implementation. `opengl` is an optional backend for an
existing OpenGL context; this repository does not yet create an OpenGL window,
context or buffer swap. `wgpu` is an optional portability backend that runs the
same GAL on Vulkan, Metal or Direct3D 12 through wgpu. `quasar-render-2d`,
`quasar-render-3d` and `quasar-frame` default to Vulkan and forward
`vulkan`/`opengl`/`wgpu` to the GAL.

```toml
# Vulkan, the default.
quasar-render-2d = { path = "crates/quasar-render-2d" }

# OpenGL-only build with no ash/shaderc dependency.
quasar-render-2d = { path = "crates/quasar-render-2d", default-features = false, features = ["gpu", "opengl"] }
quasar-render-3d = { path = "crates/quasar-render-3d", default-features = false, features = ["opengl"] }

# wgpu-only build (Metal on macOS, Direct3D 12 or Vulkan elsewhere), no ash.
quasar-render-2d = { path = "crates/quasar-render-2d", default-features = false, features = ["gpu", "wgpu"] }
```

Games that let players choose a backend pass a `GraphicsBackend` (`auto`,
`vulkan`, `wgpu-vulkan`, `wgpu-metal`, `wgpu-dx12`, ...) through `GalOptions`;
`VulkanicGal::backend_info()` (also on `Renderer2D`) reports what actually runs
for debug screens. Games that do not care get native Vulkan by default. See the
[VulkanicGAL README](crates/vulkanic-gal/README.md#choosing-and-reporting-the-backend).

`VULKANIC_BACKEND=wgpu` (or `vulkan`, `opengl`, `wgpu-vulkan`, ...) overrides
automatic selection and the `BackendChoice` constructors, so an existing game
runs on another compiled-in backend without code changes; explicit
`GalOptions` choices are only overridden when they allow it:

```sh
VULKANIC_BACKEND=wgpu cargo run -p starfall --features vulkanic-gal/wgpu
```

`Renderer2D::headless_with(BackendChoice)` and
`Renderer2D::from_windowed_gal(...)` request a renderer rather than a Vulkan
renderer. `for_window` is the window convenience path: Vulkan by default, wgpu in
builds without Vulkan. Build without
a backend to use the GAL frontend with a caller-provided backend or for
diagnostics; backend creation then returns an explicit unsupported error.

For opt-in per-pixel 3D lighting, classic/unlit materials and custom fragment
shaders, directional/spotlight shadows and normal maps, see
[3D lighting](docs/lighting-3d.md), [surface/shadow rendering](docs/shadows-3d.md) and the playable
[Nightwatch FPS example](examples/nightwatch/README.md). This path preserves
legacy rendering and requires no scene, ECS or application runner.

## Software framebuffer presentation

`quasar-render-2d::FramebufferPresenter` presents a tightly packed RGBA8
software framebuffer through the same GPU quad pipeline, viewport math, and
surface lifecycle as native 2D rendering. The texture, view, samplers, bindings,
and per-frame upload slots persist; only pixels are uploaded. Fit/integer
scaling, nearest/linear filtering, letterboxing, resize, minimized-surface
deferral, and completion polling match `Renderer2D`. `write_png` and
`render_to_image` remain available for headless output.

```sh
cargo run -p framebuffer -- --frames 30 --exercise-window
cargo run -p framebuffer -- --headless --screenshot /tmp/framebuffer.png
cargo run -p framebuffer -- --frames 364 --benchmark /tmp/framebuffer-bench.json
```

For shared-device composition, `RenderState2D::create_dynamic_texture` and
`update_dynamic_texture` provide the same persistent-texture path without owning
a window, so software framebuffers can be drawn alongside 3D, native 2D, and
direct VulkanicGAL passes in one submission. See the
[framebuffer example](examples/framebuffer/README.md).

## Audio

`quasar-audio` is an independent, device-optional audio system: deterministic
sample-frame mixing, WAV/Ogg decoding, streaming Ogg, buses and voice groups,
priority/limit policies, spatial attenuation and pan, procedural sources, and an
optional CPAL backend. The same mixer renders offline PCM/WAV for tests and
benchmarks, so audio verification does not require speakers.

```sh
cargo run -p audio-example -- --offline /tmp/audio-example.wav
cargo run -p audio-example -- --devices
cargo run -p audio-example -- --play
```

See the [audio example](examples/audio/README.md), [audit and feature
matrix](docs/audio-audit.md), and [implementation
report](docs/audio-implementation-report.md).

## Configurable controls

Pong defines action names; `quasar-input` handles devices, bindings, and action
transitions. Export a preset, edit the keys/mouse buttons/chords/axes, and load it:

```sh
cargo run -p pong --no-default-features -- --write-default-bindings /tmp/pong-controls.json
cargo run -p pong --no-default-features -- --check-bindings /tmp/pong-controls.json
cargo run -p pong -- --bindings /tmp/pong-controls.json
```

Normal desktop runs load `data/config/shared/controls.json`, honoring
`BASALT_SHARED_CONFIG_DIR`. Missing controls initialize with defaults; invalid
or unsupported saved files are preserved while defaults are used. Explicit
presets fail on errors and apply only to that run. `--no-save` skips stored
controls; explicit presets are still honored. HUD hints show the first binding
for each action. Replay files store game actions, so rebinding physical controls
does not change recorded playback.

See [quasar-input's API and semantics](crates/quasar-input/README.md).

## Build and verify

Use a current stable Rust toolchain (minimum 1.94; developed with Rust 1.94). The initial
build needs CMake, Ninja or Make, and a C++ compiler to build shaderc from source.
On Debian/Ubuntu, useful development packages are `build-essential cmake ninja-build
pkg-config libxkbcommon-dev libxkbcommon-x11-0 libasound2-dev libwayland-dev
libx11-dev libxrandr-dev libxi-dev libxcursor-dev libvulkan-dev`. A Vulkan loader/driver and a desktop session are
needed to play; no standalone GLSL compiler is needed at runtime.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test -p quasar-audio --no-default-features --locked
cargo test -p quasar-text --features serde --locked
cargo run --release -p quasar-text --example text_benchmark
cargo test -p quasar-ui --features text-render2d --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --workspace --no-deps --locked
python3 tools/verify_pong.py
```

Audio can be verified without a sound device:

```sh
cargo run -p audio-example -- --offline /tmp/audio-example.wav
cargo run -p audio-example -- --devices
cargo run -p audio-example -- --play   # requires a working output device
```

Imported OpenGL/GPU checks and the 2D renderer conformance tests are explicitly ignored in ordinary tests because
they require a graphics environment. Run the full suite with driver access, and again on wgpu:

```sh
cargo test --workspace --locked -- --include-ignored
VULKANIC_BACKEND=wgpu cargo test --workspace --features vulkanic-gal/wgpu -- --include-ignored --test-threads=1
cargo run -p pong -- --autoplay --no-save --smoke-frames 30 --exercise-window
```

The native exercise changes size, enters/exits fullscreen, changes sampling,
and verifies that a zero-sized presentation surface defers rendering and then
recovers. It fails if the requested frames cannot be presented within 30 seconds.
Some other imported bootstrap checks tolerate absent GPU drivers; ordinary test
success alone does not establish native graphics correctness.

## Headless execution and replay

No window or GPU is initialized by `--headless`. For a build that also excludes
windowing/Vulkan dependencies, disable the example's default features:

```sh
cargo run -p pong --no-default-features -- --headless --autoplay --ticks 12000 \
  --record /tmp/pong-inputs.json --state /tmp/pong-state.json --screenshot /tmp/pong.png
cargo run -p pong --no-default-features -- --headless \
  --replay /tmp/pong-inputs.json --state /tmp/pong-replayed.json
cmp /tmp/pong-state.json /tmp/pong-replayed.json
```

`--record` also records real keyboard play. Replays contain per-tick game inputs,
mode, version, and simulation rate. Native and headless playback use the same
simulation. Determinism is checked within this implementation; replay compatibility
across future gameplay changes or all floating-point architectures is not promised.
Native screenshots use GPU readback at the logical game resolution. Headless
screenshots use the renderer's CPU reference path with the same draw list.
`cargo run -p pong -- --help` lists all options.

## Packaging and storage

```sh
python3 tools/package_pong.py
# Or package an already-built desktop executable during development:
python3 tools/package_pong.py --binary target/debug/pong
```

The ZIP contains only `app/bin/pong`, box art, and runtime notes. Normal play
writes `data/config/display.json`, `data/config/shared/controls.json`, and
`data/save/records.json`, relative to the
installation, and honors Basalt directory overrides. Checkout runs use this
repository's `data/` directory. Missing data initializes automatically; corrupt
or unsupported files are preserved. `--no-save` skips these settings and records;
headless execution never loads or writes them. Explicit CLI output paths are
always honored.

[packaging/Pong.json](packaging/Pong.json) is a proposed Linux x64 Basalt catalog
entry. The package command does not publish a release or install the catalog
entry. Packages use the build host's libc baseline; a portable Linux baseline
and other platforms' runtime bundling still need release validation.

See [architecture and extension guidance](docs/architecture.md) and
[VulkanicGAL provenance](crates/vulkanic-gal/UPSTREAM.md).

## Tilemaps and scrolling 2D game

```sh
cargo run -p tilequest
cargo run -p quasar-tilemap -- validate examples/tilequest/assets/island.tmj --assets
python tools/verify_tilemap.py
```

[Tilequest](examples/tilequest/README.md) exercises animated PNG atlases, flipped
sprites, ordered foreground/parallax layers, camera culling, collision properties
and object markers. [quasar-tilemap](crates/quasar-tilemap/README.md) provides the
versioned runtime format, supported Tiled JSON interchange, spatial queries,
conversion/edit/preview CLI and benchmarks. Its default drawing adapter creates
Canvas commands; native rendering remains a consumer of VulkanicGAL through
quasar-render-2d. Parsing and simulation also run without GPU/window dependencies.
The map editor remains future work.

## License

Quasar is licensed under the MIT License. See [LICENSE](LICENSE). The imported
VulkanicGAL source retains its upstream provenance as documented in
[crates/vulkanic-gal/UPSTREAM.md](crates/vulkanic-gal/UPSTREAM.md).

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
| `quasar-input` | Portable keyboard/mouse state, named actions, axes, chords, serializable bindings | None |
| `quasar-window` | Native windows, desktop identity, fullscreen, DPI, cursor capture, native input adapter | quasar-input |
| `quasar-core` | Fixed-step timing and shared viewport/scaling math | None |
| `quasar-storage` | Basalt v1 paths and atomic JSON persistence | None |
| `quasar-audio` | WAV/Ogg decode, deterministic mixing, buses, spatial audio, scheduling, streaming Ogg, offline WAV and CPAL output | Optional quasar-storage (settings) |
| `vulkanic-gal` | Independent extraction of MattMC's graphics abstraction layer | None |
| `quasar-render-2d` | Ordered instanced quads, sprites/atlases, bitmap text, camera/transforms, clipping, GPU readback, persistent software-framebuffer presentation | VulkanicGAL, quasar-core, quasar-frame |
| `quasar-tilemap` | Versioned/Tiled maps, indexed tile/object queries, PNG atlases, animation, culled draw lists and CLI | Optional quasar-render-2d |
| `quasar-render-3d` | Indexed meshes, camera/model transforms, textured materials, lighting, depth and prepared passes | VulkanicGAL |
| `quasar-frame` | Optional recording transactions, target caching, acquire/present/cancel and completion polling | VulkanicGAL |
| `examples/tilequest` | Scrolling top-down quest, layered maps, collision, replays and headless/native verification | Selected crates |
| `examples/framebuffer` | CPU-rendered RGBA8 framebuffer presented through `quasar-render-2d`, with headless screenshots, window exercises and benchmarks | quasar-render-2d |
| `examples/audio` | Music, overlapping/spatial effects, buses, voice groups, procedural audio, offline WAV output and audio benchmarks | quasar-audio |
| `examples/pong` | Game rules, 2D draw lists, desktop application, headless CLI and replay | Selected crates |

The desktop example submits a 640×400 logical canvas to `quasar-render-2d`,
which draws GPU quads and font atlas sprites through VulkanicGAL. It uses
native X11/Wayland on Linux; window/surface integration also has Win32 and macOS
paths through winit/ash-window. Only Linux has been verified so far. Native
Vulkan requires a Vulkan 1.3 driver with dynamic rendering, synchronization2,
and timeline semaphores. macOS additionally needs a compatible Vulkan
portability driver; distribution of that runtime is future work. x86-64 is the
verified architecture; arm64 and riscv64 remain unverified targets.

There is no editor yet. The working interpretation of [requirements](REQUIREMENTS.md)
is that a future optional editor uses Qt/KDE frameworks; standalone game windows
use Rust/winit. Controller/analog backends, advanced 3D rendering,
and an editor are future additions. Audio is provided by `quasar-audio`; see
[the audio audit and feature matrix](docs/audio-audit.md) and
[the implementation report](docs/audio-implementation-report.md).

## Shared 3D and 2D rendering

Git tag `2.1` introduced caller-owned render state, shared targets and GPU textures,
bounded prepared-command lifetimes, and optional frame helpers. The current
release is `v0.2.4`, with workspace package versions set to `0.2.4`.
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
context or buffer swap. `quasar-render-2d` and `quasar-render-3d` default to
Vulkan and forward `vulkan`/`opengl` to the GAL.

```toml
# Vulkan, the default.
quasar-render-2d = { path = "crates/quasar-render-2d" }

# OpenGL-only build with no ash/shaderc dependency.
quasar-render-2d = { path = "crates/quasar-render-2d", default-features = false, features = ["gpu", "opengl"] }
quasar-render-3d = { path = "crates/quasar-render-3d", default-features = false, features = ["opengl"] }
```

`Renderer2D::headless_with(BackendChoice)` and
`Renderer2D::from_windowed_gal(...)` request a renderer rather than a Vulkan
renderer. `for_window` remains the Vulkan window convenience path. Build without
a backend to use the GAL frontend with a caller-provided backend or for
diagnostics; backend creation then returns an explicit unsupported error.

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
they require a graphics environment. Run the full suite with driver access:

```sh
cargo test --workspace --locked -- --include-ignored
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

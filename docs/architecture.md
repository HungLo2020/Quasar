# Architecture and extension guide

## Ownership

Pong's `main.rs` selects desktop or headless execution. `lib.rs` contains only
game rules and a software renderer; the desktop runner is optional. No engine
runner, asset compiler, editor, project format, or special build command is
required. Dependencies always point from examples/high-level helpers toward
independently usable low-level crates.

The desktop runner implements winit's ordinary `ApplicationHandler`.
`quasar-window` re-exports winit so callers can use its complete event model,
monitor access, raw handles, icons, and advanced window operations. `GameWindow`
adds shared lifecycle and input behavior without hiding those capabilities.

`quasar-render-2d` contains the windowed 2D renderer and the
`FramebufferPresenter` for CPU-rendered RGBA8 framebuffers. It accepts any owned
window implementing raw-window-handle traits and uses `quasar-frame` for target
caching, acquire/present, cancellation and completion polling. A custom
application can use `RenderState2D` on a caller-owned GAL instead, or skip the
presenter and record directly through VulkanicGAL for higher-level 2D/3D
rendering. Dynamic RGBA8 textures are persistent GPU resources updated through
per-frame upload slots, so they compose with 3D and direct GAL work.

## Windowing and input contract

Create windows inside `ApplicationHandler::resumed`. Feed matching window events
to `GameWindow::process`, and feed device events to `process_device`. Close,
resize, DPI, focus and redraw events remain visible to the application.

`GameWindow::create` keeps the existing `WindowConfig` contract and applies a
default `DesktopConfig` derived from `application_id`. Games that need distinct
X11 instance/class names, an explicitly transparent surface, or a different
opaque-region policy call `create_with_desktop`. X11 identity, `_NET_WM_PID`,
normal-window type, full opacity and the physical-size `_NET_WM_OPAQUE_REGION`
are applied before the call returns, so slow VulkanicGAL initialization cannot
delay them. `process` reconciles the opaque region after resize, DPI and
fullscreen events; `sync_desktop` is available after caller-owned graphics
initialization. Wayland uses winit's application id, Windows a Win32 class name,
and macOS bundle identity; unsupported X11 properties are never fabricated. The
`x11` Cargo feature is default-on Linux only and can be disabled without
affecting Wayland or non-Linux builds. See
[desktop window identity](window-identity.md) for the reference-game audit,
property contract, migration examples and validation procedure.

`quasar-diagnostics` is an explicit, independently usable observability layer.
It has no global logger or runtime; callers own a `Diagnostics` handle and a
`Sink`. Records are bounded structured data with optional spans, counters,
failure chains and CPU frame timing behind a `timing` feature. Window, input,
audio, frame and asset hooks are optional Cargo features and accept an explicit
handle, so default builds do not compile the diagnostics crate or format records.
See [diagnostics](diagnostics.md).

`quasar-input` owns backend-independent physical key/mouse/gamepad identifiers,
device state, named action values, and accumulated press/release edges. It
depends only on serde by default. Controller types, `GamepadHub` player routing
and `GamepadEvent` ingestion are pure Rust; the optional `gilrs` feature adds the
native adapter without changing the core dependency graph. One `PlayerInput` per
player combines keyboard, mouse and assigned-gamepad bindings; deadzones,
calibration, reconnect preferences and multiple local players stay in the input
layer rather than a renderer or runner. `quasar-window::input` translates native
winit events; `GameWindow` automatically feeds its public `InputState` through
that adapter. See [controller input](controller-input.md) for hotplug, player
routing, calibration and platform notes.

Games install serializable `Bindings` and define their own action names. Digital
buttons, opposing-button axes, and chords can feed an action; alternatives sum
and clamp to [-1,1]. Actions update on each device change, preserving short taps
and correct release behavior when alternate bindings overlap. Empty action lists
are deliberate unbindings. Wheel units and relative mouse motion remain raw data;
controller buttons and analog axes use the gamepad types described above.

Call `consume_edges` after the first consuming simulation tick, not per render
or native event. Held values survive catch-up ticks. OS repeats do not create
new physical press edges. Text remains separate from physical key state.

Focus loss releases devices and clears stale action presses, motion and text. The
window layer releases/shows the cursor; pausing simulation remains game policy.

## Asset loading and caching

`quasar-assets` is a headless, synchronous layer between filesystems/archives and
games. `AssetPath` is validated and portable; `AssetServer` resolves the longest
matching mount prefix, gives the source a relative path, and invokes an explicitly
registered typed `AssetLoader`. Loaders never guess a decoder from a file
extension. `Handle<T>` owns an `Arc<T>`, so cache eviction cannot invalidate
values still in use. `Asset::dependencies` records graph edges used for
`load_order`, `dependents` and unload safety; loader/asset schema versions and
`LoadContext::require_schema_version` make incompatible data an explicit error.
`FileSource` canonicalizes and confines reads to its root (including symlinks),
while `MemorySource` covers embedded/generated content and tests. Budgets limit
entries and bytes and evict only unreferenced, non-dependency values. The core
has no required dependencies; optional features add serde paths/IDs and Basalt
path helpers. Asset files need not be in Git and no editor or build tool is
involved. Optional `async`/`hot-reload` features add a bounded serial worker and
`notify` watcher; typed reloads are explicit, cache revisions expose conflicts,
and old `Handle` values remain valid. See [asset management](asset-management.md)
and [async assets](async-assets.md) for the API, cache and lifecycle rules.

Pong consumes named paddle/serve/pause/restart actions. Desktop commands such as
fullscreen/filter/scale/quit also use named bindings, consuming their own edges
through `take_pressed` immediately after event translation. This prevents a
pending fullscreen edge from toggling again before the next simulation tick
without discarding unrelated gameplay input. Its HUD receives binding labels
as plain strings, so the software renderer does not depend on an input backend.

`native()` exposes the underlying winit window for monitors, title/icon changes,
physical/logical conversions, positioning, and other advanced operations.
`shared_native()` retains ownership when constructing graphics resources.
Cursor locking falls back to confinement if the platform does not support a lock.

## Presentation and lifetime

VulkanicGAL's `vulkan` and `opengl` backends are independent Cargo features;
`vulkan` is the default. Vulkan-only dependencies (`ash`, `ash-window`,
`shaderc`) and OpenGL-only dependencies (`glow`) are not compiled unless their
feature is selected. `BackendChoice` remains available in both configurations;
requesting a backend that was compiled out returns an explicit unsupported
error. A future Direct3D, Metal or software backend can follow the same feature
and frontend pattern without changing the renderer API.

`Renderer2D::headless_with(BackendChoice)` and
`Renderer2D::from_windowed_gal(...)` are backend-neutral creation paths.
`Renderer2D::headless()` and `for_window()` retain the Vulkan default. The
presenter creates an owned Vulkan surface through ash-window. The backend
retains an Arc to the native window until its Vulkan resources have been
destroyed. The legacy raw-integer native-window constructor is unsafe: its
caller must guarantee native handle validity and lifetime. OpenGL keeps its
existing borrowed/isolated-context behavior; Quasar does not create an OpenGL
window, context or buffer swap. Framebuffer pixels are uploaded into a
persistent RGBA8 texture; a fullscreen quad uses the same viewport math as
other 2D draws.

A framebuffer is tightly packed RGBA8, top row first. Source dimensions stay
fixed for one presenter. A persistent sampled GPU texture and a fullscreen quad
handle letterboxing, nearest/linear filtering and scaling. `Viewport` maps
physical cursor coordinates back to source pixels and rejects positions in the
bars. A custom display aspect accommodates non-square source pixels. Integer
scaling falls back to fit when the window is too small, and custom non-square
proportions use fit scaling.

Resize events mark the surface for recreation before the next acquisition.
Zero-sized windows defer presentation. Acquire/present status handles changing
and suboptimal swapchains. OS suspension drops the presenter before the game
window; resume reconstructs both while preserving simulation state.

`FrameSurface` caches acquired targets and `RenderState2D` caches passes.
Up to three prepared slots keep geometry, uniforms and framebuffer uploads
separate until their submission completes; synchronous retirement remains the
default and completion polling is opt-in. Dynamic framebuffer pixels are staged
per prepared command list, so out-of-order submission cannot overwrite an
in-flight frame's contents. The imported GAL still serializes graphics backend
lifetimes with a global lock; multiple simultaneous GAL instances are not
currently supported by this layer.

## Timing and automation

`FixedStep` accepts a game-selected frequency, including fractional rates. It
bounds catch-up work and exposes the next deadline and interpolation fraction.
The application supplies elapsed time; there is no global clock or compulsory
loop. Pong uses 120 Hz. Native rendering does not advance simulation.

Pong records action inputs at the simulation boundary. Focus-loss pausing is
recorded as a pause input, so it replays without needing OS focus events.
Headless execution advances directly by tick count. Autoplay is a deterministic
exercise driver, not proof that a human can complete a particular game.

## Audio

`quasar-audio` is independent of the renderers, window, input, storage, ECS and
application loop. `Mixer` is a deterministic sample-frame mixer with no device
dependency; `OfflineRenderer` writes the same mix to PCM/WAV for headless
verification. The optional CPAL backend owns a device thread and feeds the same
`Mixer`.

A mixer has a master bus, caller-created named buses, and optional voice groups.
Buses carry gain/mute/voice limits; groups carry admission limits independent of
mixing. Voices reference immutable `Arc` clip data, a bounded streaming decoder,
or a procedural generator. `VoiceHandle` is a stable identity with status and
position, not a reusable slot.

Scheduling uses an absolute 64-bit mixer frame. Simulation time remains the
game's clock; a fixed-step game converts its tick to mixer frames and calls
`play_at`. Mixer frames stop while globally paused. Requested frames in the past
start on the next rendered frame and increment a late-start diagnostic. Device
presentation is later than mixer time by backend buffering; no API claims
sample-accurate speaker output.

The native callback drains a bounded crossbeam command queue with `try_recv` and
publishes atomics for diagnostics. Streaming decoder threads are started on the
caller and use bounded queues; they never run in the callback. CPAL streams are
created and destroyed on one device-owner thread because some backends mark them
`!Send`; the `AudioEngine` handle is `Send + Sync`. Device loss is reported, and
`reopen` reconstructs the engine without pretending active voices survived.

WAV and Ogg Vorbis are decoded (`hound`, `lewton`); Ogg can stream. Spatial
models support listener transforms, inverse/linear/exponential attenuation,
stereo pan and a game-supplied `SpatialModel`. `AudioSettings` is an optional
schema-versioned adapter for Basalt-resolved paths; it does not hardcode a game
directory or own game-specific sound names.

## Persistence and Basalt

`quasar-storage` resolves each Basalt location from its environment override.
Without overrides, packaged `app/bin/<executable>` resolves its install root
from the executable rather than the current working directory. Applications
can provide a development root; otherwise the executable directory is used.
Only requested writes create directories.

JSON writes use a temporary file in the destination directory and atomic
replacement. Typed callers own schema versions and migration. Reads distinguish
missing files from malformed data. Pong accepts schema version 1, initializes
missing files, and preserves unsupported/corrupt display settings, controls or records.
Controls are stored under the shared configuration directory and validate action
names and schema version. Explicit CLI presets are validated before graphics
initialization and never replace saved controls.

VulkanicGAL's shader disk cache is disabled unless `VULKANIC_SPIRV_CACHE_DIR` is
explicitly set. The library does not invent a HOME/XDG location. Applications
can opt in with a directory under their resolved cache path. Driver-owned shader
caches are outside the application's storage API.

## Boundaries for the next consumers

Adopt the window/input and pixel presenter APIs first for a software-rendered
port. Keep its original simulation frequency, pixel dimensions, display aspect,
input meanings, and headless entry point. No other repository was changed for
this milestone. Audio and controller backends can be added independently; named
keyboard/mouse action mapping already lives in `quasar-input`. The editor remains
optional and should call the same public APIs as code/CLI tooling.

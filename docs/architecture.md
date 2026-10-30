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
rendering. Dynamic RGBA8 texture IDs retain immutable native upload generations, so they compose with 3D and direct GAL work.

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

`quasar-config` is an optional typed settings/CLI foundation; it owns no global
configuration and writes no user files. See [configuration](configuration.md).
`quasar-profile` standardizes bounded CPU/GPU frame, upload, resource and
synchronization measurements; it owns no GAL or renderer. See
[profiling](profiling.md). The [Starfall reference game](../examples/starfall/README.md)
combines these layers with the existing window, input, jobs, assets, audio and
renderers.

`quasar-spatial` is a dependency-free 2D/3D geometry and spatial-index crate.
`Aabb`, `Ray`, `Plane` and `Frustum` are usable alone; `SpatialGrid<N, T>` is a
deterministic spatial hash with caller keys, bounded large-item handling and
radius/frustum/ray/closest queries. It owns no ECS, renderer or world origin.

`quasar-physics` is an optional thin adapter over `rapier2d`/`rapier3d`. It
provides caller-owned collision-only worlds, bounded fixed-step dynamics,
collision groups, sensors/events and optional `quasar-spatial` index conversion,
while `Deref` and Rapier re-exports keep every advanced engine API available. It
has no renderer, window, audio, ECS or application-runner dependency.

`quasar-animation` is a renderer-neutral timeline/state-machine layer. Clips,
tracks, markers, players, pose blending and layered transitions have no ECS,
scene graph or GPU dependency. Optional sprite conversion and skeletal pose
adapters keep engine integration thin; games own entities and update cadence.
See [animation](animation.md).

`quasar-nav` owns generic graph/grid path queries and optional tilemap,
spatial, jobs and navmesh adapters. It has no agent steering, ECS or renderer;
games own decision-making and movement. See [navigation](nav.md).

`quasar-text` is an optional, renderer-independent text stack. Callers register
their own TTF/OTF bytes; it shapes with cosmic-text/Harfrust, lays out wrapped
and bidirectional text, exposes caret/hit/selection geometry, and rasterizes
glyphs into reusable CPU atlas pages. It owns no window, GPU object, ECS or
application runner. `quasar-render-2d` can upload changed atlas pages through
persistent dynamic textures, and `quasar-ui` can use the same shaped layouts for
widget measurement, editing and drawing. See [text](text.md) for audits,
fallback/licensing policy, measurements and limitations.

`quasar-ui` is the shared retained UI foundation for games, development tools
and the future optional Quasar editor. Its current controls support menus,
pause screens, options dialogs, HUD widgets, modal confirmations and overlays.
It owns widget trees, layout, focus, modal scopes, text editing and
consumed-input reporting,
but no window, GPU device, ECS or scene graph. Games advance it with their own
input snapshot and decide what each action means. The optional `render2d`
feature appends quads to the caller's `quasar-render-2d` canvas on the same GAL
frame, so UI never creates a device or submission. See [shared UI](ui.md) for
the audit, composition model, themes, editor roadmap and current limitations.

The future editor and visual tilemap tools use this same public UI API and
Quasar's window/input/rendering infrastructure. General-purpose controls,
including docking, trees, tables, inspectors and embedded viewports, belong in
the shared libraries; editor-specific document/workspace coordination belongs
in the optional tool. These advanced controls and the editor executable are
not implemented yet. The editor depends on the UI, with no reverse dependency
from games or UI libraries, and uses no Qt/KDE/GTK widget toolkit. Essential
editing operations must also work through public Rust APIs and CLI/headless
workflows, preserving ordinary Cargo projects and optional runner adoption.

`quasar-sim` is an optional deterministic simulation layer built on generic
game-owned state. It reuses `quasar-core::FixedStep` for scheduling, records
validated input/hash journals, verifies replay divergence, and supports
caller-owned snapshots and state hashes. It imposes no ECS, state layout,
renderer or network. See [simulation](simulation.md).

`quasar-jobs` is an optional bounded worker pool for caller-owned CPU/file work.
It has no global scheduler, async runtime, window or ECS dependency. Queued
cancellation, cooperative running cancellation, dependencies, priorities,
manual deterministic execution, metrics and optional diagnostics are described
in [jobs](jobs.md). It is not required by asset loading, rendering or games.

`quasar-app` is an optional lifecycle composition layer, not a required runner.
Its `Application` trait separates `frame_update` (variable-rate UI/camera work)
from fixed `tick` steps and `render`, with one `FrameInput` snapshot shared by
all phases. The default `desktop` feature adds one `GameWindow`, winit event
forwarding, fixed/variable scheduling, focus and suspend/resume handling, and
ordered shutdown hooks. Applications own their GAL, renderer, audio and assets,
and can bypass the runner entirely. See [app runner](app-runner.md) for the
phase/input/timing contract, [adoption analysis](app-adoption.md) for the
MissileCommand/MicroMages audit and
[the v0.2.11 migration guide](app-migration-v0.2.11.md).

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
Native auto-repeat never creates a physical/action press edge; repeated printable
text still reaches `InputState::text` for chat, console and UI editing. Focus loss
releases held devices; gamepad routing adds `PlayerInput`/`GamepadHub` release
helpers that preserve assignments. See [input hardening](input-hardening.md).

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

VulkanicGAL is an unmodified upstream wgpu re-export. Native types and command
buffers pass directly between renderers and applications. Only VulkanicGAL owns
the external graphics dependency; architecture tests enforce this boundary.
`quasar-frame` supplies optional native setup, surface/readback and profiling
helpers. See [native graphics migration](native-wgpu-migration.md) for the current
API, feature forwarding, ownership and composition rules.

Surfaces retain an Arc window, defer zero-size acquisition and recover from
outdated/lost images. A SurfaceFrame owns an acquired image until presentation
or Drop. No global device lock or fixed prepared-frame registry exists.
Native command buffers retain their resources; explicit encoder ordering governs
writes, render passes and copies. Updated dynamic images use immutable snapshots
so preparing or submitting another frame cannot corrupt an outstanding frame.
The `Viewport` still handles fit/integer scaling, aspect correction, nearest/linear
filtering, letterboxing and physical mouse coordinates.

## Timing and automation

`FixedStep` accepts a game-selected frequency, including fractional rates. It
bounds catch-up work and exposes the next deadline and interpolation fraction.
`FixedStepConfig` adds an optional per-advance elapsed clamp, a per-advance
catch-up budget, drop/preserve backlog policy, exact-step construction and
advance accounting while `FixedStep::new` keeps its historical defaults. The
application supplies elapsed time; there is no global clock or compulsory loop.
Pong uses 120 Hz. Native rendering does not advance simulation. See
[timing](timing.md) for the MicroMages deadline comparison and issue #41
boundary.

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

Shader/pipeline cache behavior belongs to upstream wgpu and drivers. Quasar does
not implement a Shaderc/SPIR-V disk cache or choose a hidden cache directory.
Explicit application caches can use Basalt-resolved locations.

## Boundaries for the next consumers

Adopt the window/input and pixel presenter APIs first for a software-rendered
port. Keep its original simulation frequency, pixel dimensions, display aspect,
input meanings, and headless entry point. No other repository was changed for
this milestone. Audio and controller backends can be added independently; named
keyboard/mouse action mapping already lives in `quasar-input`. The editor remains
optional and should call the same public APIs as code/CLI tooling.

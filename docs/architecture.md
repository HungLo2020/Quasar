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

`quasar-present` accepts any owned window implementing raw-window-handle traits;
it has no dependency on `quasar-window`. A custom application can use either
crate alone. It can also skip the framebuffer presenter and record directly
through VulkanicGAL for higher-level 2D/3D rendering.

## Windowing and input contract

Create windows inside `ApplicationHandler::resumed`. Feed matching window events
to `GameWindow::process`, and feed device events to `process_device`. Close,
resize, DPI, focus and redraw events remain visible to the application.

`quasar-input` owns backend-independent physical key/mouse identifiers, device
state, named action values, and accumulated press/release edges. It depends only
on serde. `quasar-window::input` translates native winit events; `GameWindow`
automatically feeds its public `InputState` through that adapter.

Games install serializable `Bindings` and define their own action names. Digital
buttons, opposing-button axes, and chords can feed an action; alternatives sum
and clamp to [-1,1]. Actions update on each device change, preserving short taps
and correct release behavior when alternate bindings overlap. Empty action lists
are deliberate unbindings. Wheel units and relative mouse motion remain raw data;
controller/analog backends are not implemented yet.

Call `consume_edges` after the first consuming simulation tick, not per render
or native event. Held values survive catch-up ticks. OS repeats do not create
new physical press edges. Text remains separate from physical key state. Focus
loss releases devices and clears stale action presses, motion and text. The
window layer releases/shows the cursor; pausing simulation remains game policy.

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

The presenter creates an owned VulkanicGAL surface through ash-window. The
backend retains an Arc to the native window until its Vulkan resources have
been destroyed. The legacy raw-integer native-window constructor is unsafe:
its caller must guarantee native handle validity and lifetime.

A framebuffer is tightly packed RGBA8, top row first. Source dimensions stay
fixed for one presenter. A sampled GPU texture and a fullscreen triangle handle
letterboxing, nearest/linear filtering and scaling. `Viewport` maps physical
cursor coordinates back to source pixels and rejects positions in the bars.
A custom display aspect accommodates non-square source pixels. Integer scaling
falls back to fit when the window is too small, and custom non-square proportions
use fit scaling.

Resize events mark the surface for recreation before the next acquisition.
Zero-sized windows defer presentation. Acquire/present status handles changing
and suboptimal swapchains. OS suspension drops the presenter before the game
window; resume reconstructs both while preserving simulation state.

The initial presenter uses one frame in flight and waits for submission
retirement before reusing its upload storage. This favors straightforward
resource ownership. It is a functional baseline, not a high-throughput renderer;
per-acquisition frame targets and passes could be cached/pool-managed later.
The imported GAL also serializes graphics backend lifetimes with a global lock;
multiple simultaneous GAL instances are not currently supported by this layer.

## Timing and automation

`FixedStep` accepts a game-selected frequency, including fractional rates. It
bounds catch-up work and exposes the next deadline and interpolation fraction.
The application supplies elapsed time; there is no global clock or compulsory
loop. Pong uses 120 Hz. Native rendering does not advance simulation.

Pong records action inputs at the simulation boundary. Focus-loss pausing is
recorded as a pause input, so it replays without needing OS focus events.
Headless execution advances directly by tick count. Autoplay is a deterministic
exercise driver, not proof that a human can complete a particular game.

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

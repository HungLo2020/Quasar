# quasar-app

Optional lifecycle composition for graphical and headless Quasar applications.
It is not an ECS, renderer, editor or required runtime. Lower-level crates remain
usable without it.

```toml
quasar-app = { path = "../Quasar/crates/quasar-app" }
```

Minimal headless application:

```rust
use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless};

struct Game;
impl Application for Game {
    type Error = std::convert::Infallible;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> { Ok(()) }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Ok(Flow::Continue)
    }
}

run_headless(Game, HeadlessConfig { ticks: 60, ..HeadlessConfig::default() })?;
```

Desktop applications implement `DesktopApplication` and call `run`:

```rust
use quasar_app::{AppConfig, AppContext, DesktopApplication, Flow, run};

struct Game;
impl quasar_app::Application for Game {
    type Error = std::convert::Infallible;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> { Ok(()) }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Ok(Flow::Continue)
    }
}
impl DesktopApplication for Game {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Game, AppConfig::default())
}
```

## Lifecycle

- `run` creates a normal `EventLoop` and one `GameWindow` in `resumed`.
- `Application::setup` runs after window creation and again after suspension.
- `GameWindow::process`/`process_device` forward input before application hooks.
- `tick` is called per fixed step, or once per frame with
  `TimingConfig::EveryFrame`. The runner consumes the window input edges after
  each successful tick, so applications read `context.window.input` during the
  tick instead of polling/repairing edge state themselves. `window_event` hooks
  can inspect pending edges before the tick.
- `render` is called only from `RedrawRequested`; `RenderPolicy` controls whether
  the runner requests redraws every frame, after a tick, or never.
- `suspended` is called before the window is dropped; release GPU resources here.
- `shutdown` is called once at final teardown.
- `configure_window` runs immediately after window creation/recreation and
  before `setup`; use it for serializable input bindings, cursor capture or
  other mutable window policy.
- `resumed` is a desktop hook run after `setup`; both run again after a
  suspend/resume window recreation.
- `frame`/`tick` are monotonic 1-based counters and stay monotonic across
  resume; elapsed time and the fixed-step accumulator reset so suspended wall
  time is not simulated.
- Errors from setup/ticks/render/window hooks are returned by `run`.

Applications own their VulkanicGAL device, renderer, audio, assets and presenters.
The runner passes a `GameWindow` reference and never owns graphics state. This
supports caller-owned GAL and custom update/render work. There is no mandatory
renderer or lower-level access barrier: applications can use `context.window`
directly or ignore the runner for their own event loop.

## Features

- `desktop` (default): winit lifecycle, window, input forwarding and scheduling.
- No default features: headless-only build, no winit dependency.

`cargo run`, `cargo build`, `cargo test`, and custom feature sets remain standard
Cargo workflows.

## Verification

```sh
cargo test -p quasar-app
cargo test -p quasar-app --no-default-features
cargo clippy -p quasar-app --all-targets -- -D warnings
cargo clippy -p quasar-app --no-default-features --all-targets -- -D warnings
DISPLAY=:99 cargo run -p quasar-app --example desktop
DISPLAY=:99 cargo run -p quasar-app --example desktop -- --headless
cargo run -p quasar-app --example headless
```

The same `Demo` type runs through `run` or `run_headless`, demonstrating a normal
`main` with both desktop and headless paths. The desktop example is exercised
under a nested X server in this repository’s native verification procedure; a
release build completed 30 fixed 60 Hz ticks in 0.51 s wall time, including
window setup and teardown. Headless tests cover setup/tick/shutdown order, early
exit, bounded tick counts and error propagation.

## Limitations

- One window per runner instance. Multiple windows are possible with lower-level
  winit/`GameWindow` code but are not an `AppConfig` feature yet.
- The runner does not create a GAL, renderer, audio device, asset server or
  network client; applications own those resources.
- No application-specific pause/menu policy, save system, hot reload or editor.
- `Application::render` is not called in headless mode.
- Desktop lifecycle behavior was tested on Linux/X11; Windows/macOS compile
  checks are provided by the workspace CI matrix but were not run here.

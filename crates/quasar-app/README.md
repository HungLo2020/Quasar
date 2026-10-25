# quasar-app

Optional lifecycle composition for graphical and headless Quasar applications.
It is not an ECS, renderer, editor or required runtime. Lower-level crates remain
usable without it, and applications can keep their own `winit` loop.

```toml
quasar-app = { path = "../Quasar/crates/quasar-app" }
```

Minimal headless application:

```rust
use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless};

struct Game;
impl Application for Game {
    type Error = std::convert::Infallible;
    fn frame_update(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Ok(Flow::Continue)
    }
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
- `setup` runs once after the first window exists. Window-bound graphics belong
  in `resumed`, which runs after every window creation/recreation.
- `GameWindow::process`/`process_device` forward input before application hooks.
- `frame_update` runs exactly once per frame, before fixed ticks. Use it for UI,
  cameras, networking and visual state.
- `tick` runs zero or more times per frame according to
  `TimingConfig::Fixed`; with `TimingConfig::Variable` it never runs and
  `frame_update` owns scheduling.
- One `FrameInput` snapshot is captured per frame and shared by `frame_update`,
  every tick and `render`. The runner clears transient edges after the first
  successful tick; applications claim UI input with
  `FrameInput::consume_action`/`consume_pointer`/`consume_all`/etc.
- `Flow::Deferred` from `tick` pauses the fixed schedule: no time/tick index
  advance, transient input stays pending and paused backlog is not replayed.
- `render` runs at most once per frame, after the tick batch, when a redraw was
  requested and the window is drawable. `RenderPolicy` controls redraw requests.
- `suspended` is called before the window is dropped; release GPU resources here.
- `shutdown` is called once at final teardown.
- `configure_window` runs immediately after window creation/recreation and
  before `setup`; use it for serializable input bindings, cursor capture or
  other mutable window policy.
- `frame`/`tick` are monotonic 1-based counters and stay monotonic across
  resume; elapsed time and the fixed-step accumulator reset so suspended wall
  time is not simulated.
- Errors from setup/ticks/render/window hooks are returned by `run`; a shutdown
  error is appended instead of hiding the first failure.

Applications own their VulkanicGAL device, renderer, audio, assets and presenters.
The runner passes a `GameWindow` reference and never owns graphics state. This
supports caller-owned GAL and custom update/render work. There is no mandatory
renderer or lower-level access barrier: applications can use `context.window`
directly or ignore the runner for their own event loop.

See [`docs/app-runner.md`](../../docs/app-runner.md) for the full contract and
[`docs/app-migration-v0.2.11.md`](../../docs/app-migration-v0.2.11.md) for the
API migration.

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
DISPLAY=:99 cargo run -p quasar-app --example desktop
DISPLAY=:99 cargo run -p quasar-app --example desktop -- --headless
cargo run -p quasar-app --example headless
cargo run -p app-stress -- --headless --ticks 120
```

The same `Demo` type runs through `run` or `run_headless`, demonstrating a normal
`main` with both desktop and headless paths. Headless tests cover phase ordering,
early exit, zero/one/many ticks, deferred ticks, bounded catch-up, setup/shutdown
deduplication and combined errors. `app-stress` exercises the native and headless
paths together, including jobs, async assets, pause/resume and resize.

## Limitations

- One window per runner instance. Multiple windows are possible with lower-level
  winit/`GameWindow` code but are not an `AppConfig` feature yet.
- The runner does not create a GAL, renderer, audio device, asset server or
  network client; applications own those resources.
- No application-specific pause/menu policy, save system, hot reload or editor.
- `Application::render` is not called in headless mode.
- Desktop lifecycle behavior was tested on Linux/X11; Windows/macOS compile
  checks are provided by the workspace CI matrix but were not run here.

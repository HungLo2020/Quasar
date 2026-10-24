# Optional application runner

Issue #19 is implemented by the independently usable `quasar-app` crate. It
composes existing window/input/timing/lifecycle pieces; it does not introduce an
ECS, renderer, editor, asset runtime or global service container.

## Audit of existing applications

MissileCommand’s `src/desktop.rs` and MicroMages’ desktop path independently
implement the same recurring lifecycle:

- create a `GameWindow` inside `ApplicationHandler::resumed`;
- install the presenter/renderer after window creation;
- forward `WindowEvent` to `GameWindow::process` and device events to
  `process_device`;
- resize the presenter on native size changes and synchronize desktop metadata;
- reset elapsed time on focus loss/suspend;
- accumulate elapsed time and run one or more simulation ticks;
- request a redraw and wait until the next tick/deadline;
- release presenter/GPU state before the window on suspend/exit;
- retain the first failure and print/return it after the event loop.

MicroMages additionally uses a 60 Hz frame deadline and a factory for its custom
native window; MissileCommand uses a fixed-step 120 Hz simulation and a
caller-owned framebuffer presenter. `quasar-app` captures the common sequencing
without copying game rules, their menu state, or their rendering choices. Neither
reference repository was modified.

## Architecture

`Application` is the common, headless-capable trait: `setup`, `tick`, `render`,
`suspended`, `shutdown`. `DesktopApplication` adds window/device event hooks and
`resumed`. The runner consumes `GameWindow` input edges after each successful
tick, so applications inspect `context.window.input` during `tick` and no longer
need their own per-frame `consume_edges` workaround.

`run` owns exactly one `GameWindow` and an ordinary winit `EventLoop`. It:

1. creates the window and applies the desktop policy;
2. calls setup/resumed;
3. forwards native/device events to the window first, then the application;
4. advances a `FixedStep` or calls `tick` once per frame;
5. calls `render` from `RedrawRequested` according to `RenderPolicy`;
6. requests the next deadline and handles errors/shutdown in order.

`run_headless` drives `setup/tick/shutdown` for a bounded tick budget without a
window or winit. `HeadlessConfig` sets the tick count and elapsed step exposed in
`AppContext`. Desktop frame/tick counters are 1-based and remain monotonic
across a suspend/resume window recreation, while elapsed time and the fixed-step
accumulator reset so suspended wall time is never simulated.

See [adoption examples](app-adoption.md) for the read-only MissileCommand and
MicroMages migration analysis and LOC estimates.

Applications own their VulkanicGAL device, renderer, presenter, audio and assets.
They create those resources in `setup` and must release graphics resources in
`suspended`/`shutdown`; the runner drops the window only after those callbacks.
`AppConfig` exposes window identity/desktop policy, fixed or per-frame timing,
render policy and maximum wait. Direct `context.window` access remains available
for advanced lifecycle/renderer work.

## Minimal `main` and headless execution

`crates/quasar-app/examples/desktop.rs` contains one `Demo` application used by
both paths:

```sh
cargo run -p quasar-app --example desktop             # native window
cargo run -p quasar-app --example desktop -- --headless
cargo run -p quasar-app --example headless
cargo test -p quasar-app
```

The binary's `main` selects `quasar_app::run` or `quasar_app::run_headless`
without a custom build system. `Application` is object-agnostic and
`DesktopApplication` is additive, so the same application can support both. The
native example deliberately uses `RenderPolicy::Manual` to show that the
application retains render scheduling control.

## Local startup/pacing measurement

On this Linux/X11 host under an isolated Xephyr server, a release build of the
desktop example completed 30 fixed 60 Hz ticks in **0.51 s wall time**
(`/usr/bin/time`; expected 0.50 s of pacing), including window creation and
teardown. This verifies scheduling and lifecycle, not a comparative benchmark:
the runner's own dispatch cost is not separable from winit and the window server,
and game initialization/render work depends on the application.

## Verification

```sh
cargo test -p quasar-app
cargo test -p quasar-app --no-default-features
cargo clippy -p quasar-app --all-targets -- -D warnings
cargo clippy -p quasar-app --no-default-features --all-targets -- -D warnings
DISPLAY=:99 cargo run -p quasar-app --example desktop
DISPLAY=:99 cargo run -p quasar-app --example desktop -- --headless
```

Headless tests verify setup/tick/shutdown ordering, early `Flow::Exit`, bounded
tick count and error propagation with shutdown. The desktop example was run under
an isolated Xephyr X server; it created a real window, ran 30 fixed ticks and shut
down cleanly.

## Acceptance criteria

- `quasar-app` is optional and independently usable, with a headless-only build.
- Normal `cargo run`, `cargo build` and `cargo test` remain the only workflow.
- A minimal `main` can delegate desktop or headless execution to Quasar.
- Window creation, event forwarding, resize/focus processing, scheduling,
  errors and ordered teardown are reusable rather than copied per game.
- Applications own VulkanicGAL, renderers, presenters, audio and assets.
- Fixed, per-frame and custom render schedules are supported.
- `suspended`/`shutdown` run before the runner drops the window.
- The MissileCommand and MicroMages lifecycle shapes fit one API without a
  game-specific runner branch.
- No ECS, renderer, editor or game-state ownership is required.

## Partial capabilities and limitations

- Single window per runner. Multiple windows require lower-level winit code.
- No GL/audio/asset ownership or automatic device recovery.
- No game-specific pause policy, menu, save, editor or replay integration.
- Render scheduling is intentional only at frame/tick granularity; custom
  multi-pass or offscreen schedules belong in the application.
- Desktop behavior was verified on Linux/X11. Windows/macOS are compile-checked
  by CI but not hardware-verified here.
- No comparative performance claim: the runner adds one event-loop dispatch
  layer and `FixedStep` bookkeeping. The local 30-tick pacing measurement above
  verifies scheduling accuracy, not throughput versus a game's hand-written loop.

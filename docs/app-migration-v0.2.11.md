# Migrating a `quasar-app` application from v0.2.11

This guide covers the desktop-overhaul API change in `quasar-app` and
`quasar-input`. It is written for applications that adopted the runner as of
v0.2.11. Lower-level code that uses `quasar-window`, `quasar-frame` and
VulkanicGAL directly without `quasar-app` is unaffected.

The new contract is deliberately narrower:

- one `frame_update` per frame for variable-rate work;
- zero or more fixed `tick`s per frame for simulation;
- one input snapshot shared by both, with explicit consumption;
- one render pass after the tick batch;
- `setup` once, `resumed` after every window creation/recreation.

## API map

| v0.2.11 | New |
|---|---|
| `TimingConfig::EveryFrame` | `TimingConfig::Variable` |
| `tick` used for both fixed steps and per-frame work | `frame_update` (per frame) and `tick` (fixed step) |
| `context.window.input` (`InputState`) | `context.input` (`FrameInput`), also available in `frame_update` and `render` |
| Runner consumed edges after `tick` | Runner clears transient edges after the first tick; applications explicitly consume UI input in `frame_update` |
| `render` returned `Result<(), Error>` | `render` returns `Result<Flow, Error>` |
| `setup` re-ran after every window recreation | `setup` runs once; `resumed` re-runs after recreation |
| `AppConfig { window, desktop, timing, render, max_frame_wait }` | Adds `frame_interval: Option<Duration>` |
| — | `Flow::Deferred` pauses fixed simulation without losing input |
| — | `TimingInfo`, `AppPhase`, `focus_changed`, `quasar_input::InputRead` |
| `quasar_ui::from_input_state` only | Adds `quasar_ui::from_frame_input` |

Frame and tick numbers remain 1-based. `setup` failures still run `shutdown`, and
the first error is still retained.

## 1. Split per-frame work out of `tick`

In v0.2.11, under `TimingConfig::EveryFrame`, `tick` ran once per frame; under
`Fixed`, it ran once per simulation step. Anything that was really UI, camera or
visual work in that callback must move to `frame_update`.

Before (v0.2.11):

```rust
fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, MyError> {
    let window = context.window.expect("desktop");
    if self.menu.is_open() {
        let input = from_input_state(&window.input, &UiBindings::default());
        self.menu.update(&input, viewport, scale, context.elapsed.as_secs_f32())?;
        return Ok(Flow::Continue);
    }
    if window.input.pressed(Key::Escape) {
        self.menu.open_pause();
        return Ok(Flow::Continue);
    }
    self.simulate(window.input.action("move").value)?;
    Ok(Flow::Continue)
}
```

After:

```rust
fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, MyError> {
    if self.menu.is_open() {
        let input = from_frame_input(context.input, &UiBindings::default());
        self.menu.update(&input, viewport, scale, context.elapsed.as_secs_f32())?;
        context.input.consume_all();
        return Ok(Flow::Continue);
    }
    if context.input.pressed(Key::Escape) {
        self.menu.open_pause();
        context.input.consume_all();
        return Ok(Flow::Continue);
    }
    Ok(Flow::Continue)
}

fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, MyError> {
    if self.menu.is_open() {
        return Ok(Flow::Deferred); // no simulation while paused
    }
    self.simulate(context.input.action("move").value)?;
    Ok(Flow::Continue)
}
```

`Flow::Deferred` is optional: returning `Continue` while paused would still run
the tick (and clear edges). Use `Deferred` when the simulation genuinely did not
run. The runner discards the frame's clock backlog on deferral, so resuming does
not replay wall time spent paused.

## 2. Read input through the frame snapshot

`context.window.input` still exists for raw inspection, but it is the live
pending state, not the frame snapshot. Simulation and UI should read
`context.input`.

Control-mapping functions that took `&InputState` can accept `&impl InputRead` so
they work in both runner and lower-level paths:

```rust
use quasar_input::{Binding, Bindings, InputRead};

pub fn game_input(input: &impl InputRead) -> GameInput {
    GameInput {
        forward: input.action("forward").value,
        fire: input.action("fire").pressed,
        look: [input.relative_motion().0 as f32, input.relative_motion().1 as f32],
    }
}
```

The same function then accepts `&InputState` in unit tests and `&FrameInput` in
`tick`.

Do not call `GameWindow::input.consume_edges()` yourself anymore. The runner
applies `FrameInput` consumption to the pending state and clears transient edges
after the first successful tick. Manual consumption will hide presses from a
later fixed tick.

## 3. Consume UI input explicitly

The runner cannot know whether a key press belongs to a menu or the game. Route
that decision through `FrameInput`:

```rust
if context.input.action("pause").pressed {
    context.input.consume_action("pause");
    self.pause();
}
```

Available consumers: `consume_action`, `consume_button`, `consume_pressed`,
`consume_keyboard`, `consume_pointer`, `consume_text`, `consume_relative_motion`,
`consume_scroll` and `consume_all`. Consumption is applied to the pending input
after `frame_update`, so a claimed edge is not retried on the next frame.

`consume_all` also releases held state (for modal menus). Use the narrower
methods when gameplay should continue underneath a HUD.

## 4. Update render and lifecycle signatures

```rust
fn render(&mut self, context: &mut AppContext<'_>) -> Result<Flow, MyError> {
    if context.drawable() {
        self.draw(context.size)?;
    }
    Ok(Flow::Continue)
}
```

`render` may now return `Flow::Exit`, which is useful for `--frames N` smoke
paths. `setup` and `shutdown` signatures are unchanged.

Window-bound graphics should be created in `resumed`, not `setup`, because
`setup` no longer re-runs after suspension:

```rust
impl Application for MyApp {
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.load_assets()?; // once
        Ok(())
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.renderer.take(); // before the window is dropped
        Ok(())
    }
}

impl DesktopApplication for MyApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        if self.renderer.is_none() {
            self.renderer = Some(create_renderer(context)?);
        }
        Ok(())
    }
}
```

All built-in games follow this pattern; see `examples/pong/src/desktop.rs` and
`examples/starfall/src/desktop.rs`.

## 5. Update configuration

```rust
AppConfig {
    window: WindowConfig { .. },
    desktop: DesktopConfig::for_application("my-app"),
    timing: TimingConfig::Fixed(FixedStepConfig::for_rate(120.0, 8)),
    render: RenderPolicy::EveryFrame,
    max_frame_wait: Duration::from_millis(16),
    frame_interval: None, // new
}
```

`TimingConfig::Variable` replaces `EveryFrame`. In variable mode `tick` is never
called; put all work in `frame_update`.

## 6. Headless behavior changed

`run_headless` still runs one `frame_update` and one attempted `tick` per
iteration, but:

- a deferred tick does not advance `simulation_time` or `tick_index`;
- the iteration count (`HeadlessConfig::ticks`) is consumed even when the tick
  defers;
- input is empty, so a headless application must drive pause/resume itself.

If your headless harness relied on `ticks` counting only completed simulation
steps, account for deferred iterations explicitly or avoid `Flow::Deferred` in
headless mode.

## Checklist

- [ ] Move per-frame UI/camera/network work from `tick` to `frame_update`.
- [ ] Change control mapping functions to accept `impl InputRead`.
- [ ] Replace `window.input` reads in gameplay/UI with `context.input`.
- [ ] Add explicit `consume_*` calls for UI actions, pointer, text and scroll.
- [ ] Add `Flow::Deferred` for real pauses instead of running a no-op tick.
- [ ] Return `Flow::Continue` (or `Exit`) from `render`.
- [ ] Create window-bound graphics in `resumed`; keep `setup` one-time.
- [ ] Add `frame_interval: None` to every `AppConfig` literal.
- [ ] Replace `TimingConfig::EveryFrame` with `TimingConfig::Variable`.
- [ ] Re-run native and headless smoke tests, including resize/fullscreen and
  suspend/resume if the platform emits it.

## Failure modes to watch for

- **Double simulation:** leaving fixed-step logic in `frame_update` while also
  implementing `tick`.
- **Lost presses:** consuming in `frame_update` and again in `tick`, or calling
  `consume_edges` manually.
- **Menu clicks leaking into gameplay:** not consuming pointer/action edges in
  the menu branch.
- **Blank window after resume:** destroying the presenter in `suspended` without
  recreating it in `resumed`.
- **Pause catch-up burst:** returning `Continue` from a paused tick instead of
  `Deferred`, or resetting the clock manually after the runner already reset it.
- **Repeated setup side effects:** relying on `setup` to recreate graphics after
  a window recreation.

## Verification

```sh
cargo test -p quasar-app
cargo test -p quasar-app --no-default-features
cargo run -p app-stress -- --headless --ticks 120
cargo run -p app-stress -- --frames 40 --exercise-window
```

`app-stress` is the reference integration harness: it combines fixed simulation,
variable camera work, an asynchronous job and channel, explicit input
consumption, `Flow::Deferred` pause/resume, resize and fullscreen exercise, and
ordered teardown with a deterministic state hash.

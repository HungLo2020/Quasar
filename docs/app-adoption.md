# Adopting `quasar-app`: MissileCommand and MicroMages

This is a read-only audit. Neither reference repository was modified.

## What `quasar-app` replaces

Both games independently implement these lifecycle pieces:

| Concern | MissileCommand | MicroMages |
|---|---|---|
| Event loop | `desktop::run` creates `EventLoop` | `run_desktop` creates `EventLoop` |
| Window creation | `App::initialize` calls `GameWindow::create_with_desktop` | `window::create` calls `GameWindow::create_with_desktop` |
| Event forwarding | `window_event`/`device_event` call `GameWindow::process*` | `App` forwards to `Session::window_event` → `NativeWindow` |
| Presenter resize | `apply_size` | `NativeWindow` surface resize |
| Focus loss | resets clock/input | session guards |
| Tick scheduling | `FixedStep` in `about_to_wait` | `FrameClock` in `Session::frame`; 60 Hz deadline in `App::about_to_wait` |
| Redraw scheduling | request/wait code | frame deadline code |
| Suspend/exit | drop presenter then window | `exiting` calls `Session::finish` |
| Error capture | `self.error` + event-loop exit | `self.failure` + event-loop exit |

`quasar-app` centralizes those concerns while leaving game rules, input mappings,
audio, assets, presenters, menus and renderer state in the game.

## MissileCommand shape

Source ranges in the audited `src/desktop.rs` (591 lines):

| Range | Current responsibility | What can move to `quasar-app` |
|---|---|---|
| 57–117 `run` | loads records/preferences, opens audio, constructs `App`, runs event loop, saves on exit | event-loop creation/run (104–105); app construction remains |
| 119–147 `App` fields | window, presenter, timing, game session, errors | `window` ownership, event-loop error slot |
| 154–157 `fail` | stores a string and exits the loop | runner error propagation |
| 159–167 `apply_size` | presenter resize | stays (application render resource) |
| 169–211 `initialize` | window/desktop creation, presenter creation, title, timing reset | window/desktop creation, title application, clock reset |
| 359–507 `ApplicationHandler` | forwarding, close/focus/resize, scheduling, redraw request | window/device forwarding, close handling, elapsed/render scheduling, teardown order |
| 213–357 game methods | replay, input mapping, gameplay, audio, persistence, rendering | stays |

The game-specific `redraw`, `process_input`, gameplay updates, audio event
mapping, replay handling and persistence remain. A realistic net reduction is
**roughly 80–110 LOC**: the generic event-loop/window/forwarding/scheduling
blocks are removed, while presenter construction/resize, pause policy, input
mapping and game callbacks remain (some of which move into `setup`, `resumed`, `frame_update` and `tick`).

A minimal adoption shape:

```rust
impl Application for MissileApp {
    type Error = MyError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.load_records()?; // once per run
        Ok(())
    }
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        // Menus, overlays and anything that must run even with zero ticks.
        if context.input.action("pause").pressed {
            context.input.consume_action("pause");
            self.session.pause();
        }
        Ok(Flow::Continue)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        if self.session.is_paused() {
            return Ok(Flow::Deferred);
        }
        self.session.tick(context.input, &mut self.audio)?;
        Ok(Flow::Continue)
    }
    fn render(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.renderer.render(...);
        self.presenter.as_mut().unwrap().present(self.renderer.rgba())?;
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.presenter.take(); // before the runner drops the window
        Ok(())
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.save_records()?;
        Ok(())
    }
}
impl DesktopApplication for MissileApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        if self.presenter.is_none() {
            let window = context.window.expect("desktop runner provides a window");
            self.presenter = Some(FramebufferPresenter::new(
                window.shared_native(), context.size.into(), (WIDTH, HEIGHT), options,
            )?);
        }
        Ok(())
    }
}
```

The `AppConfig` supplies the existing MissileCommand window title, application
id, X11 `missile-command`/`MissileCommand` identity and 120 Hz timing. The
window creation, event forwarding, runtime error/teardown ordering and redraw
scheduling can be deleted. This estimate is from line-level audit, not a
compiled migration.

## MicroMages shape

Source ranges:

| Range | Current responsibility | What can move to `quasar-app` |
|---|---|---|
| `main.rs` 632–639 `run_headless` | loops `Session::frame` until done | `run_headless` tick/shutdown loop |
| `main.rs` 641–672 window factory + `run_desktop` | event loop, app construction, failure return | event loop and failure return |
| `main.rs` 674–772 `App` + handler | window factory, session creation, event forwarding, 60 Hz deadline, failure handling, finish | window ownership, forwarding, fixed-step/frame scheduling, ordered shutdown |
| `main.rs` 774–794 `run` | CLI/load + desktop/headless branch | branch can remain; both paths delegate to Quasar |
| `src/timing.rs` 1–78 | 60 Hz `FrameClock`, 250 ms clamp, eight-step limit, pause reset | `FixedStepConfig` + runner scheduling |
| `src/window.rs` 28–41 `NativeWindow` | owns presenter and `GameWindow` | window ownership |
| `src/window.rs` 43–102 `create` | window/desktop identity, presenter creation, resize | window/desktop identity and creation; presenter stays |
| `src/window.rs` 109–119 event forwarding | forwards events | `DesktopApplication` hooks |
| `src/window.rs` 121–150 `poll` | input snapshot, edge consumption, size policy | edge consumption (runner); game-specific key/menu snapshot stays |

The `App`/event-handler/factory blocks, `timing.rs`, and the `NativeWindow`
window ownership/event forwarding can be removed. `AppConfig` represents the
MicroMages 60 Hz step (`FixedStepConfig { hz: 60.0, max_catch_up_ticks: 8,
max_elapsed: Some(250 ms), .. }`), identity and window size. A realistic net
reduction is **roughly 180–230 LOC**, depending on whether `NativeWindow` is kept
as a presenter adapter or folded into the application. This is larger than
MissileCommand because MicroMages has its own frame-deadline clock and a
dedicated window adapter. Both estimates are read-only audit estimates, not
measured patches.

A minimal adoption shape:

```rust
impl Application for MicroMagesApp {
    type Error = MyError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.session.load()?; // once per run
        Ok(())
    }
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        // Menu/deadline handling that must run on frames without a fixed tick.
        self.session.update_ui(context.input)?;
        Ok(Flow::Continue)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.session.tick(context.input)?;
        Ok(Flow::Continue)
    }
    fn render(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.presenter.as_mut().unwrap().present(&self.displayed, scaling)?;
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.presenter.take();
        Ok(())
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.session.finish()?;
        Ok(())
    }
}
impl DesktopApplication for MicroMagesApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        if self.presenter.is_none() {
            self.presenter = Some(FramebufferPresenter::new(
                context.window.expect("window").shared_native(),
                context.size.into(), (WIDTH, HEIGHT), options,
            )?);
        }
        Ok(())
    }
}
```

The `App`/event-handler/factory blocks, `timing.rs`, and the `NativeWindow`
window ownership/event forwarding can be removed. `AppConfig` represents the
MicroMages 60 Hz step (`FixedStepConfig { hz: 60.0, max_catch_up_ticks: 8,
max_elapsed: Some(250 ms), .. }`), identity and window size. A realistic net
reduction is **roughly 200–260 LOC**, depending on whether `NativeWindow` is kept
as a presenter adapter or folded into the application.

## Shared API check

| Requirement | MissileCommand | MicroMages |
|---|---|---|
| Custom X11 identity | `AppConfig.desktop` | `AppConfig.desktop` |
| Window size/title/min size | `AppConfig.window` | `AppConfig.window` |
| Build presenter after window | `resumed` + `context.window.shared_native()` | same |
| Caller-owned VulkanicGAL/presenter | app field | app field |
| Read input without per-game edge consumption | `frame_update`/`tick` read `context.input`; runner clears edges | same |
| Resize handling | `window_event` + `context.size` | same |
| 120 Hz sim / 60 Hz sim | `TimingConfig::Fixed` | `TimingConfig::Fixed` |
| Fixed render or frame deadline | `RenderPolicy`/app render | fixed step + app render |
| Pause/focus reset | `Flow::Deferred` + `focus_changed` | same |
| Headless | `run_headless` with game session | `run_headless` with game session |
| Normal Cargo run | `quasar_app::run` in `main` | same |

No game-specific runner branch is required. The runner never owns GAL, an
audio device, a presenter or game state. It only creates/forwards the window and
calls application hooks; both games retain their existing rendering and input
semantics.

## Limitations of adoption

- The current `quasar-app` runner owns one window. Both audited games have one
  window, so this is not a blocker.
- MicroMages’ custom fullscreen/menu reconciliation still belongs in its
  application hooks; it is game policy, not generic lifecycle.
- The runner clears transient edges after the first fixed tick. Games must move
  immediate input/UI processing into `frame_update` and claim it with the
  explicit `FrameInput::consume_*` methods; gameplay reads `context.input` in
  `tick`. Do not consume edges manually in `about_to_wait`.
- Window-bound graphics are created in `resumed`, not `setup`, because `setup`
  runs only once per run. `suspended` still releases GPU state before the runner
  drops the window.
- Exact LOC reductions are estimates based on the current source ranges and
  will vary with how presenters/window adapters are folded.

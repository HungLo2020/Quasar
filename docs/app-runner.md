# Optional application runner

`quasar-app` composes existing Quasar pieces into one explicit desktop or
headless lifecycle. It owns no ECS, renderer, GAL, audio device, asset runtime or
editor, and nothing else in the workspace requires it. Applications that want
direct control can keep using `quasar-window` and VulkanicGAL without this crate.

The runner exists to make one thing unambiguous: every frame has a fixed
callback order, one input snapshot, one variable-rate update, zero or more fixed
simulation ticks and at most one render pass after the tick batch.

## Callback phases

`Application` is the headless-capable trait:

| Method | When it runs | Owns |
|---|---|---|
| `setup` | Once, after the first window exists (desktop) or before the first headless iteration | Window-independent state, session loading, audio, assets |
| `frame_update` | Exactly once per application frame, before fixed ticks | Variable-rate UI, cameras, networking, input routing, visual state |
| `tick` | Zero or more times per frame | Deterministic simulation |
| `render` | At most once per frame, after all of the frame's ticks | Drawing and presentation |
| `suspended` | Before the window/surface is destroyed or the app is suspended | Releasing window-bound graphics |
| `shutdown` | Once at final teardown, after `suspended` when applicable | Persistence, final metrics, remaining cleanup |

`DesktopApplication` adds:

| Method | When it runs |
|---|---|
| `configure_window` | Immediately after every window creation/recreation, before `setup`/`resumed` |
| `window_event` | After `GameWindow::process`, for every native window event |
| `device_event` | After `GameWindow::process_device`, for every raw device event |
| `resumed` | After every window creation/recreation, after `setup` on the first one |

`setup` runs **once per run**. `resumed` runs on the initial window and again
after a suspend/recreate cycle, which is where window-bound graphics belong:

```rust
fn setup(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
    self.load_session_and_audio()?; // window-independent
    Ok(())
}

impl DesktopApplication for MyApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.create_renderer(context)?; // window/surface-bound
        Ok(())
    }
}
```

A frame is processed in this order:

1. Native events update `GameWindow` input, desktop metadata and focus state.
2. The fixed clock (if any) accepts the frame's elapsed time and reports due
   ticks.
3. One `FrameInput` snapshot is captured from the pending native input.
4. `frame_update` runs exactly once and may consume UI actions or text.
5. The frame's consumption requests are applied to the pending input state.
6. Fixed ticks run, bounded by the clock's catch-up budget. Transient edges are
   cleared after the first successful tick; later ticks in the same frame see
   held state but not repeated presses.
7. `render` runs if a redraw was requested and the window is drawable. It never
   runs in the middle of a tick batch.
8. The runner requests the next redraw according to `RenderPolicy` and waits
   until the next fixed deadline or `max_frame_wait` (also respecting
   `frame_interval`).

`Flow::Exit` from any callback stops the event loop and still runs `shutdown`
once. `Flow::Deferred` from `tick` means "this simulation tick did not run" (for
example, the game is paused): simulation time and tick indices do not advance,
transient input stays pending, and the frame's accumulated clock backlog is
discarded rather than replayed as a catch-up burst. `Flow::Deferred` from
`frame_update` or `render` behaves like `Continue`.

## Input contract

Native transitions accumulate in `quasar_input::InputState`. The runner captures
a `FrameInput` snapshot once per frame; the same snapshot is passed to
`frame_update`, every tick in that frame and `render`.

- Short taps survive frames with zero fixed ticks. A press is not lost just
  because the frame produced no simulation step.
- A press is delivered to the first tick and is not delivered again during
  catch-up. Held and axis values remain visible to later ticks.
- After the first tick, `pressed`/`released` edges, text, relative motion and
  scroll deltas are cleared in the snapshot.
- A tick returning `Flow::Deferred` does not clear anything, so the next tick
  that actually runs receives the pending edges.
- UI consumers run in `frame_update` and can claim input before gameplay:
  `consume_action`, `consume_button`, `consume_pressed`, `consume_keyboard`,
  `consume_pointer`, `consume_text`, `consume_relative_motion`, `consume_scroll`
  or `consume_all`. Consumption is applied to the pending `InputState` after the
  frame callback, so a consumed action does not fire on the next frame either.
  `consume_all` also releases held buttons, which is what a modal menu wants.
- `quasar_ui::from_frame_input` maps a snapshot into a UI event struct.

Game code can accept `impl quasar_input::InputRead` to work with either an
`InputState` (lower-level loops) or a `FrameInput` (runner callbacks). The
action-mapped built-in games (Pong, Tilequest, Starfall) use this for their
control mapping; the direct-key examples read `context.input` instead.

Focus loss releases held actions (emitting release edges through `GameWindow`)
and resets the fixed clock; `focus_changed` reports the transition on the
callbacks for that frame. Applications can therefore pause on focus loss without
leaving movement or fire actions stuck while the window is unfocused.

## Timing

`TimingConfig` selects the scheduling model:

- `Fixed(FixedStepConfig)` runs `tick` zero or more times per frame from a bounded
  accumulator. `FixedStepConfig` controls:
  - `hz`: exact-ish fractional simulation rate;
  - `max_catch_up_ticks`: hard per-frame tick bound;
  - `max_elapsed`: optional clamp applied before the accumulator (suspend spikes);
  - `backlog`: `Drop` discards time beyond the catch-up cap, `Preserve` keeps it
    for later frames.
- `Variable` never calls `tick`; `frame_update` owns all scheduling. This is for
  genuinely frame-driven applications. A game that needs both should use
  `Fixed` and put variable work in `frame_update`.

`TimingInfo` reports `frame_elapsed`, `fixed_step`, `interpolation`,
`frame_index`, `tick_index`, `ticks_this_frame`, `simulation_time`,
`accepted_elapsed`, `clamped_elapsed`, `dropped_elapsed` and `backlog`. Frame and
tick indices are 1-based and remain monotonic across a suspend/recreate cycle;
elapsed time, the accumulator and `simulation_time`/indices restart only when the
application is re-created (a new run). Interpolation is the fraction of a step
before the next tick, which is what renderers need for presentation smoothing.

`RenderPolicy` selects redraw requests: `EveryFrame` (including frames with zero
ticks), `AfterTick`, or `Manual`. With `Manual`, the application calls
`context.request_redraw()` when it wants to render. A window that is minimized,
zero-sized or otherwise not drawable suppresses rendering; `context.drawable()`
reports that state.

`AppConfig` combines the window/desktop configuration, timing, render policy,
`max_frame_wait` (upper bound on one wait, keeps the loop responsive) and an
optional `frame_interval` (minimum wall-clock spacing between application
frames). Invalid timing, a zero wait or a zero frame interval is rejected before
the event loop is created.

## Focus, suspend and lifecycle

- `focus_changed` is `Some(true/false)` on callbacks associated with a focus
  transition. The fixed clock is reset on focus loss so background time is never
  simulated.
- On platform suspend, `suspended` runs before the runner drops the window.
  Graphics state is released there. The runner clears input, resets the clock and
  the frame timer.
- On resume, the runner recreates the window, applies `configure_window`, runs
  `resumed` and continues with fresh input/timing state. `setup` and `shutdown`
  are not repeated.
- Final teardown calls `shutdown` once, drops the window after it, and combines a
  primary callback/event-loop error with a shutdown error instead of hiding the
  first failure.
- A setup failure still runs `shutdown` and returns the original error.

## Headless execution

`run_headless(application, HeadlessConfig { ticks, step })` drives the same
application without a window, winit or rendering:

- one `frame_update` and one attempted `tick` per iteration, with an empty input
  snapshot;
- `setup` before the first iteration and `shutdown` exactly once afterwards;
- a deferred tick does not advance simulation time or tick indices, and the
  iteration is still consumed;
- `Flow::Exit` stops early;
- errors from setup/frame/tick are returned with a shutdown error appended if
  cleanup also fails;
- a zero `step` is rejected before any callback.

Use it for deterministic replay, CI simulation and performance work. For native
visual validation use a windowed `--frames`/`--exercise-window` path.

## Ownership and lower-level exceptions

Applications own their VulkanicGAL device, renderer, presenter, audio and assets.
The runner only owns the window and scheduling. The migrated built-in examples
are:

| Example | Timing | Notes |
|---|---|---|
| Pong | 120 Hz fixed | Runner owns the window; game owns the 2D presenter |
| Tilequest | Fixed | Caller-owned GAL, frame surface, map traversal and screenshots |
| Starfall | Fixed | 3D+2D frame, assets, audio, UI menus, replay, profiling |
| Falling Sand | Fixed | Cellular update in `tick`, menu/UI in `frame_update` |
| Physics Playground | Fixed | First-person input, Rapier physics and renderer |
| `app-stress` | Fixed | Internal integration harness: jobs, async assets, Deferred pause, input consumption, resize/fullscreen exercise |

`examples/framebuffer` and `examples/composition` intentionally keep their direct
`winit` + `quasar-window` loops. They are lower-level graphics examples: one
exercises `FramebufferPresenter`/raw allocation accounting and the other explicit
single-GAL 3D+2D composition submission. The runner would only add a layer between
those APIs and the example without exercising anything they test. They remain the
documented escape hatch for applications that need their own event loop.

## Configuration reference

```rust
AppConfig {
    window: WindowConfig { /* title, application_id, size, fullscreen, ... */ },
    desktop: DesktopConfig::for_application("my-app"),
    timing: TimingConfig::Fixed(FixedStepConfig {
        hz: 120.0,
        max_catch_up_ticks: 8,
        max_elapsed: Some(Duration::from_millis(250)),
        backlog: BacklogPolicy::Drop,
    }),
    render: RenderPolicy::EveryFrame,
    max_frame_wait: Duration::from_millis(16),
    frame_interval: None,
}
```

## Local measurements

Release build, Linux/X11 on an RTX 2070 host, `app-stress` (one variable
`frame_update`, fixed 60 Hz `tick`, input snapshot, job/channel poll per frame):

| Mode | Work | Wall time | Notes |
|---|---|---|---|
| Headless | 1,000,000 frames / 499,999 ticks | 0.86 s | ~0.86 µs per frame incl. one fixed tick; 8.7 MB RSS |
| Native (Xvfb) | 120 frames / 58 ticks | 1.48 s | includes Vulkan device/surface creation and teardown; frame-to-tick ratio follows presentation cadence |

These are single-host observations for scheduling overhead, not comparative
benchmarks against a hand-written loop. `frame_update` intentionally runs at the
display/event cadence while `tick` stays at the configured simulation rate, so
the frame-to-tick ratio depends on presentation rate.

## Verification

```sh
cargo test -p quasar-app
cargo test -p quasar-app --no-default-features
cargo clippy -p quasar-app --all-targets -- -D warnings
cargo run -p quasar-app --example desktop
cargo run -p quasar-app --example desktop -- --headless
cargo run -p app-stress -- --headless --ticks 120
cargo run -p app-stress -- --frames 40 --exercise-window
cargo test -p quasar-input --lib
cargo test -p quasar-core --lib
```

Tests cover frame/tick ordering, zero/one/many ticks, catch-up bounds,
`Flow::Deferred` pausing without replaying backlog, variable timing, short taps
across zero-tick frames, consumption isolation, setup/shutdown deduplication and
combined errors. Native runs verify window creation, resize, fullscreen,
zero-size deferral, rendering and ordered GPU teardown under X11/Xvfb.

## Migrating from v0.2.11

See [the v0.2.11 migration guide](app-migration-v0.2.11.md) for the old
`tick`/`window.input` style and the mechanical changes to adopt `frame_update`
and `FrameInput`.

## Limitations

- One window per runner. Multiple windows require lower-level winit code.
- No automatic GPU device-loss recovery, audio ownership or asset reloading.
- No built-in pause/menu policy: use `Flow::Deferred`, `focus_changed` and
  explicit input consumption.
- Render scheduling is intentionally at frame/tick granularity; offscreen or
  multi-pass schedules belong in `render`.
- Desktop behavior is verified on Linux/X11. Windows/macOS are compile-checked by
  CI but not hardware-verified in this workspace.
- Automated native runs cover window creation, resize, fullscreen, zero-size
  recovery, rendering and GPU teardown. Platform suspend/resume and real focus
  transitions need a window manager/session and are implemented but not
  hardware-automated here; the input and scheduling pieces behind them are
  unit-tested.
- The runner adds one event-loop dispatch layer and `FixedStep` bookkeeping; it
  makes no comparative throughput claim against a hand-written loop.

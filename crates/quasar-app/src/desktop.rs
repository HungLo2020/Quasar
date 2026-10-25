use crate::{
    AppContext, AppPhase, DesktopApplication, Flow, RenderPolicy, TimingConfig, TimingInfo,
};
use quasar_core::FixedStep;
use quasar_input::FrameInput;
use quasar_window::{
    DesktopConfig, GameWindow, WindowConfig,
    winit::{
        application::ApplicationHandler,
        event::{DeviceEvent, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::WindowId,
    },
};
use std::{
    error::Error,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub window: WindowConfig,
    pub desktop: DesktopConfig,
    pub timing: TimingConfig,
    pub render: RenderPolicy,
    /// Upper bound on a single wait; keeps the loop responsive to ticks and
    /// native events.
    pub max_frame_wait: Duration,
    /// Optional minimum interval between application frames. This paces frame
    /// updates without changing the fixed simulation rate. `None` lets winit
    /// and VSync determine cadence.
    pub frame_interval: Option<Duration>,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self::for_window(WindowConfig {
            title: "Quasar App".into(),
            application_id: "quasar-app".into(),
            ..WindowConfig::default()
        })
    }
}
impl AppConfig {
    pub fn for_window(window: WindowConfig) -> Self {
        let desktop = DesktopConfig::for_application(window.application_id.clone());
        Self {
            window,
            desktop,
            timing: TimingConfig::default(),
            render: RenderPolicy::default(),
            max_frame_wait: Duration::from_millis(16),
            frame_interval: None,
        }
    }
}

#[derive(Debug)]
enum Clock {
    Fixed(FixedStep),
    Variable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lifecycle {
    Uninitialized,
    SettingUp,
    Ready,
    Suspended,
    Failed,
    Shutdown,
}

/// Runs a desktop application using an ordinary winit event loop.
///
/// The runner owns one `GameWindow`; applications own their GAL, renderer,
/// audio and asset state. Native events are forwarded into `GameWindow` input,
/// one `FrameInput` snapshot is captured per application frame, and fixed ticks
/// consume the snapshot's transient edges exactly once.
pub fn run<A: DesktopApplication>(application: A, config: AppConfig) -> Result<(), Box<dyn Error>> {
    if config.max_frame_wait.is_zero() {
        return Err("max_frame_wait must be nonzero".into());
    }
    if config
        .frame_interval
        .is_some_and(|interval| interval.is_zero())
    {
        return Err("frame_interval must be nonzero".into());
    }
    let clock = match config.timing {
        TimingConfig::Fixed(timing) => Clock::Fixed(FixedStep::with_config(timing).map_err(
            |error| -> Box<dyn Error> { format!("invalid application timing: {error}").into() },
        )?),
        TimingConfig::Variable => Clock::Variable,
    };
    let event_loop = EventLoop::new()?;
    let mut runner = Runner {
        application,
        config,
        clock,
        window: None,
        frame_input: FrameInput::default(),
        last_frame: None,
        last_elapsed: Duration::ZERO,
        frame: 0,
        tick: 0,
        simulated_time: Duration::ZERO,
        focus_changed: None,
        redraw_pending: false,
        setup_attempted: false,
        lifecycle: Lifecycle::Uninitialized,
        error: None,
    };
    let event_loop_result = event_loop.run_app(&mut runner);
    let runner_error = runner.error.take();
    match (event_loop_result, runner_error) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(error)) => Err(error),
        (Err(event_loop_error), None) => Err(Box::new(event_loop_error)),
        (Err(event_loop_error), Some(primary)) => Err(Box::new(CombinedError {
            primary,
            cleanup: Box::new(event_loop_error),
        })),
    }
}

struct Runner<A> {
    application: A,
    config: AppConfig,
    clock: Clock,
    window: Option<GameWindow>,
    frame_input: FrameInput,
    last_frame: Option<Instant>,
    last_elapsed: Duration,
    frame: u64,
    tick: u64,
    simulated_time: Duration,
    focus_changed: Option<bool>,
    redraw_pending: bool,
    setup_attempted: bool,
    lifecycle: Lifecycle,
    error: Option<Box<dyn Error>>,
}

impl<A: DesktopApplication> Runner<A> {
    fn fixed_step(&self) -> Duration {
        match &self.clock {
            Clock::Fixed(clock) => clock.step(),
            Clock::Variable => Duration::ZERO,
        }
    }

    fn interpolation(&self) -> f64 {
        match &self.clock {
            Clock::Fixed(clock) => clock.interpolation(),
            Clock::Variable => 0.0,
        }
    }

    fn backlog(&self) -> Duration {
        match &self.clock {
            Clock::Fixed(clock) => clock.backlog(),
            Clock::Variable => Duration::ZERO,
        }
    }

    fn timing(
        &self,
        ticks_this_frame: u32,
        report: Option<quasar_core::AdvanceReport>,
    ) -> TimingInfo {
        TimingInfo {
            frame_elapsed: self.last_elapsed,
            fixed_step: self.fixed_step(),
            interpolation: self.interpolation(),
            frame_index: self.frame,
            tick_index: self.tick,
            ticks_this_frame,
            simulation_time: self.simulated_time,
            accepted_elapsed: report.map_or(self.last_elapsed, |report| report.accepted),
            clamped_elapsed: report.map_or(Duration::ZERO, |report| report.clamped),
            dropped_elapsed: report.map_or(Duration::ZERO, |report| report.dropped),
            backlog: self.backlog(),
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<Box<dyn Error>>) {
        if self.error.is_none() {
            self.error = Some(error.into());
        }
        if self.lifecycle != Lifecycle::Shutdown {
            self.lifecycle = Lifecycle::Failed;
        }
        event_loop.exit();
    }

    fn fail_application(&mut self, event_loop: &ActiveEventLoop, error: impl Error + 'static) {
        self.fail(event_loop, Box::new(error));
    }
}

/// Advances the clock for one application frame and reports the bounded number
/// of fixed ticks that are due.
fn plan_frame(clock: &mut Clock, elapsed: Duration) -> (Option<quasar_core::AdvanceReport>, u32) {
    match clock {
        Clock::Fixed(clock) => {
            let report = clock.advance_with_report(elapsed);
            (Some(report), clock.pending_ticks())
        }
        Clock::Variable => (None, 0),
    }
}

/// Whether the fixed clock still has a tick due within this frame's budget.
fn tick_due(clock: &Clock) -> bool {
    match clock {
        Clock::Fixed(clock) => clock.pending_ticks() > 0,
        Clock::Variable => false,
    }
}

/// Commits one successful fixed tick to the accumulator.
fn complete_tick(clock: &mut Clock) {
    if let Clock::Fixed(clock) = clock {
        clock.take_tick();
    }
}

/// Handles a deferred tick by pausing the fixed schedule rather than replaying
/// the frame's backlog as a catch-up burst once the simulation resumes.
fn defer_tick(clock: &mut Clock) {
    if let Clock::Fixed(clock) = clock {
        clock.reset();
    }
}

/// Wait until the next tick/deadline, bounded by `max_frame_wait`, and padded to
/// the configured minimum frame interval.
fn next_wait(clock: &Clock, last_elapsed: Duration, config: &AppConfig) -> Duration {
    let wait = match clock {
        Clock::Fixed(clock) => clock.until_next_tick().min(config.max_frame_wait),
        Clock::Variable => config.max_frame_wait,
    };
    config.frame_interval.map_or(wait, |interval| {
        wait.max(interval.saturating_sub(last_elapsed))
    })
}

impl<A: DesktopApplication> ApplicationHandler for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut window = match GameWindow::create_with_desktop(
            event_loop,
            &self.config.window,
            &self.config.desktop,
        ) {
            Ok(window) => window,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        if let Err(error) = self.application.configure_window(&mut window) {
            self.fail_application(event_loop, error);
            return;
        }
        let first_window = !self.setup_attempted;
        self.window = Some(window);
        self.lifecycle = Lifecycle::SettingUp;
        self.frame_input
            .capture(&self.window.as_ref().unwrap().input, 0);
        let timing = self.timing(0, None);
        if first_window {
            self.setup_attempted = true;
            let setup_result = {
                let mut context = make_context(
                    self.window.as_ref(),
                    &mut self.frame_input,
                    timing,
                    AppPhase::Frame,
                    self.focus_changed,
                );
                self.application.setup(&mut context)
            };
            if let Err(error) = setup_result {
                self.fail_application(event_loop, error);
                return;
            }
        }
        let resumed_result = {
            let mut context = make_context(
                self.window.as_ref(),
                &mut self.frame_input,
                timing,
                AppPhase::Frame,
                self.focus_changed,
            );
            self.application.resumed(&mut context)
        };
        if let Err(error) = resumed_result {
            self.fail_application(event_loop, error);
            return;
        }
        if let Clock::Fixed(clock) = &mut self.clock {
            clock.reset();
        }
        self.last_frame = Some(Instant::now());
        self.last_elapsed = Duration::ZERO;
        self.focus_changed = None;
        self.redraw_pending = false;
        self.lifecycle = Lifecycle::Ready;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let update = {
            let Some(window) = &mut self.window else {
                return;
            };
            if window.native().id() != id {
                return;
            }
            window.process(&event)
        };
        if let Some(focused) = update.focus_changed {
            self.focus_changed = Some(focused);
            if !focused {
                // `GameWindow` releases held input on focus loss; the runner
                // also discards accumulated time so background time is not
                // simulated.
                if let Clock::Fixed(clock) = &mut self.clock {
                    clock.reset();
                }
            }
        }
        if update.redraw_requested {
            self.redraw_pending = true;
        }
        let timing = self.timing(0, None);
        let hook_result = {
            let mut context = make_context(
                self.window.as_ref(),
                &mut self.frame_input,
                timing,
                AppPhase::Frame,
                self.focus_changed,
            );
            self.application.window_event(&mut context, &event)
        };
        if let Err(error) = hook_result {
            self.fail_application(event_loop, error);
            return;
        }
        if update.close_requested {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _: &ActiveEventLoop,
        _: quasar_window::winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        let Some(window) = &mut self.window else {
            return;
        };
        window.process_device(&event);
        let timing = self.timing(0, None);
        let hook_result = {
            let mut context = make_context(
                self.window.as_ref(),
                &mut self.frame_input,
                timing,
                AppPhase::Frame,
                self.focus_changed,
            );
            self.application.device_event(&mut context, &event)
        };
        if let Err(error) = hook_result {
            self.error = Some(Box::new(error));
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            event_loop.exit();
            return;
        }
        let Some(window) = &mut self.window else {
            return;
        };
        window.sync_desktop();
        if let Some(error) = window.desktop_error().map(str::to_owned) {
            self.fail(event_loop, std::io::Error::other(error));
            return;
        }

        let now = Instant::now();
        self.last_elapsed = self
            .last_frame
            .map_or(Duration::ZERO, |last| now.duration_since(last));
        self.last_frame = Some(now);
        self.frame = self.frame.wrapping_add(1);

        let (report, ticks_this_frame) = plan_frame(&mut self.clock, self.last_elapsed);

        self.frame_input.capture(&window.input, self.frame);
        // UI/frame consumers may consume actions before fixed simulation.
        let frame_timing = self.timing(ticks_this_frame, report);
        let frame_flow = {
            let mut context = make_context(
                self.window.as_ref(),
                &mut self.frame_input,
                frame_timing,
                AppPhase::Frame,
                self.focus_changed,
            );
            self.application.frame_update(&mut context)
        };
        self.frame_input
            .apply_consumption(&mut self.window.as_mut().unwrap().input);
        match frame_flow {
            Ok(Flow::Continue | Flow::Deferred) => {}
            Ok(Flow::Exit) => {
                event_loop.exit();
                return;
            }
            Err(error) => {
                self.fail_application(event_loop, error);
                return;
            }
        }
        let mut ticks_run = 0u32;
        let mut tick_deferred = false;
        loop {
            if ticks_run >= ticks_this_frame {
                break;
            }
            if !tick_due(&self.clock) {
                break;
            }
            let next_tick = self.tick.wrapping_add(1);
            let tick_timing = TimingInfo {
                tick_index: next_tick,
                interpolation: self.interpolation(),
                simulation_time: self.simulated_time,
                ..frame_timing
            };
            let tick_flow = {
                let mut context = make_context(
                    self.window.as_ref(),
                    &mut self.frame_input,
                    tick_timing,
                    AppPhase::Tick,
                    self.focus_changed,
                );
                self.application.tick(&mut context)
            };
            match tick_flow {
                Ok(Flow::Continue) => {
                    complete_tick(&mut self.clock);
                    self.tick = next_tick;
                    ticks_run += 1;
                    if ticks_run == 1 {
                        self.frame_input
                            .apply_consumption(&mut self.window.as_mut().unwrap().input);
                        self.frame_input.clear_transient();
                        self.window.as_mut().unwrap().input.consume_edges();
                    }
                    self.simulated_time = self.simulated_time.saturating_add(self.fixed_step());
                }
                Ok(Flow::Deferred) => {
                    // A deferred tick is a deliberate pause: keep transient
                    // edges pending, do not advance time, and discard the
                    // frame's accumulated backlog instead of replaying it as a
                    // catch-up burst after the simulation resumes.
                    defer_tick(&mut self.clock);
                    tick_deferred = true;
                    break;
                }
                Ok(Flow::Exit) => {
                    event_loop.exit();
                    return;
                }
                Err(error) => {
                    self.fail_application(event_loop, error);
                    return;
                }
            }
        }
        if ticks_run == 0 || tick_deferred {
            // Keep edges pending until a tick or an explicit consumer handles
            // them on a later frame.
            self.frame_input
                .apply_consumption(&mut self.window.as_mut().unwrap().input);
        }

        if self.redraw_pending {
            self.redraw_pending = false;
            if self.window.as_ref().is_some_and(GameWindow::drawable) {
                let render_timing = self.timing(ticks_run, report);
                let render_result = {
                    let mut context = make_context(
                        self.window.as_ref(),
                        &mut self.frame_input,
                        render_timing,
                        AppPhase::Render,
                        self.focus_changed,
                    );
                    self.application.render(&mut context)
                };
                match render_result {
                    Ok(Flow::Continue | Flow::Deferred) => {}
                    Ok(Flow::Exit) => {
                        event_loop.exit();
                        return;
                    }
                    Err(error) => {
                        self.fail_application(event_loop, error);
                        return;
                    }
                }
            }
        }

        let request_redraw = match self.config.render {
            RenderPolicy::EveryFrame => true,
            RenderPolicy::AfterTick => ticks_run > 0,
            RenderPolicy::Manual => false,
        };
        if request_redraw && self.window.as_ref().is_some_and(GameWindow::drawable) {
            self.window.as_ref().unwrap().native().request_redraw();
        }
        self.focus_changed = None;

        let wait = next_wait(&self.clock, self.last_elapsed, &self.config);
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + wait));
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }
        let timing = self.timing(0, None);
        let suspend_result = {
            let mut context = make_context(
                self.window.as_ref(),
                &mut self.frame_input,
                timing,
                AppPhase::Suspended,
                self.focus_changed,
            );
            self.application.suspended(&mut context)
        };
        if let Err(error) = suspend_result {
            self.error = Some(Box::new(error));
            event_loop.exit();
        }
        self.window.take();
        self.frame_input = FrameInput::default();
        if let Clock::Fixed(clock) = &mut self.clock {
            clock.reset();
        }
        self.last_frame = None;
        self.last_elapsed = Duration::ZERO;
        self.redraw_pending = false;
        self.lifecycle = Lifecycle::Suspended;
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if self.setup_attempted && self.lifecycle != Lifecycle::Shutdown {
            let timing = self.timing(0, None);
            let shutdown_result = {
                let mut context = make_context(
                    self.window.as_ref(),
                    &mut self.frame_input,
                    timing,
                    AppPhase::Shutdown,
                    self.focus_changed,
                );
                self.application.shutdown(&mut context)
            };
            if let Err(error) = shutdown_result {
                if let Some(primary) = self.error.take() {
                    self.error = Some(Box::new(CombinedError {
                        primary,
                        cleanup: Box::new(error),
                    }));
                } else {
                    self.error = Some(Box::new(error));
                }
            }
        }
        self.lifecycle = Lifecycle::Shutdown;
        self.window.take();
    }
}

#[derive(Debug)]
struct CombinedError {
    primary: Box<dyn Error>,
    cleanup: Box<dyn Error>,
}
impl std::fmt::Display for CombinedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}; cleanup also failed: {}",
            self.primary, self.cleanup
        )
    }
}
impl Error for CombinedError {}

fn make_context<'a>(
    window: Option<&'a GameWindow>,
    input: &'a mut FrameInput,
    timing: TimingInfo,
    phase: AppPhase,
    focus_changed: Option<bool>,
) -> AppContext<'a> {
    let (size, scale_factor, focused) = window.map_or(((0, 0), 1.0, false), |window| {
        let size = window.native().inner_size();
        (
            (size.width, size.height),
            window.native().scale_factor(),
            window.focused(),
        )
    });
    AppContext {
        window,
        input,
        size,
        scale_factor,
        elapsed: timing.frame_elapsed,
        frame: timing.frame_index,
        tick: timing.tick_index,
        interpolation: timing.interpolation,
        timing,
        phase,
        focused,
        focus_changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Application;
    use quasar_core::FixedStepConfig;

    struct Noop;
    impl Application for Noop {
        type Error = std::convert::Infallible;
    }
    impl DesktopApplication for Noop {}

    #[test]
    fn invalid_timing_is_rejected_before_an_event_loop_is_created() {
        let error = run(
            Noop,
            AppConfig {
                timing: TimingConfig::Fixed(FixedStepConfig {
                    hz: 0.0,
                    ..FixedStepConfig::default()
                }),
                ..AppConfig::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid application timing"));
    }

    #[test]
    fn zero_frame_wait_is_rejected() {
        let error = run(
            Noop,
            AppConfig {
                max_frame_wait: Duration::ZERO,
                ..AppConfig::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("max_frame_wait"));
    }

    fn fixed(hz: f64, max: u32) -> Clock {
        Clock::Fixed(FixedStep::with_config(FixedStepConfig::for_rate(hz, max)).unwrap())
    }

    #[test]
    fn a_short_frame_schedules_zero_ticks() {
        let mut clock = fixed(60.0, 8);
        let step = match &clock {
            Clock::Fixed(clock) => clock.step(),
            Clock::Variable => unreachable!(),
        };
        let (report, due) = plan_frame(&mut clock, step / 4);
        assert_eq!(due, 0);
        assert!(!tick_due(&clock));
        assert_eq!(report.unwrap().backlog, step / 4);
    }

    #[test]
    fn one_and_many_ticks_are_bounded_by_the_catch_up_limit() {
        let mut clock = fixed(60.0, 8);
        let step = match &clock {
            Clock::Fixed(clock) => clock.step(),
            Clock::Variable => unreachable!(),
        };
        let (_, due) = plan_frame(&mut clock, step);
        assert_eq!(due, 1);
        assert!(tick_due(&clock));
        complete_tick(&mut clock);
        assert!(!tick_due(&clock));

        let mut clock = fixed(60.0, 8);
        let (_, due) = plan_frame(&mut clock, step * 3 + step / 2);
        assert_eq!(due, 3);
        for _ in 0..3 {
            assert!(tick_due(&clock));
            complete_tick(&mut clock);
        }
        assert!(!tick_due(&clock));
        assert!(
            (match &clock {
                Clock::Fixed(clock) => clock.interpolation(),
                Clock::Variable => unreachable!(),
            } - 0.5)
                .abs()
                < 1e-6
        );

        let mut clock = fixed(60.0, 2);
        let (report, due) = plan_frame(&mut clock, step * 10);
        assert_eq!(due, 2);
        assert!(report.unwrap().dropped > Duration::ZERO);
    }

    #[test]
    fn deferred_tick_pauses_the_schedule_without_replaying_backlog() {
        let mut clock = fixed(60.0, 8);
        let step = match &clock {
            Clock::Fixed(clock) => clock.step(),
            Clock::Variable => unreachable!(),
        };
        let (_, due) = plan_frame(&mut clock, step * 5);
        assert_eq!(due, 5);
        complete_tick(&mut clock);
        defer_tick(&mut clock);
        assert!(!tick_due(&clock));
        assert_eq!(
            match &clock {
                Clock::Fixed(clock) => clock.backlog(),
                Clock::Variable => unreachable!(),
            },
            Duration::ZERO
        );
        // The next frame starts from a clean accumulator: at most one step of
        // newly elapsed time can be simulated, never the paused backlog.
        let (_, due) = plan_frame(&mut clock, step);
        assert_eq!(due, 1);
        complete_tick(&mut clock);
        assert!(!tick_due(&clock));
    }

    #[test]
    fn frame_interval_pads_and_max_frame_wait_bounds_the_next_wait() {
        let framed = AppConfig {
            max_frame_wait: Duration::from_millis(100),
            frame_interval: Some(Duration::from_millis(40)),
            ..AppConfig::default()
        };
        let clock = fixed(60.0, 8);
        let step = match &clock {
            Clock::Fixed(clock) => clock.step(),
            Clock::Variable => unreachable!(),
        };
        // A fresh accumulator would wait one step (16.6 ms), but the minimum
        // frame interval pads it to 35 ms.
        assert_eq!(
            next_wait(&clock, Duration::from_millis(5), &framed),
            Duration::from_millis(35)
        );
        // Elapsed time already exceeds the interval: do not add a negative delay.
        assert_eq!(next_wait(&clock, Duration::from_millis(45), &framed), step);
        let capped = AppConfig {
            max_frame_wait: Duration::from_millis(16),
            ..AppConfig::default()
        };
        assert_eq!(
            next_wait(&clock, Duration::ZERO, &capped),
            Duration::from_millis(16)
        );
        let variable = AppConfig {
            max_frame_wait: Duration::from_millis(4),
            ..AppConfig::default()
        };
        assert_eq!(
            next_wait(&Clock::Variable, Duration::ZERO, &variable),
            Duration::from_millis(4)
        );
    }

    #[test]
    fn variable_timing_never_plans_fixed_ticks() {
        let mut clock = Clock::Variable;
        let (report, due) = plan_frame(&mut clock, Duration::from_secs(5));
        assert!(report.is_none());
        assert_eq!(due, 0);
        assert!(!tick_due(&clock));
        complete_tick(&mut clock);
        defer_tick(&mut clock);
        assert!(!tick_due(&clock));
    }

    #[test]
    fn zero_frame_interval_is_rejected() {
        let error = run(
            Noop,
            AppConfig {
                frame_interval: Some(Duration::ZERO),
                ..AppConfig::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("frame_interval"));
    }
}

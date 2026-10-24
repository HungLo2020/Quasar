use crate::{AppContext, DesktopApplication, Flow, RenderPolicy, TimingConfig};
use quasar_core::FixedStep;
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
        }
    }
}

enum Clock {
    Fixed(FixedStep),
    EveryFrame,
}

/// Runs a desktop application using an ordinary winit event loop.
///
/// The runner owns one `GameWindow`; applications own their GAL, renderer,
/// audio and asset state. It forwards input to `GameWindow`, schedules fixed or
/// frame-driven ticks, and tears down the window after `suspended`/`shutdown`
/// callbacks so graphics resources can be released first.
pub fn run<A: DesktopApplication>(application: A, config: AppConfig) -> Result<(), Box<dyn Error>> {
    let clock = match config.timing {
        TimingConfig::Fixed(timing) => Clock::Fixed(FixedStep::with_config(timing).map_err(
            |error| -> Box<dyn Error> { format!("invalid application timing: {error}").into() },
        )?),
        TimingConfig::EveryFrame => Clock::EveryFrame,
    };
    if config.max_frame_wait.is_zero() {
        return Err("max_frame_wait must be nonzero".into());
    }
    let event_loop = EventLoop::new()?;
    let mut runner = Runner {
        application,
        config,
        clock,
        window: None,
        last_frame: None,
        last_elapsed: Duration::ZERO,
        frame: 0,
        tick: 0,
        started: false,
        shutdown_done: false,
        error: None,
    };
    event_loop.run_app(&mut runner)?;
    match runner.error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Runner<A> {
    application: A,
    config: AppConfig,
    clock: Clock,
    window: Option<GameWindow>,
    last_frame: Option<Instant>,
    last_elapsed: Duration,
    frame: u64,
    tick: u64,
    started: bool,
    shutdown_done: bool,
    error: Option<Box<dyn Error>>,
}
impl<A: DesktopApplication> Runner<A> {
    fn interpolation(&self) -> f64 {
        match &self.clock {
            Clock::Fixed(clock) => clock.interpolation(),
            Clock::EveryFrame => 0.0,
        }
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<Box<dyn Error>>) {
        self.error = Some(error.into());
        event_loop.exit();
    }
    fn fail_application(&mut self, event_loop: &ActiveEventLoop, error: impl Error + 'static) {
        self.fail(event_loop, Box::new(error));
    }
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
        let mut context = AppContext {
            window: Some(&window),
            size: {
                let size = window.native().inner_size();
                (size.width, size.height)
            },
            scale_factor: window.native().scale_factor(),
            elapsed: Duration::ZERO,
            frame: 0,
            tick: 0,
            interpolation: 0.0,
            focused: window.focused(),
        };
        self.started = true;
        if let Err(error) = self.application.setup(&mut context) {
            self.fail_application(event_loop, error);
            return;
        }
        if let Err(error) = self.application.resumed(&mut context) {
            self.fail_application(event_loop, error);
            return;
        }
        self.window = Some(window);
        if let Clock::Fixed(clock) = &mut self.clock {
            clock.reset();
        }
        // Frame/tick counters stay monotonic across suspend/resume; only the
        // elapsed clock and accumulator are reset so a suspended interval is
        // never simulated.
        self.last_frame = Some(Instant::now());
        self.last_elapsed = Duration::ZERO;
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
        if let Err(error) = self.application.window_event(
            &mut context(
                self.window.as_ref(),
                self.last_elapsed,
                self.frame,
                self.tick,
                self.interpolation(),
            ),
            &event,
        ) {
            self.error = Some(Box::new(error));
            event_loop.exit();
            return;
        }
        if update.close_requested {
            event_loop.exit();
            return;
        }
        if matches!(event, WindowEvent::RedrawRequested)
            && let Err(error) = self.application.render(&mut context(
                self.window.as_ref(),
                self.last_elapsed,
                self.frame,
                self.tick,
                self.interpolation(),
            ))
        {
            self.error = Some(Box::new(error));
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _: &ActiveEventLoop,
        _: quasar_window::winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        {
            let Some(window) = &mut self.window else {
                return;
            };
            window.process_device(&event);
        }
        if let Err(error) = self.application.device_event(
            &mut context(
                self.window.as_ref(),
                self.last_elapsed,
                self.frame,
                self.tick,
                self.interpolation(),
            ),
            &event,
        ) {
            self.error = Some(Box::new(error));
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            event_loop.exit();
            return;
        }
        {
            let Some(window) = &mut self.window else {
                return;
            };
            window.sync_desktop();
        }
        if let Some(error) = self
            .window
            .as_ref()
            .and_then(GameWindow::desktop_error)
            .map(str::to_owned)
        {
            self.error = Some(std::io::Error::other(error).into());
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        self.last_elapsed = self
            .last_frame
            .map_or(Duration::ZERO, |last| now.duration_since(last));
        self.last_frame = Some(now);
        self.frame = self.frame.wrapping_add(1);

        let mut ticks = 0u32;
        let mut exit = false;
        match &mut self.clock {
            Clock::Fixed(clock) => {
                clock.advance(self.last_elapsed);
                while clock.take_tick() {
                    ticks += 1;
                    self.tick = self.tick.wrapping_add(1);
                    let interpolation = clock.interpolation();
                    let flow = {
                        let mut context = context(
                            self.window.as_ref(),
                            self.last_elapsed,
                            self.frame,
                            self.tick,
                            interpolation,
                        );
                        self.application.tick(&mut context)
                    };
                    if let Some(window) = &mut self.window {
                        window.input.consume_edges();
                    }
                    match flow {
                        Ok(Flow::Continue) => {}
                        Ok(Flow::Exit) => {
                            exit = true;
                            break;
                        }
                        Err(error) => {
                            self.error = Some(Box::new(error));
                            event_loop.exit();
                            return;
                        }
                    }
                }
            }
            Clock::EveryFrame => {
                ticks = 1;
                self.tick = self.tick.wrapping_add(1);
                let flow = {
                    let mut context = context(
                        self.window.as_ref(),
                        self.last_elapsed,
                        self.frame,
                        self.tick,
                        0.0,
                    );
                    self.application.tick(&mut context)
                };
                if let Some(window) = &mut self.window {
                    window.input.consume_edges();
                }
                match flow {
                    Ok(Flow::Continue) => {}
                    Ok(Flow::Exit) => exit = true,
                    Err(error) => {
                        self.error = Some(Box::new(error));
                        event_loop.exit();
                        return;
                    }
                }
            }
        }
        if exit {
            event_loop.exit();
            return;
        }
        let request_redraw = match self.config.render {
            RenderPolicy::EveryFrame => true,
            RenderPolicy::AfterTick => ticks > 0,
            RenderPolicy::Manual => false,
        };
        if request_redraw && self.window.as_ref().is_some_and(GameWindow::drawable) {
            self.window
                .as_ref()
                .expect("window exists")
                .native()
                .request_redraw();
        }
        let wait = match &self.clock {
            Clock::Fixed(clock) => clock.until_next_tick().min(self.config.max_frame_wait),
            Clock::EveryFrame => self.config.max_frame_wait,
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + wait));
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }
        let mut context = context(
            self.window.as_ref(),
            self.last_elapsed,
            self.frame,
            self.tick,
            self.interpolation(),
        );
        if let Err(error) = self.application.suspended(&mut context) {
            self.error = Some(Box::new(error));
        }
        self.window.take();
        if let Clock::Fixed(clock) = &mut self.clock {
            clock.reset();
        }
        self.last_frame = None;
        self.last_elapsed = Duration::ZERO;
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if self.started && !self.shutdown_done {
            let mut context = context(
                self.window.as_ref(),
                self.last_elapsed,
                self.frame,
                self.tick,
                self.interpolation(),
            );
            if let Err(error) = self.application.shutdown(&mut context) {
                self.error = Some(Box::new(error));
            }
            self.shutdown_done = true;
        }
        self.window.take();
    }
}

fn context<'a>(
    window: Option<&'a GameWindow>,
    elapsed: Duration,
    frame: u64,
    tick: u64,
    interpolation: f64,
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
        size,
        scale_factor,
        elapsed,
        frame,
        tick,
        interpolation,
        focused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Application, Flow};
    use quasar_core::FixedStepConfig;

    struct Noop;
    impl Application for Noop {
        type Error = std::convert::Infallible;
        fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
            Ok(())
        }
        fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
            Ok(Flow::Continue)
        }
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
}

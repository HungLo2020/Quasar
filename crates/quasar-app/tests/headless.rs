use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless};
use std::{cell::RefCell, error::Error, fmt, io, rc::Rc};

#[derive(Clone, Default)]
struct Events {
    setup: Rc<RefCell<u32>>,
    ticks: Rc<RefCell<u32>>,
    shutdown: Rc<RefCell<u32>>,
    seen_ticks: Rc<RefCell<Vec<u64>>>,
}
impl Application for Events {
    type Error = io::Error;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        *self.setup.borrow_mut() += 1;
        Ok(())
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        *self.ticks.borrow_mut() += 1;
        self.seen_ticks.borrow_mut().push(context.tick);
        Ok(Flow::Continue)
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        *self.shutdown.borrow_mut() += 1;
        Ok(())
    }
}

#[test]
fn headless_runner_orders_setup_ticks_and_shutdown() {
    let events = Events::default();
    let setup = Rc::clone(&events.setup);
    let ticks = Rc::clone(&events.ticks);
    let shutdown = Rc::clone(&events.shutdown);
    let seen = Rc::clone(&events.seen_ticks);
    run_headless(
        events,
        HeadlessConfig {
            ticks: 5,
            ..HeadlessConfig::default()
        },
    )
    .unwrap();
    assert_eq!(*setup.borrow(), 1);
    assert_eq!(*ticks.borrow(), 5);
    assert_eq!(*shutdown.borrow(), 1);
    assert_eq!(*seen.borrow(), vec![1, 2, 3, 4, 5]);
}

struct ExitAfter(Rc<RefCell<u32>>);
impl Application for ExitAfter {
    type Error = io::Error;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        *self.0.borrow_mut() += 1;
        Ok(if *self.0.borrow() == 2 {
            Flow::Exit
        } else {
            Flow::Continue
        })
    }
}

#[test]
fn headless_exit_stops_early_and_still_shuts_down() {
    let ticks = Rc::new(RefCell::new(0));
    run_headless(
        ExitAfter(Rc::clone(&ticks)),
        HeadlessConfig {
            ticks: 100,
            ..HeadlessConfig::default()
        },
    )
    .unwrap();
    assert_eq!(*ticks.borrow(), 2);
}

#[derive(Debug)]
struct Failure(&'static str);
impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}
impl Error for Failure {}

struct TickFailure(Rc<RefCell<bool>>);
impl Application for TickFailure {
    type Error = Failure;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Err(Failure("tick failed"))
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        *self.0.borrow_mut() = true;
        Ok(())
    }
}

#[test]
fn headless_error_is_returned_and_shutdown_runs() {
    let shutdown = Rc::new(RefCell::new(false));
    let error = run_headless(
        TickFailure(Rc::clone(&shutdown)),
        HeadlessConfig {
            ticks: 10,
            ..HeadlessConfig::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "tick failed");
    assert!(*shutdown.borrow());
}

#[derive(Default)]
struct PhaseEvents {
    frames: u32,
    ticks: u32,
    shutdown: u32,
    frame_before_tick: Vec<(u64, u64, f64)>,
    phases: Vec<quasar_app::AppPhase>,
}
impl Application for PhaseEvents {
    type Error = io::Error;
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.frames += 1;
        self.phases.push(context.phase);
        self.frame_before_tick
            .push((context.frame, context.tick, context.interpolation));
        Ok(Flow::Continue)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.ticks += 1;
        self.phases.push(context.phase);
        Ok(Flow::Continue)
    }
    fn shutdown(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.shutdown += 1;
        self.phases.push(context.phase);
        Ok(())
    }
}

#[test]
fn frame_updates_run_once_per_tick_and_before_each_tick() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(PhaseEvents::default()));
    let app = PhaseEventsHandle {
        inner: events.clone(),
    };
    run_headless(
        app,
        HeadlessConfig {
            ticks: 3,
            step: std::time::Duration::from_millis(10),
        },
    )
    .unwrap();
    let events = events.borrow().clone_for_summary();
    assert_eq!(events.frames, 3);
    assert_eq!(events.ticks, 3);
    assert_eq!(events.shutdown, 1);
    assert_eq!(events.frame_before_tick[0], (1, 0, 0.0));
    assert_eq!(events.frame_before_tick[2], (3, 2, 0.0));
    assert_eq!(
        events.phases,
        vec![
            quasar_app::AppPhase::Frame,
            quasar_app::AppPhase::Tick,
            quasar_app::AppPhase::Frame,
            quasar_app::AppPhase::Tick,
            quasar_app::AppPhase::Frame,
            quasar_app::AppPhase::Tick,
            quasar_app::AppPhase::Shutdown,
        ]
    );
}

#[derive(Clone)]
struct PhaseEventsHandle {
    inner: std::rc::Rc<std::cell::RefCell<PhaseEvents>>,
}
impl Application for PhaseEventsHandle {
    type Error = io::Error;
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.inner.borrow_mut().frame_update(context)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.inner.borrow_mut().tick(context)
    }
    fn shutdown(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.inner.borrow_mut().shutdown(context)
    }
}
impl PhaseEvents {
    fn clone_for_summary(&self) -> Self {
        Self {
            frames: self.frames,
            ticks: self.ticks,
            shutdown: self.shutdown,
            frame_before_tick: self.frame_before_tick.clone(),
            phases: self.phases.clone(),
        }
    }
}

struct SetupFailure {
    shutdown: std::rc::Rc<std::cell::RefCell<u32>>,
}
impl Application for SetupFailure {
    type Error = Failure;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Err(Failure("setup failed"))
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        *self.shutdown.borrow_mut() += 1;
        Ok(())
    }
}

#[test]
fn setup_failure_runs_shutdown_once_and_preserves_the_error() {
    let shutdown = std::rc::Rc::new(std::cell::RefCell::new(0));
    let error = run_headless(
        SetupFailure {
            shutdown: shutdown.clone(),
        },
        HeadlessConfig {
            ticks: 5,
            ..HeadlessConfig::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "setup failed");
    assert_eq!(*shutdown.borrow(), 1);
}

struct DoubleFailure {
    shutdown: std::rc::Rc<std::cell::RefCell<u32>>,
}
impl Application for DoubleFailure {
    type Error = Failure;
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Err(Failure("tick failed"))
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        *self.shutdown.borrow_mut() += 1;
        Err(Failure("shutdown failed"))
    }
}

#[test]
fn cleanup_failure_does_not_hide_the_primary_tick_error() {
    let shutdown = std::rc::Rc::new(std::cell::RefCell::new(0));
    let error = run_headless(
        DoubleFailure {
            shutdown: shutdown.clone(),
        },
        HeadlessConfig {
            ticks: 1,
            ..HeadlessConfig::default()
        },
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("tick failed"));
    assert!(message.contains("shutdown failed"));
    assert_eq!(*shutdown.borrow(), 1);
}

#[test]
fn zero_headless_step_is_rejected_before_callbacks() {
    let error = run_headless(
        PhaseEvents::default(),
        HeadlessConfig {
            ticks: 1,
            step: std::time::Duration::ZERO,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("step"));
}

#[derive(Default)]
struct DeferringEvents {
    ticks: u32,
    frames: u32,
    indices: Vec<(u64, u64)>,
    simulation_times: Vec<std::time::Duration>,
}
struct DeferringHandle(Rc<RefCell<DeferringEvents>>);
impl Application for DeferringHandle {
    type Error = io::Error;
    fn frame_update(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.0.borrow_mut().frames += 1;
        // A deferred frame is not a request to stop the runner.
        Ok(Flow::Deferred)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        let mut state = self.0.borrow_mut();
        state.ticks += 1;
        state.indices.push((context.frame, context.tick));
        state.simulation_times.push(context.timing.simulation_time);
        // Pause on every second attempted tick.
        Ok(if state.ticks.is_multiple_of(2) {
            Flow::Deferred
        } else {
            Flow::Continue
        })
    }
}

#[test]
fn deferred_ticks_do_not_advance_tick_index_or_simulation_time() {
    let state = Rc::new(RefCell::new(DeferringEvents::default()));
    run_headless(
        DeferringHandle(Rc::clone(&state)),
        HeadlessConfig {
            ticks: 4,
            step: std::time::Duration::from_millis(10),
        },
    )
    .unwrap();
    let state = state.borrow();
    assert_eq!(state.frames, 4);
    assert_eq!(state.ticks, 4);
    assert_eq!(
        state.indices,
        vec![(1, 1), (2, 2), (3, 2), (4, 3)],
        "a deferred attempt must reuse the same tick index on the next frame"
    );
    assert_eq!(
        state.simulation_times,
        vec![
            std::time::Duration::ZERO,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
        ]
    );
}

#[test]
fn headless_setup_is_skipped_after_validation_error() {
    let state = Rc::new(RefCell::new(DeferringEvents::default()));
    let error = run_headless(
        DeferringHandle(Rc::clone(&state)),
        HeadlessConfig {
            ticks: 1,
            step: std::time::Duration::ZERO,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("step"));
    assert_eq!(state.borrow().frames, 0);
    assert_eq!(state.borrow().ticks, 0);
}

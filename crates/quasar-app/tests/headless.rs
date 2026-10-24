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

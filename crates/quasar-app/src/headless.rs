use crate::{AppContext, Application, Flow};
use std::{error::Error, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeadlessConfig {
    pub ticks: u64,
    pub step: Duration,
}
impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            ticks: 60,
            step: Duration::from_secs_f64(1.0 / 60.0),
        }
    }
}

/// Runs a headless application for a bounded number of ticks.
///
/// No window or event loop is created. `setup`, `tick` and `shutdown` are called
/// in that order; `render` is intentionally not called.
pub fn run<A: Application>(
    mut application: A,
    config: HeadlessConfig,
) -> Result<(), Box<dyn Error>> {
    let mut context = AppContext {
        #[cfg(feature = "desktop")]
        window: None,
        size: (0, 0),
        scale_factor: 1.0,
        elapsed: Duration::ZERO,
        frame: 0,
        tick: 0,
        interpolation: 0.0,
        focused: true,
        #[cfg(not(feature = "desktop"))]
        marker: std::marker::PhantomData,
    };
    application
        .setup(&mut context)
        .map_err(|error| Box::new(error) as Box<dyn Error>)?;
    let mut result = Ok(());
    for tick in 0..config.ticks {
        context.elapsed = config.step;
        context.tick = tick + 1;
        context.frame = tick + 1;
        match application.tick(&mut context) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Exit) => break,
            Err(error) => {
                result = Err(Box::new(error) as Box<dyn Error>);
                break;
            }
        }
    }
    if let Err(error) = application.shutdown(&mut context)
        && result.is_ok()
    {
        result = Err(Box::new(error) as Box<dyn Error>);
    }
    result
}

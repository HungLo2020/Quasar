use crate::{AppContext, AppPhase, Application, Flow, TimingInfo};
use quasar_input::{FrameInput, InputState};
use std::{error::Error, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeadlessConfig {
    /// Number of deterministic frame/tick iterations.
    ///
    /// Each iteration runs `frame_update` once and then attempts one fixed
    /// tick. A tick that returns [`Flow::Deferred`] does not advance simulation
    /// time, but the iteration is still consumed.
    pub ticks: u64,
    /// Deterministic elapsed time per iteration.
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

/// Runs a headless application for a bounded number of deterministic iterations.
///
/// One `frame_update` and one `tick` run per iteration, with an empty input
/// snapshot. A deferred tick leaves transient input pending and does not
/// advance simulation time or tick indices. Rendering is intentionally not
/// called. `setup` and `shutdown` run exactly once; shutdown still runs after
/// setup/tick/frame failures.
pub fn run<A: Application>(
    mut application: A,
    config: HeadlessConfig,
) -> Result<(), Box<dyn Error>> {
    if config.step.is_zero() {
        return Err("headless step must be nonzero".into());
    }
    let mut state = InputState::default();
    let mut frame = FrameInput::default();
    state.capture_frame(&mut frame, 0);
    let mut primary: Option<Box<dyn Error>> = None;

    let setup_result = {
        let mut context = context(
            &mut frame,
            config.step,
            0,
            0,
            0.0,
            AppPhase::Frame,
            TimingInfo {
                fixed_step: config.step,
                ..TimingInfo::default()
            },
        );
        application.setup(&mut context)
    };
    if let Err(error) = setup_result {
        primary = Some(Box::new(error));
    }

    let mut simulated = Duration::ZERO;
    let mut ticks_done = 0u64;
    let mut frames_done = 0u64;
    if primary.is_none() {
        for index in 0..config.ticks {
            frames_done += 1;
            state.capture_frame(&mut frame, index + 1);
            let timing = TimingInfo {
                frame_elapsed: config.step,
                fixed_step: config.step,
                interpolation: 0.0,
                frame_index: index + 1,
                tick_index: ticks_done,
                ticks_this_frame: 1,
                simulation_time: simulated,
                accepted_elapsed: config.step,
                ..TimingInfo::default()
            };
            let flow = {
                let mut context = context(
                    &mut frame,
                    config.step,
                    index + 1,
                    ticks_done,
                    0.0,
                    AppPhase::Frame,
                    timing,
                );
                application.frame_update(&mut context)
            };
            // Consumption recorded while handling the frame applies before the
            // first fixed tick sees the snapshot.
            frame.apply_consumption(&mut state);
            match flow {
                Ok(Flow::Continue | Flow::Deferred) => {}
                Ok(Flow::Exit) => break,
                Err(error) => {
                    primary = Some(Box::new(error));
                    break;
                }
            }
            let timing = TimingInfo {
                tick_index: ticks_done + 1,
                interpolation: 1.0,
                ..timing
            };
            let flow = {
                let mut context = context(
                    &mut frame,
                    config.step,
                    index + 1,
                    ticks_done + 1,
                    1.0,
                    AppPhase::Tick,
                    timing,
                );
                application.tick(&mut context)
            };
            match flow {
                Ok(Flow::Continue) => {
                    ticks_done += 1;
                    simulated = simulated.saturating_add(config.step);
                    frame.apply_consumption(&mut state);
                    state.consume_edges();
                    frame.clear_transient();
                }
                Ok(Flow::Deferred) => {}
                Ok(Flow::Exit) => break,
                Err(error) => {
                    primary = Some(Box::new(error));
                    break;
                }
            }
        }
    }

    {
        let mut context = context(
            &mut frame,
            config.step,
            frames_done,
            ticks_done,
            0.0,
            AppPhase::Shutdown,
            TimingInfo {
                frame_elapsed: config.step,
                fixed_step: config.step,
                frame_index: frames_done,
                tick_index: ticks_done,
                simulation_time: simulated,
                ..TimingInfo::default()
            },
        );
        if let Err(error) = application.shutdown(&mut context) {
            if let Some(previous) = primary.take() {
                primary = Some(Box::new(CombinedError {
                    primary: previous,
                    cleanup: Box::new(error),
                }));
            } else {
                primary = Some(Box::new(error));
            }
        }
    }
    primary.map_or(Ok(()), Err)
}

#[allow(clippy::too_many_arguments)]
fn context<'a>(
    frame: &'a mut FrameInput,
    elapsed: Duration,
    frame_index: u64,
    tick_index: u64,
    interpolation: f64,
    phase: AppPhase,
    timing: TimingInfo,
) -> AppContext<'a> {
    AppContext {
        #[cfg(feature = "desktop")]
        window: None,
        input: frame,
        size: (0, 0),
        scale_factor: 1.0,
        elapsed,
        frame: frame_index,
        tick: tick_index,
        interpolation,
        timing: TimingInfo {
            frame_index,
            tick_index,
            interpolation,
            ..timing
        },
        phase,
        focused: true,
        focus_changed: None,
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
            "{}; shutdown also failed: {}",
            self.primary, self.cleanup
        )
    }
}
impl Error for CombinedError {}

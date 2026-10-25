//! Device-independent timing utilities. No window, renderer, or application loop required.
mod viewport;
use std::time::Duration;
pub use viewport::{DisplayOptions, Filter, Scaling, Viewport};

/// What happens when elapsed input would leave more work than the catch-up cap.
///
/// `Drop` preserves the historical Quasar behavior: the excess is discarded and
/// reported. `Preserve` keeps the backlog for later advances, while still
/// limiting the number of ticks returned from one `advance` call by
/// [`FixedStepConfig::max_catch_up_ticks`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BacklogPolicy {
    #[default]
    Drop,
    Preserve,
}

/// Timing policy. [`FixedStep::new`] retains its historical defaults through
/// [`FixedStepConfig::for_rate`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedStepConfig {
    /// Simulation frequency in Hz. Fractional rates are supported.
    pub hz: f64,
    /// Maximum ticks consumed after one `advance` call.
    pub max_catch_up_ticks: u32,
    /// Optional per-call input clamp. Suspension can be bounded before it reaches
    /// the accumulator.
    pub max_elapsed: Option<Duration>,
    /// Whether excess time beyond the catch-up allowance is dropped or retained.
    pub backlog: BacklogPolicy,
}
impl FixedStepConfig {
    pub fn for_rate(hz: f64, max_catch_up_ticks: u32) -> Self {
        Self {
            hz,
            max_catch_up_ticks,
            max_elapsed: None,
            backlog: BacklogPolicy::Drop,
        }
    }
}
impl Default for FixedStepConfig {
    fn default() -> Self {
        Self::for_rate(60.0, 8)
    }
}

/// Report returned by [`FixedStep::advance_with_report`].
///
/// `dropped` is meaningful for [`BacklogPolicy::Drop`]; `backlog` is the time
/// retained after the advance. `clamped` records input removed by
/// `max_elapsed`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdvanceReport {
    pub elapsed: Duration,
    pub accepted: Duration,
    pub clamped: Duration,
    pub dropped: Duration,
    pub backlog: Duration,
}

/// A bounded fixed-step accumulator. The game chooses its simulation frequency.
#[derive(Debug)]
pub struct FixedStep {
    config: FixedStepConfig,
    step: Duration,
    accumulated: Duration,
    catch_up_budget: u32,
    dropped_total: Duration,
    clamped_total: Duration,
}

impl FixedStep {
    /// Rejects invalid rates and caps catch-up work to `max_steps` per advance.
    ///
    /// This constructor keeps the historical API and semantics:
    /// `BacklogPolicy::Drop`, no separate elapsed clamp.
    pub fn new(hz: f64, max_steps: u32) -> Result<Self, &'static str> {
        Self::with_config(FixedStepConfig::for_rate(hz, max_steps))
    }

    /// Builds an accumulator with an exact integer-nanosecond step.
    ///
    /// Use this when reproducing a legacy frame duration such as MicroMages'
    /// `Duration::from_nanos(1_000_000_000 / 60)`. [`Self::new`] derives its step
    /// from a floating rate, so it may round a fractional step differently.
    pub fn from_step(step: Duration, max_catch_up_ticks: u32) -> Result<Self, &'static str> {
        if step.is_zero()
            || max_catch_up_ticks == 0
            || step.checked_mul(max_catch_up_ticks).is_none()
        {
            return Err("step and catch-up limit must be positive and finite");
        }
        let hz = 1.0 / step.as_secs_f64();
        let config = FixedStepConfig::for_rate(hz, max_catch_up_ticks);
        let mut clock = Self::with_config(config)?;
        clock.step = step;
        Ok(clock)
    }

    /// Builds an accumulator from an explicit timing policy.
    pub fn with_config(config: FixedStepConfig) -> Result<Self, &'static str> {
        if !config.hz.is_finite() || config.hz <= 0.0 || config.max_catch_up_ticks == 0 {
            return Err("frequency and catch-up limit must be positive and finite");
        }
        if config.max_elapsed.is_some_and(|elapsed| elapsed.is_zero()) {
            return Err("elapsed clamp must be nonzero");
        }
        let step = Duration::try_from_secs_f64(1.0 / config.hz)
            .map_err(|_| "frequency is outside the supported range")?;
        if step.is_zero() || step.checked_mul(config.max_catch_up_ticks).is_none() {
            return Err("frequency is outside the supported range");
        }
        Ok(Self {
            config,
            step,
            accumulated: Duration::ZERO,
            catch_up_budget: config.max_catch_up_ticks,
            dropped_total: Duration::ZERO,
            clamped_total: Duration::ZERO,
        })
    }

    pub fn config(&self) -> FixedStepConfig {
        self.config
    }
    /// Fractional input is preserved in the `Duration` accumulator; no float
    /// timestamp is used for tick decisions.
    pub fn advance(&mut self, elapsed: Duration) {
        let _ = self.advance_with_report(elapsed);
    }

    /// Accumulates elapsed time with explicit clamp/backlog accounting.
    ///
    /// The number of ticks that can be consumed after this call is bounded by
    /// `max_catch_up_ticks`. Calling `advance` again starts a new catch-up
    /// budget, even when `Preserve` retained a backlog.
    pub fn advance_with_report(&mut self, elapsed: Duration) -> AdvanceReport {
        let accepted = self
            .config
            .max_elapsed
            .map_or(elapsed, |limit| elapsed.min(limit));
        let clamped = elapsed.saturating_sub(accepted);
        self.accumulated = self.accumulated.saturating_add(accepted);
        let cap = self.step * self.config.max_catch_up_ticks;
        let dropped = match self.config.backlog {
            BacklogPolicy::Drop if self.accumulated > cap => {
                let dropped = self.accumulated - cap;
                self.accumulated = cap;
                dropped
            }
            BacklogPolicy::Drop | BacklogPolicy::Preserve => Duration::ZERO,
        };
        self.clamped_total = self.clamped_total.saturating_add(clamped);
        self.dropped_total = self.dropped_total.saturating_add(dropped);
        self.catch_up_budget = self.config.max_catch_up_ticks;
        AdvanceReport {
            elapsed,
            accepted,
            clamped,
            dropped,
            backlog: self.accumulated,
        }
    }

    /// Consumes exactly one simulation step when enough time has accumulated and
    /// the current advance's catch-up budget still has room.
    pub fn take_tick(&mut self) -> bool {
        if self.catch_up_budget == 0 || self.accumulated < self.step {
            return false;
        }
        self.accumulated -= self.step;
        self.catch_up_budget -= 1;
        true
    }

    pub fn step(&self) -> Duration {
        self.step
    }
    pub fn until_next_tick(&self) -> Duration {
        self.step.saturating_sub(self.accumulated)
    }
    pub fn interpolation(&self) -> f64 {
        self.accumulated.as_secs_f64() / self.step.as_secs_f64()
    }
    /// Time retained for future ticks.
    pub fn backlog(&self) -> Duration {
        self.accumulated
    }
    /// Remaining catch-up ticks available to this advance.
    pub fn catch_up_remaining(&self) -> u32 {
        self.catch_up_budget
    }
    /// Number of ticks currently due, bounded by the remaining catch-up budget.
    /// This lets a runner describe the frame before consuming ticks.
    pub fn pending_ticks(&self) -> u32 {
        let due = (self.accumulated.as_nanos() / self.step.as_nanos().max(1)) as u32;
        due.min(self.catch_up_budget)
    }
    pub fn dropped_total(&self) -> Duration {
        self.dropped_total
    }
    pub fn clamped_total(&self) -> Duration {
        self.clamped_total
    }
    /// Call on pause/resume so time spent suspended is never simulated.
    pub fn reset(&mut self) {
        self.accumulated = Duration::ZERO;
        self.catch_up_budget = self.config.max_catch_up_ticks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_fractional_rates_and_bounds_catchup() {
        let mut clock = FixedStep::new(60.0988, 5).unwrap();
        clock.advance(Duration::from_secs(10));
        let mut ticks = 0;
        while clock.take_tick() {
            ticks += 1;
        }
        assert_eq!(ticks, 5);
        assert_eq!(clock.until_next_tick(), clock.step());
        clock.advance(clock.step() / 2);
        assert!(!clock.take_tick());
        assert!((clock.interpolation() - 0.5).abs() < 0.000001);
        clock.reset();
        assert_eq!(clock.interpolation(), 0.0);
    }

    #[test]
    fn rejects_invalid_rates() {
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e30, 1e-30] {
            assert!(FixedStep::new(rate, 5).is_err());
        }
        assert!(
            FixedStep::with_config(FixedStepConfig {
                max_elapsed: Some(Duration::ZERO),
                ..FixedStepConfig::default()
            })
            .is_err()
        );
    }

    #[test]
    fn elapsed_clamp_reports_and_limits_suspension_spikes() {
        let mut clock = FixedStep::with_config(FixedStepConfig {
            max_elapsed: Some(Duration::from_millis(100)),
            ..FixedStepConfig::for_rate(60.0, 3)
        })
        .unwrap();
        let report = clock.advance_with_report(Duration::from_secs(10));
        assert_eq!(report.accepted, Duration::from_millis(100));
        assert_eq!(
            report.clamped,
            Duration::from_secs(10) - Duration::from_millis(100)
        );
        assert!(report.dropped > Duration::ZERO);
        assert_eq!(
            clock.clamped_total(),
            Duration::from_secs(10) - Duration::from_millis(100)
        );
        let mut ticks = 0;
        while clock.take_tick() {
            ticks += 1;
        }
        assert_eq!(ticks, 3);
    }

    #[test]
    fn preserve_backlog_and_catch_up_budget_are_independent() {
        let mut clock = FixedStep::with_config(FixedStepConfig {
            backlog: BacklogPolicy::Preserve,
            ..FixedStepConfig::for_rate(60.0, 2)
        })
        .unwrap();
        let report = clock.advance_with_report(Duration::from_millis(100));
        assert_eq!(report.dropped, Duration::ZERO);
        assert!(clock.backlog() > clock.step() * 2);
        assert_eq!(clock.catch_up_remaining(), 2);
        assert!(clock.take_tick());
        assert!(clock.take_tick());
        assert!(!clock.take_tick());
        assert!(clock.backlog() > Duration::ZERO);
        clock.advance(Duration::ZERO);
        assert_eq!(clock.catch_up_remaining(), 2);
        assert!(clock.take_tick());
    }

    #[test]
    fn micromages_steady_60hz_deadline_sequence_matches_fixed_step() {
        // MicroMages uses this exact truncated frame duration and ticks once when
        // its next deadline is reached. Compare equivalent steady/slow frame
        // sequences; a fixed-step accumulator deliberately differs under stalls.
        const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 60);
        let frame_times: Vec<Duration> = (0..600)
            .map(|index| FRAME + Duration::from_micros(index as u64 % 100))
            .collect();
        let mut next_deadline = FRAME;
        let mut legacy_ticks = 0usize;
        let mut elapsed = Duration::ZERO;
        for frame in &frame_times {
            elapsed += *frame;
            if elapsed >= next_deadline {
                legacy_ticks += 1;
                next_deadline = elapsed + FRAME;
            }
        }
        let mut clock = FixedStep::from_step(FRAME, 1).unwrap();
        let mut fixed_ticks = 0usize;
        for frame in &frame_times {
            clock.advance(*frame);
            while clock.take_tick() {
                fixed_ticks += 1;
            }
        }
        assert_eq!(fixed_ticks, legacy_ticks);
        assert_eq!(fixed_ticks, frame_times.len());
    }

    fn next_random(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn randomized_stalls_and_sustained_overload_remain_bounded() {
        let mut clock = FixedStep::with_config(FixedStepConfig {
            max_elapsed: Some(Duration::from_millis(250)),
            ..FixedStepConfig::for_rate(60.0, 8)
        })
        .unwrap();
        let mut seed = 0x9e37_79b9_7f4a_7c15;
        let mut total_ticks = 0u64;
        for _ in 0..20_000 {
            let frame_ns = 1 + next_random(&mut seed) % 400_000_000;
            let report = clock.advance_with_report(Duration::from_nanos(frame_ns));
            assert!(report.accepted <= Duration::from_millis(250));
            assert_eq!(report.elapsed, Duration::from_nanos(frame_ns));
            let mut ticks = 0;
            while clock.take_tick() {
                ticks += 1;
            }
            assert!(ticks <= 8);
            total_ticks += ticks;
            assert!(clock.backlog() <= clock.step() * 8);
            assert_eq!(
                report.clamped,
                Duration::from_nanos(frame_ns).saturating_sub(report.accepted)
            );
        }
        assert!(total_ticks > 0);
        assert!(clock.dropped_total() > Duration::ZERO);
    }

    #[test]
    fn preserve_backlog_is_bounded_per_advance_but_survives_stalls() {
        let mut clock = FixedStep::with_config(FixedStepConfig {
            backlog: BacklogPolicy::Preserve,
            max_elapsed: Some(Duration::from_millis(250)),
            ..FixedStepConfig::for_rate(60.0, 4)
        })
        .unwrap();
        let mut seed = 0x1234_5678;
        for _ in 0..1_000 {
            let frame_ns = 1 + next_random(&mut seed) % 500_000_000;
            let report = clock.advance_with_report(Duration::from_nanos(frame_ns));
            let mut ticks = 0;
            while clock.take_tick() {
                ticks += 1;
            }
            assert!(ticks <= 4);
            assert_eq!(report.dropped, Duration::ZERO);
        }
        assert!(clock.backlog() > Duration::ZERO);
    }

    #[test]
    fn randomized_pause_resume_discards_only_paused_time() {
        let mut clock = FixedStep::with_config(FixedStepConfig {
            max_elapsed: Some(Duration::from_millis(250)),
            ..FixedStepConfig::for_rate(60.0, 8)
        })
        .unwrap();
        let mut seed = 0xa5a5_5a5a_1234_9876;
        let mut saw_reset = false;
        for index in 0..10_000u64 {
            if next_random(&mut seed).is_multiple_of(13) {
                clock.reset();
                assert_eq!(clock.backlog(), Duration::ZERO);
                assert_eq!(clock.catch_up_remaining(), 8);
                saw_reset = true;
                continue;
            }
            let frame_ns = 1 + next_random(&mut seed) % 2_000_000_000;
            clock.advance(Duration::from_nanos(frame_ns));
            let mut ticks = 0;
            while clock.take_tick() {
                ticks += 1;
            }
            assert!(ticks <= 8, "index={index}");
            assert!(clock.backlog() < clock.step().max(Duration::from_nanos(1)));
        }
        assert!(saw_reset);
    }

    #[test]
    fn pause_resume_and_long_running_fractional_rates_do_not_drift() {
        let mut clock = FixedStep::new(60.0, 8).unwrap();
        clock.advance(clock.step() * 3);
        assert!(clock.take_tick());
        assert!(clock.take_tick());
        clock.reset();
        assert!(!clock.take_tick());
        assert_eq!(clock.interpolation(), 0.0);
        clock.advance(clock.step());
        assert!(clock.take_tick());

        let mut clock = FixedStep::new(59.94, 8).unwrap();
        let frame = Duration::from_nanos(1_000_000_000 / 120);
        let mut ticks = 0u64;
        for _ in 0..120 * 60 * 60 {
            clock.advance(frame);
            while clock.take_tick() {
                ticks += 1;
            }
        }
        let expected = (3600.0 / clock.step().as_secs_f64()).floor() as u64;
        assert!(
            ticks.abs_diff(expected) <= 2,
            "ticks={ticks} expected={expected}"
        );
    }

    #[test]
    fn exact_step_is_preserved_for_legacy_rates() {
        const LEGACY_FRAME: Duration = Duration::from_nanos(1_000_000_000 / 60);
        let clock = FixedStep::from_step(LEGACY_FRAME, 1).unwrap();
        assert_eq!(clock.step(), LEGACY_FRAME);
        assert_eq!(
            FixedStep::new(60.0, 1).unwrap().step(),
            Duration::from_nanos(16_666_667)
        );
    }

    #[test]
    fn historical_new_and_default_config_are_behaviorally_identical() {
        let mut legacy = FixedStep::new(60.0, 8).unwrap();
        let mut configured = FixedStep::with_config(FixedStepConfig::for_rate(60.0, 8)).unwrap();
        assert_eq!(legacy.config(), configured.config());
        let mut seed = 0xfeed_face_dead_beef;
        for _ in 0..5_000 {
            let frame = Duration::from_nanos(1 + next_random(&mut seed) % 350_000_000);
            let legacy_report = legacy.advance_with_report(frame);
            let configured_report = configured.advance_with_report(frame);
            assert_eq!(legacy_report, configured_report);
            assert_eq!(legacy.backlog(), configured.backlog());
            loop {
                let legacy_tick = legacy.take_tick();
                assert_eq!(legacy_tick, configured.take_tick());
                if !legacy_tick {
                    break;
                }
            }
            assert_eq!(legacy.interpolation(), configured.interpolation());
            assert_eq!(legacy.until_next_tick(), configured.until_next_tick());
            assert_eq!(legacy.catch_up_remaining(), configured.catch_up_remaining());
        }
        assert_eq!(legacy.dropped_total(), configured.dropped_total());
    }

    #[test]
    fn preserve_reset_discards_retained_backlog() {
        let mut clock = FixedStep::with_config(FixedStepConfig {
            backlog: BacklogPolicy::Preserve,
            ..FixedStepConfig::for_rate(60.0, 2)
        })
        .unwrap();
        clock.advance(Duration::from_millis(500));
        assert!(clock.backlog() > Duration::ZERO);
        assert!(clock.take_tick());
        clock.reset();
        assert_eq!(clock.backlog(), Duration::ZERO);
        assert_eq!(clock.catch_up_remaining(), 2);
        assert!(!clock.take_tick());
        clock.advance(clock.step());
        assert!(clock.take_tick());
    }

    #[test]
    fn fractional_interpolation_and_reset_after_backlog() {
        let mut clock = FixedStep::new(60.0, 4).unwrap();
        clock.advance(clock.step() + clock.step() / 4);
        assert!(clock.take_tick());
        assert!((clock.interpolation() - 0.25).abs() < 1e-6);
        clock.reset();
        assert_eq!(clock.backlog(), Duration::ZERO);
        assert_eq!(clock.interpolation(), 0.0);
        assert_eq!(clock.catch_up_remaining(), 4);
    }
}

#[cfg(test)]
mod pending_ticks_tests {
    use super::*;

    #[test]
    fn pending_ticks_reports_due_work_without_consuming_it() {
        let mut clock = FixedStep::new(60.0, 8).unwrap();
        clock.advance(clock.step() * 3);
        assert_eq!(clock.pending_ticks(), 3);
        assert!(clock.take_tick());
        assert_eq!(clock.pending_ticks(), 2);
        let mut ticks = 0;
        while clock.take_tick() {
            ticks += 1;
        }
        assert_eq!(ticks, 2);
        assert_eq!(clock.pending_ticks(), 0);
    }
}

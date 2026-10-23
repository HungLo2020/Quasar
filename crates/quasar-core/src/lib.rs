//! Device-independent timing utilities. No window, renderer, or application loop required.
use std::time::Duration;

/// A bounded fixed-step accumulator. The game chooses its simulation frequency.
#[derive(Debug)]
pub struct FixedStep {
    step: Duration,
    accumulated: Duration,
    max_steps: u32,
}

impl FixedStep {
    /// Rejects invalid rates and caps catch-up work to `max_steps` per advance.
    pub fn new(hz: f64, max_steps: u32) -> Result<Self, &'static str> {
        if !hz.is_finite() || hz <= 0.0 || max_steps == 0 {
            return Err("frequency and catch-up limit must be positive and finite");
        }
        let step = Duration::try_from_secs_f64(1.0 / hz)
            .map_err(|_| "frequency is outside the supported range")?;
        if step.is_zero() || step.checked_mul(max_steps).is_none() {
            return Err("frequency is outside the supported range");
        }
        Ok(Self {
            step,
            accumulated: Duration::ZERO,
            max_steps,
        })
    }

    /// Accumulates elapsed time; excess time is discarded to prevent a catch-up spiral.
    pub fn advance(&mut self, elapsed: Duration) {
        self.accumulated = self
            .accumulated
            .saturating_add(elapsed)
            .min(self.step * self.max_steps);
    }

    /// Consumes exactly one simulation step when enough time has accumulated.
    pub fn take_tick(&mut self) -> bool {
        if self.accumulated < self.step {
            return false;
        }
        self.accumulated -= self.step;
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
    /// Call on pause/resume so time spent suspended is never simulated.
    pub fn reset(&mut self) {
        self.accumulated = Duration::ZERO;
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
    }
}

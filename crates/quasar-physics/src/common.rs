use std::fmt;

/// Largest fixed timestep accepted by [`StepConfig`]. Bounding the step keeps
/// tunneling risk and solver work predictable.
pub const MAX_FIXED_DT: f32 = 0.25;

/// Fixed-step configuration for a caller-owned physics world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepConfig {
    /// Exact timestep used by the dimension-specific `PhysicsWorld::step_fixed` methods.
    pub fixed_dt: f32,
    /// Largest frame delta accepted by `advance`; larger deltas are clamped.
    pub max_frame_dt: f32,
    /// Maximum fixed steps executed by one `advance` call. Leftover time is
    /// discarded when the cap is reached (the report marks `saturated`).
    pub max_substeps: u32,
}

impl StepConfig {
    pub fn new(fixed_dt: f32, max_frame_dt: f32, max_substeps: u32) -> Result<Self, PhysicsError> {
        let config = Self {
            fixed_dt,
            max_frame_dt,
            max_substeps,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(self) -> Result<Self, PhysicsError> {
        if !self.fixed_dt.is_finite()
            || !self.max_frame_dt.is_finite()
            || self.fixed_dt <= 0.0
            || self.max_frame_dt < 0.0
            || self.fixed_dt > MAX_FIXED_DT
            || self.max_substeps == 0
        {
            return Err(PhysicsError::InvalidStep);
        }
        Ok(self)
    }
}

impl Default for StepConfig {
    fn default() -> Self {
        Self {
            fixed_dt: 1.0 / 60.0,
            max_frame_dt: 0.25,
            max_substeps: 8,
        }
    }
}

/// Gravity and stepping configuration for a physics world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldConfig<const D: usize> {
    pub gravity: [f32; D],
    pub step: StepConfig,
}

impl<const D: usize> WorldConfig<D> {
    pub fn new(gravity: [f32; D], step: StepConfig) -> Result<Self, PhysicsError> {
        let config = Self { gravity, step };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(self) -> Result<Self, PhysicsError> {
        if self.gravity.iter().any(|value| !value.is_finite()) {
            return Err(PhysicsError::InvalidGravity);
        }
        self.step.validate()?;
        Ok(self)
    }
}

impl Default for WorldConfig<2> {
    fn default() -> Self {
        Self {
            gravity: [0.0, -9.81],
            step: StepConfig::default(),
        }
    }
}

impl Default for WorldConfig<3> {
    fn default() -> Self {
        Self {
            gravity: [0.0, -9.81, 0.0],
            step: StepConfig::default(),
        }
    }
}

/// Result of one explicit stepping call.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepReport {
    /// Number of fixed steps executed by this call (0 or 1 for `step_fixed`).
    pub steps: u32,
    /// Simulated time covered by this call.
    pub simulated_seconds: f64,
    /// `advance` hit `max_substeps` and discarded leftover time.
    pub saturated: bool,
}

/// Events drained after a step from Rapier's collision and contact channels.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsEvents<C, F> {
    pub collisions: Vec<C>,
    pub contact_forces: Vec<F>,
}

impl<C, F> Default for PhysicsEvents<C, F> {
    fn default() -> Self {
        Self {
            collisions: Vec::new(),
            contact_forces: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicsError {
    InvalidGravity,
    InvalidStep,
    InvalidDelta,
}

impl PhysicsError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidGravity => "physics.invalid_gravity",
            Self::InvalidStep => "physics.invalid_step",
            Self::InvalidDelta => "physics.invalid_delta",
        }
    }
}

impl fmt::Display for PhysicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidGravity => "gravity components must be finite",
            Self::InvalidStep => {
                "step configuration requires 0 < fixed_dt <= 0.25, finite max_frame_dt >= 0 and max_substeps > 0"
            }
            Self::InvalidDelta => "frame delta must be finite and non-negative",
        })
    }
}

impl std::error::Error for PhysicsError {}

#[cfg(feature = "dynamics")]
pub(crate) fn validate_delta(delta: f32) -> Result<f32, PhysicsError> {
    if !delta.is_finite() || delta < 0.0 {
        return Err(PhysicsError::InvalidDelta);
    }
    Ok(delta)
}

pub(crate) fn normalize_direction<const D: usize>(direction: [f32; D]) -> Option<[f32; D]> {
    if direction.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let norm = direction
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return None;
    }
    let mut normalized = [0.0; D];
    for (axis, value) in direction.iter().enumerate() {
        normalized[axis] = (*value as f64 / norm) as f32;
    }
    Some(normalized)
}

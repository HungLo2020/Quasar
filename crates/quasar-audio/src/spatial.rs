//! Listener transforms, distance attenuation and stereo positioning.
//!
//! Games can use the built-in models or supply a [`SpatialModel`] to preserve
//! title-specific attenuation/pan rules. Coordinate units are game-defined;
//! the mixer treats them as metres for the default models.

use std::{fmt, sync::Arc};

/// A listener's world position and orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// Listener position in world units.
    pub position: [f32; 3],
    /// Forward direction (normalized by the mixer when used).
    pub forward: [f32; 3],
    /// Up direction (normalized by the mixer when used).
    pub up: [f32; 3],
}
impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
        }
    }
}
impl Listener {
    /// Right vector, `forward × up`.
    pub fn right(&self) -> [f32; 3] {
        cross(self.forward, self.up)
    }
}

/// Per-emitter spatial configuration.
#[derive(Clone)]
pub struct SpatialConfig {
    /// Emitter world position.
    pub position: [f32; 3],
    /// Attenuation model.
    pub model: SpatialModelKind,
    /// Stereo pan strength, normally `1.0`.
    pub pan_strength: f32,
}
impl Default for SpatialConfig {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            model: SpatialModelKind::Inverse {
                min_distance: 1.0,
                max_distance: 64.0,
                rolloff: 1.0,
            },
            pan_strength: 1.0,
        }
    }
}
impl fmt::Debug for SpatialConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpatialConfig")
            .field("position", &self.position)
            .field("model", &self.model)
            .field("pan_strength", &self.pan_strength)
            .finish()
    }
}

/// Built-in distance models. `Custom` lets a game override both gain and pan.
#[derive(Clone)]
pub enum SpatialModelKind {
    /// No attenuation; pan still follows the listener.
    None,
    /// Linear falloff from `min_distance` to `max_distance`.
    Linear {
        /// Full-gain radius.
        min_distance: f32,
        /// Silent radius.
        max_distance: f32,
    },
    /// Inverse falloff, clamped at `max_distance` (OpenAL `InverseClamped`).
    Inverse {
        /// Full-gain radius.
        min_distance: f32,
        /// Silent radius.
        max_distance: f32,
        /// Falloff strength.
        rolloff: f32,
    },
    /// Inverse-power falloff, clamped at `max_distance`.
    Exponential {
        /// Full-gain radius.
        min_distance: f32,
        /// Silent radius.
        max_distance: f32,
        /// Falloff exponent.
        rolloff: f32,
    },
    /// Game-owned model. It is called on the mixing thread and must not block,
    /// allocate or perform I/O.
    Custom(Arc<dyn SpatialModel>),
}
impl fmt::Debug for SpatialModelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Linear {
                min_distance,
                max_distance,
            } => f
                .debug_struct("Linear")
                .field("min_distance", min_distance)
                .field("max_distance", max_distance)
                .finish(),
            Self::Inverse {
                min_distance,
                max_distance,
                rolloff,
            } => f
                .debug_struct("Inverse")
                .field("min_distance", min_distance)
                .field("max_distance", max_distance)
                .field("rolloff", rolloff)
                .finish(),
            Self::Exponential {
                min_distance,
                max_distance,
                rolloff,
            } => f
                .debug_struct("Exponential")
                .field("min_distance", min_distance)
                .field("max_distance", max_distance)
                .field("rolloff", rolloff)
                .finish(),
            Self::Custom(_) => f.write_str("Custom(..)"),
        }
    }
}

/// Output of a spatial evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialGain {
    /// Linear gain multiplier.
    pub gain: f32,
    /// Stereo pan in `[-1, 1]`, left to right.
    pub pan: f32,
}

/// Custom spatial evaluation. Implementations run on the audio thread.
pub trait SpatialModel: Send + Sync {
    /// Returns gain and pan for an emitter position.
    fn evaluate(&self, listener: &Listener, emitter: [f32; 3]) -> SpatialGain;
}

/// Default evaluation for built-in models.
pub fn evaluate(
    listener: &Listener,
    emitter: [f32; 3],
    model: &SpatialModelKind,
    pan_strength: f32,
) -> SpatialGain {
    let delta = subtract(emitter, listener.position);
    let distance = length(delta);
    let gain = match model {
        SpatialModelKind::None => 1.0,
        SpatialModelKind::Custom(custom) => {
            let result = custom.evaluate(listener, emitter);
            return SpatialGain {
                gain: if result.gain.is_finite() {
                    result.gain.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                pan: if result.pan.is_finite() {
                    result.pan.clamp(-1.0, 1.0)
                } else {
                    0.0
                },
            };
        }
        SpatialModelKind::Linear {
            min_distance,
            max_distance,
        } => linear(distance, *min_distance, *max_distance),
        SpatialModelKind::Inverse {
            min_distance,
            max_distance,
            rolloff,
        } => inverse(distance, *min_distance, *max_distance, *rolloff),
        SpatialModelKind::Exponential {
            min_distance,
            max_distance,
            rolloff,
        } => exponential(distance, *min_distance, *max_distance, *rolloff),
    };
    let pan = if distance > f32::EPSILON {
        let direction = scale(delta, 1.0 / distance);
        let right = normalize(listener.right()).unwrap_or([0.0; 3]);
        (dot(direction, right) * pan_strength.clamp(-1.0, 1.0)).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    SpatialGain {
        gain: if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            0.0
        },
        pan: if pan.is_finite() { pan } else { 0.0 },
    }
}

fn linear(distance: f32, min: f32, max: f32) -> f32 {
    let (min, max) = ordered_range(min, max);
    if distance <= min {
        1.0
    } else if distance >= max {
        0.0
    } else {
        1.0 - (distance - min) / (max - min)
    }
}
fn inverse(distance: f32, min: f32, max: f32, rolloff: f32) -> f32 {
    let (min, max) = ordered_range(min, max);
    if distance <= min {
        1.0
    } else if distance >= max {
        0.0
    } else {
        (min / (min + rolloff.max(0.0) * (distance - min))).clamp(0.0, 1.0)
    }
}
fn exponential(distance: f32, min: f32, max: f32, rolloff: f32) -> f32 {
    let (min, max) = ordered_range(min, max);
    if distance <= min {
        1.0
    } else if distance >= max {
        0.0
    } else {
        (distance / min).powf(-rolloff.max(0.0)).clamp(0.0, 1.0)
    }
}
fn ordered_range(min: f32, max: f32) -> (f32, f32) {
    let min = min.max(0.0001);
    (min, max.max(min))
}
fn subtract(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], factor: f32) -> [f32; 3] {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let length = length(a);
    (length > f32::EPSILON).then(|| scale(a, 1.0 / length))
}

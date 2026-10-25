use std::fmt;

/// A typed animated value. Quaternions use `[x, y, z, w]` and are blended by
/// normalized shortest-path slerp.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Quat([f32; 4]),
    Bool(bool),
    U32(u32),
}

impl Value {
    pub fn is_finite(self) -> bool {
        match self {
            Self::Float(value) => value.is_finite(),
            Self::Vec2(values) => values.iter().all(|value| value.is_finite()),
            Self::Vec3(values) => values.iter().all(|value| value.is_finite()),
            Self::Vec4(values) => values.iter().all(|value| value.is_finite()),
            Self::Quat(values) => values.iter().all(|value| value.is_finite()),
            Self::Bool(_) | Self::U32(_) => true,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::Float(_) => "float",
            Self::Vec2(_) => "vec2",
            Self::Vec3(_) => "vec3",
            Self::Vec4(_) => "vec4",
            Self::Quat(_) => "quat",
            Self::Bool(_) => "bool",
            Self::U32(_) => "u32",
        }
    }

    pub fn same_kind(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }

    pub fn as_f32(self) -> Option<f32> {
        match self {
            Self::Float(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_u32(self) -> Option<u32> {
        match self {
            Self::U32(value) => Some(value),
            Self::Float(value) if value >= 0.0 && value <= u32::MAX as f32 => {
                Some(value.round() as u32)
            }
            _ => None,
        }
    }

    pub fn as_vec3(self) -> Option<[f32; 3]> {
        match self {
            Self::Vec3(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_quat(self) -> Option<[f32; 4]> {
        match self {
            Self::Quat(value) => Some(value),
            _ => None,
        }
    }

    /// Interpolates two values at `t`. Returns `None` for incompatible kinds or
    /// an invalid quaternion. `t` is clamped to `[0, 1]`.
    pub fn lerp(self, other: Self, t: f32) -> Option<Self> {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        match (self, other) {
            (Self::Float(a), Self::Float(b)) => Some(Self::Float(a + (b - a) * t)),
            (Self::Vec2(a), Self::Vec2(b)) => Some(Self::Vec2([
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
            ])),
            (Self::Vec3(a), Self::Vec3(b)) => Some(Self::Vec3([
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ])),
            (Self::Vec4(a), Self::Vec4(b)) => Some(Self::Vec4([
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
                a[3] + (b[3] - a[3]) * t,
            ])),
            (Self::Quat(a), Self::Quat(b)) => Some(Self::Quat(slerp(a, b, t))),
            (Self::Bool(a), Self::Bool(b)) => Some(Self::Bool(if t < 0.5 { a } else { b })),
            (Self::U32(a), Self::U32(b)) => Some(Self::U32(if t < 0.5 { a } else { b })),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Vec2(value) => write!(formatter, "{value:?}"),
            Self::Vec3(value) => write!(formatter, "{value:?}"),
            Self::Vec4(value) => write!(formatter, "{value:?}"),
            Self::Quat(value) => write!(formatter, "{value:?}"),
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::U32(value) => write!(formatter, "{value}"),
        }
    }
}

fn normalize_quat(value: [f32; 4]) -> Option<[f32; 4]> {
    let norm = value.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return None;
    }
    Some([
        value[0] / norm,
        value[1] / norm,
        value[2] / norm,
        value[3] / norm,
    ])
}

fn slerp(mut a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let Some(normalized_a) = normalize_quat(a) else {
        return a;
    };
    a = normalized_a;
    let Some(normalized_b) = normalize_quat(b) else {
        return a;
    };
    b = normalized_b;
    let mut dot = a
        .iter()
        .zip(b)
        .map(|(left, right)| left * right)
        .sum::<f32>();
    if dot < 0.0 {
        for value in &mut b {
            *value = -*value;
        }
        dot = -dot;
    }
    dot = dot.clamp(-1.0, 1.0);
    if dot > 0.9995 {
        let mut result = [0.0; 4];
        for (index, value) in result.iter_mut().enumerate() {
            *value = a[index] + (b[index] - a[index]) * t;
        }
        return normalize_quat(result).unwrap_or(a);
    }
    let theta = dot.acos();
    let sin_theta = theta.sin();
    if sin_theta.abs() <= f32::EPSILON {
        return a;
    }
    let weight_a = ((1.0 - t) * theta).sin() / sin_theta;
    let weight_b = (t * theta).sin() / sin_theta;
    let mut result = [0.0; 4];
    for (index, value) in result.iter_mut().enumerate() {
        *value = a[index] * weight_a + b[index] * weight_b;
    }
    normalize_quat(result).unwrap_or(a)
}

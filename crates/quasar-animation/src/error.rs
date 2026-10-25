use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationError {
    EmptyClip,
    EmptyTrack,
    InvalidDuration,
    NonFinite,
    InvalidKeyframeOrder,
    DuplicateProperty,
    ValueTypeMismatch,
    UnsupportedInterpolation,
    InvalidMarker,
    UnknownState,
    DuplicateState,
    InvalidTransition,
    InvalidLayer,
    InvalidSkeleton,
    Adapter(String),
}

impl AnimationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EmptyClip => "animation.empty_clip",
            Self::EmptyTrack => "animation.empty_track",
            Self::InvalidDuration => "animation.invalid_duration",
            Self::NonFinite => "animation.non_finite",
            Self::InvalidKeyframeOrder => "animation.invalid_keyframe_order",
            Self::DuplicateProperty => "animation.duplicate_property",
            Self::ValueTypeMismatch => "animation.value_type_mismatch",
            Self::UnsupportedInterpolation => "animation.unsupported_interpolation",
            Self::InvalidMarker => "animation.invalid_marker",
            Self::UnknownState => "animation.unknown_state",
            Self::DuplicateState => "animation.duplicate_state",
            Self::InvalidTransition => "animation.invalid_transition",
            Self::InvalidLayer => "animation.invalid_layer",
            Self::InvalidSkeleton => "animation.invalid_skeleton",
            Self::Adapter(_) => "animation.adapter",
        }
    }
}

impl fmt::Display for AnimationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyClip => formatter.write_str("a clip needs at least one track or marker"),
            Self::EmptyTrack => formatter.write_str("a track needs at least one keyframe"),
            Self::InvalidDuration => {
                formatter.write_str("clip duration and playback delta must be finite and >= 0")
            }
            Self::NonFinite => formatter.write_str("animation values and times must be finite"),
            Self::InvalidKeyframeOrder => formatter
                .write_str("keyframe times must be strictly increasing and within duration"),
            Self::DuplicateProperty => formatter.write_str("clip has duplicate property tracks"),
            Self::ValueTypeMismatch => {
                formatter.write_str("keyframes in one track must have the same value type")
            }
            Self::UnsupportedInterpolation => {
                formatter.write_str("bool/integer values only support step interpolation")
            }
            Self::InvalidMarker => {
                formatter.write_str("markers need a non-empty name and time within duration")
            }
            Self::UnknownState => formatter.write_str("state is not defined in this layer"),
            Self::DuplicateState => formatter.write_str("state name is already defined"),
            Self::InvalidTransition => {
                formatter.write_str("transition source/target or duration is invalid")
            }
            Self::InvalidLayer => formatter.write_str("layer index is out of range"),
            Self::InvalidSkeleton => {
                formatter.write_str("skeleton bones need unique names and earlier parents")
            }
            Self::Adapter(message) => write!(formatter, "animation adapter: {message}"),
        }
    }
}

impl std::error::Error for AnimationError {}

pub type AnimationResult<T> = Result<T, AnimationError>;

pub(crate) fn finite(value: f32) -> AnimationResult<f32> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(AnimationError::NonFinite)
    }
}

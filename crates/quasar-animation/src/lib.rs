//! Renderer-neutral animation timelines, blending and state machines.
//!
//! `quasar-animation` owns clips, tracks, events, playback, crossfades and
//! layered state machines, but not entities, scene graphs, GPU resources or a
//! global clock. Games advance players/state machines explicitly with their
//! own fixed timestep.
//!
//! ```
//! use std::sync::Arc;
//! use quasar_animation::{Clip, Interpolation, Keyframe, LoopMode, Marker, Player, Track, Value};
//!
//! let clip = Arc::new(Clip::new(
//!     "pulse",
//!     1.0,
//!     vec![Track::new(
//!         "scale",
//!         Interpolation::Linear,
//!         vec![
//!             Keyframe::new(0.0, Value::Float(1.0)),
//!             Keyframe::new(1.0, Value::Float(2.0)),
//!         ],
//!     )?],
//!     vec![Marker::new(0.5, "peak")],
//! )?);
//! let mut player = Player::new(clip, LoopMode::Loop);
//! player.advance(0.5)?;
//! assert_eq!(player.sample().get("scale"), Some(Value::Float(1.5)));
//! # Ok::<(), quasar_animation::AnimationError>(())
//! ```
//!
//! # Determinism
//!
//! Time is advanced by caller-provided `f32` deltas; there is no wall clock,
//! randomness or global state. Results are deterministic for the same delta
//! sequence and platform. Cross-platform floating-point bitwise equality is not
//! claimed, matching `quasar-sim`'s general simulation policy. Large looped
//! advances report skipped wrap counts instead of emitting an unbounded event
//! stream; `PingPong` playback caps segment expansion and sets `truncated`.
mod clip;
mod error;
mod pose;
mod skeleton;
#[cfg(feature = "sprite")]
mod sprite;
mod state;
mod value;

pub use clip::{
    Clip, Interpolation, Keyframe, LoopMode, Marker, Playback, PlaybackEvent, Player, Track,
};
pub use error::{AnimationError, AnimationResult};
pub use pose::Pose;
pub use skeleton::{
    BONE_ROTATION_SUFFIX, BONE_SCALE_SUFFIX, BONE_TRANSLATION_SUFFIX, Bone, Skeleton, SkeletonPose,
    Transform,
};
pub use state::{Compare, Condition, ParameterValue, StateMachine, Transition};
pub use value::Value;

/// Conventional property name used by the sprite adapter.
pub const SPRITE_FRAME_PROPERTY: &str = "sprite.frame";

/// Reads a sampled sprite frame from a pose. Floats are rounded and must be in
/// the `u32` range.
pub fn sprite_frame(pose: &Pose) -> Option<u32> {
    pose.get(SPRITE_FRAME_PROPERTY)?.as_u32()
}

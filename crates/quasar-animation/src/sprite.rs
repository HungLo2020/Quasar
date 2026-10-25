//! Optional bridge to `quasar-render-2d`'s sprite clip type.
//!
//! The animation crate itself only samples a `sprite.frame` property; games can
//! ignore the renderer entirely or convert a step track into the renderer's
//! frame list.
use crate::{AnimationError, AnimationResult, Clip, Interpolation, SPRITE_FRAME_PROPERTY, Value};

impl Clip {
    /// Converts the `sprite.frame` step track into `quasar-render-2d`'s
    /// caller-clocked `AnimationClip`.
    pub fn to_sprite_clip(&self) -> AnimationResult<quasar_render_2d::AnimationClip> {
        let track = self
            .tracks()
            .iter()
            .find(|track| track.property() == SPRITE_FRAME_PROPERTY)
            .ok_or_else(|| AnimationError::Adapter("missing sprite.frame track".into()))?;
        if track.interpolation() != Interpolation::Step {
            return Err(AnimationError::Adapter(
                "sprite.frame must use step interpolation".into(),
            ));
        }
        let mut frames = Vec::new();
        let keyframes = track.keyframes();
        let mut start = 0.0f32;
        for (index, keyframe) in keyframes.iter().enumerate() {
            if index == 0 && keyframe.time > 0.0 {
                push_frame(&mut frames, keyframe.value, keyframe.time)?;
                start = keyframe.time;
            }
            let end = keyframes
                .get(index + 1)
                .map_or(self.duration(), |next| next.time);
            if end > start {
                push_frame(&mut frames, keyframe.value, end - start)?;
            }
            start = end;
        }
        if frames.is_empty() {
            return Err(AnimationError::Adapter("sprite clip has no frames".into()));
        }
        quasar_render_2d::AnimationClip::new(frames)
            .map_err(|error| AnimationError::Adapter(error.to_string()))
    }
}

fn push_frame(
    frames: &mut Vec<quasar_render_2d::AnimationFrame>,
    value: Value,
    seconds: f32,
) -> AnimationResult<()> {
    let frame = value
        .as_u32()
        .ok_or_else(|| AnimationError::Adapter("sprite.frame must be a u32 or float".into()))?;
    let duration_ms = (seconds * 1000.0).round();
    if !duration_ms.is_finite() || duration_ms < 1.0 {
        return Err(AnimationError::Adapter(
            "sprite frame durations must be at least 1 ms".into(),
        ));
    }
    frames.push(quasar_render_2d::AnimationFrame {
        frame,
        duration_ms: duration_ms as u32,
    });
    Ok(())
}

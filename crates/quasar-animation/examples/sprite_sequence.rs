//! Headless sprite-sequence example shared with the renderer's animation model.
//!
//! ```sh
//! cargo run -p quasar-animation --example sprite_sequence
//! cargo run -p quasar-animation --features sprite --example sprite_sequence
//! ```
use quasar_animation::{Clip, Interpolation, Keyframe, LoopMode, Marker, Player, Track, Value};
use std::sync::Arc;

fn main() {
    let clip = Arc::new(
        Clip::new(
            "walk-sprite",
            0.48,
            vec![
                Track::new(
                    "sprite.frame",
                    Interpolation::Step,
                    vec![
                        Keyframe::new(0.00, Value::U32(0)),
                        Keyframe::new(0.12, Value::U32(1)),
                        Keyframe::new(0.24, Value::U32(2)),
                        Keyframe::new(0.36, Value::U32(3)),
                    ],
                )
                .unwrap(),
                Track::new(
                    "sprite.scale",
                    Interpolation::Linear,
                    vec![
                        Keyframe::new(0.0, Value::Float(1.0)),
                        Keyframe::new(0.24, Value::Float(1.08)),
                        Keyframe::new(0.48, Value::Float(1.0)),
                    ],
                )
                .unwrap(),
            ],
            vec![Marker::new(0.12, "footstep")],
        )
        .unwrap(),
    );
    let mut player = Player::new(clip.clone(), LoopMode::Loop);
    for step in 0..12 {
        let playback = player.advance(0.05).unwrap();
        let pose = player.sample();
        let frame = pose.get("sprite.frame").and_then(Value::as_u32);
        let scale = pose.f32("sprite.scale").unwrap_or(0.0);
        println!(
            "t={:.2} frame={frame:?} scale={scale:.3} events={:?}",
            (step + 1) as f32 * 0.05,
            playback
                .events
                .iter()
                .map(|event| event.marker.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[cfg(feature = "sprite")]
    {
        let converted = clip.to_sprite_clip().unwrap();
        println!("render-2d sprite clip: {} ms", converted.duration_ms());
    }
    #[cfg(not(feature = "sprite"))]
    {
        println!("enable the `sprite` feature to convert this clip for quasar-render-2d");
    }
}

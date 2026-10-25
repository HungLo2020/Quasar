//! Headless skeletal pose sampling and blending with the same core clips.
//!
//! ```sh
//! cargo run -p quasar-animation --example skeletal_blend
//! ```
use quasar_animation::{
    Bone, Clip, Interpolation, Keyframe, LoopMode, Player, Skeleton, Track, Transform, Value,
};
use std::sync::Arc;

fn rotation_z(angle: f32) -> [f32; 4] {
    let half = angle * 0.5;
    [0.0, 0.0, half.sin(), half.cos()]
}

fn sample_clip(name: &str, angle: f32) -> Arc<Clip> {
    Arc::new(
        Clip::new(
            name,
            1.0,
            vec![
                Track::new(
                    "bone.arm.rotation",
                    Interpolation::Linear,
                    vec![
                        Keyframe::new(0.0, Value::Quat(rotation_z(0.0))),
                        Keyframe::new(0.5, Value::Quat(rotation_z(angle))),
                        Keyframe::new(1.0, Value::Quat(rotation_z(0.0))),
                    ],
                )
                .unwrap(),
            ],
            vec![],
        )
        .unwrap(),
    )
}

fn main() {
    let skeleton = Skeleton::new(vec![
        Bone::new("root", None, Transform::default()).unwrap(),
        Bone::new("arm", Some(0), Transform::default()).unwrap(),
    ])
    .unwrap();
    let idle = Player::new(sample_clip("idle", 0.3), LoopMode::Loop);
    let wave = Player::new(sample_clip("wave", 1.2), LoopMode::Loop);

    for index in 0..=10 {
        let weight = index as f32 / 10.0;
        let idle_pose = idle.clip().sample(0.25);
        let wave_pose = wave.clip().sample(0.25);
        let blended = idle_pose.blend(&wave_pose, weight);
        let pose = skeleton.sample(&blended);
        println!(
            "blend={weight:.1} arm.rotation=[{:.3},{:.3},{:.3},{:.3}]",
            pose.bones()[1].rotation[0],
            pose.bones()[1].rotation[1],
            pose.bones()[1].rotation[2],
            pose.bones()[1].rotation[3]
        );
    }
}

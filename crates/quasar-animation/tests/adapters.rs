use quasar_animation::{Bone, Clip, Interpolation, Keyframe, Skeleton, Track, Transform, Value};
#[cfg(feature = "sprite")]
use std::sync::Arc;

fn rotation_z(angle: f32) -> [f32; 4] {
    let half = angle * 0.5;
    [0.0, 0.0, half.sin(), half.cos()]
}

#[test]
fn skeleton_rest_and_property_sampling_work() {
    let skeleton = Skeleton::new(vec![
        Bone::new("root", None, Transform::default()).unwrap(),
        Bone::new("arm", Some(0), Transform::default()).unwrap(),
    ])
    .unwrap();
    let clip = Clip::new(
        "arm",
        1.0,
        vec![
            Track::new(
                "bone.arm.rotation",
                Interpolation::Linear,
                vec![
                    Keyframe::new(0.0, Value::Quat(rotation_z(0.0))),
                    Keyframe::new(1.0, Value::Quat(rotation_z(1.0))),
                ],
            )
            .unwrap(),
        ],
        vec![],
    )
    .unwrap();
    let pose = skeleton.sample(&clip.sample(0.5));
    assert!((pose.bones()[1].rotation[2] - (0.5f32 * 0.5).sin()).abs() < 1e-4);
    let rest = skeleton.sample(&quasar_animation::Pose::new());
    assert_eq!(rest.bones()[0].rotation, [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn skeleton_rejects_duplicate_names_and_forward_parents() {
    assert!(
        Skeleton::new(vec![
            Bone::new("a", None, Transform::default()).unwrap(),
            Bone::new("a", Some(0), Transform::default()).unwrap(),
        ])
        .is_err()
    );
    assert!(Skeleton::new(vec![Bone::new("a", Some(0), Transform::default()).unwrap()]).is_err());
}

#[test]
fn skeleton_pose_blending_interpolates_transforms() {
    let skeleton =
        Skeleton::new(vec![Bone::new("root", None, Transform::default()).unwrap()]).unwrap();
    let left = quasar_animation::Pose::new();
    let mut right = quasar_animation::Pose::new();
    right.insert("bone.root.translation", Value::Vec3([10.0, 0.0, 0.0]));
    let a = skeleton.sample(&left);
    let b = skeleton.sample(&right);
    let blended = a.blend(&b, 0.25);
    assert!((blended.bones()[0].translation[0] - 2.5).abs() < 1e-5);
}

#[cfg(feature = "sprite")]
#[test]
fn sprite_track_converts_to_render_2d_frames() {
    let clip = Arc::new(
        Clip::new(
            "walk",
            0.4,
            vec![
                Track::new(
                    "sprite.frame",
                    Interpolation::Step,
                    vec![
                        Keyframe::new(0.0, Value::U32(0)),
                        Keyframe::new(0.1, Value::U32(1)),
                        Keyframe::new(0.2, Value::U32(2)),
                        Keyframe::new(0.3, Value::U32(3)),
                    ],
                )
                .unwrap(),
            ],
            vec![],
        )
        .unwrap(),
    );
    let converted = clip.to_sprite_clip().unwrap();
    assert_eq!(converted.duration_ms(), 400);
    assert_eq!(converted.frame_at(0), 0);
    assert_eq!(converted.frame_at(150), 1);
    assert_eq!(converted.frame_at(399), 3);
    assert_eq!(quasar_animation::sprite_frame(&clip.sample(0.25)), Some(2));
}

#[cfg(feature = "serde")]
#[test]
fn clips_and_transitions_round_trip_stably() {
    let clip = Clip::new(
        "serde",
        1.0,
        vec![
            Track::new(
                "x",
                Interpolation::Smooth,
                vec![
                    Keyframe::new(0.0, Value::Float(0.0)),
                    Keyframe::new(1.0, Value::Float(3.0)),
                ],
            )
            .unwrap(),
        ],
        vec![quasar_animation::Marker::new(0.5, "mid")],
    )
    .unwrap();
    let encoded = serde_json::to_string(&clip).unwrap();
    let decoded: Clip = serde_json::from_str(&encoded).unwrap();
    assert_eq!(clip, decoded);
    let transition = quasar_animation::Transition::new(
        Some("idle".into()),
        "run",
        0.2,
        quasar_animation::Condition::Float {
            parameter: "speed".into(),
            compare: quasar_animation::Compare::Greater,
            value: 1.0,
        },
    )
    .unwrap();
    let encoded = serde_json::to_string(&transition).unwrap();
    let decoded: quasar_animation::Transition = serde_json::from_str(&encoded).unwrap();
    assert_eq!(transition, decoded);

    let mut pose = quasar_animation::Pose::new();
    pose.insert("speed", Value::Float(2.0));
    let encoded = serde_json::to_string(&pose).unwrap();
    assert!(encoded.contains("\"speed\""));
    let decoded: quasar_animation::Pose = serde_json::from_str(&encoded).unwrap();
    assert_eq!(pose, decoded);
}

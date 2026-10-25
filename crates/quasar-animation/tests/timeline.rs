use quasar_animation::{
    AnimationError, Clip, Interpolation, Keyframe, LoopMode, Marker, Playback, Player, Pose, Track,
    Value,
};
use std::sync::Arc;

fn linear_clip() -> Arc<Clip> {
    Arc::new(
        Clip::new(
            "test",
            1.0,
            vec![
                Track::new(
                    "value",
                    Interpolation::Linear,
                    vec![
                        Keyframe::new(0.0, Value::Float(0.0)),
                        Keyframe::new(0.5, Value::Float(10.0)),
                        Keyframe::new(1.0, Value::Float(0.0)),
                    ],
                )
                .unwrap(),
            ],
            vec![
                Marker::new(0.0, "start"),
                Marker::new(0.5, "mid"),
                Marker::new(1.0, "end"),
            ],
        )
        .unwrap(),
    )
}

#[test]
fn track_sampling_covers_step_linear_smooth_and_boundaries() {
    let step = Track::new(
        "x",
        Interpolation::Step,
        vec![
            Keyframe::new(0.0, Value::U32(1)),
            Keyframe::new(0.5, Value::U32(2)),
            Keyframe::new(1.0, Value::U32(3)),
        ],
    )
    .unwrap();
    assert_eq!(step.sample(-1.0), Value::U32(1));
    assert_eq!(step.sample(0.0), Value::U32(1));
    assert_eq!(step.sample(0.499), Value::U32(1));
    assert_eq!(step.sample(0.5), Value::U32(2));
    assert_eq!(step.sample(2.0), Value::U32(3));

    let linear = Track::new(
        "x",
        Interpolation::Linear,
        vec![
            Keyframe::new(0.0, Value::Float(0.0)),
            Keyframe::new(1.0, Value::Float(10.0)),
        ],
    )
    .unwrap();
    assert_eq!(linear.sample(0.25), Value::Float(2.5));
    let smooth = Track::new(
        "x",
        Interpolation::Smooth,
        vec![
            Keyframe::new(0.0, Value::Float(0.0)),
            Keyframe::new(1.0, Value::Float(10.0)),
        ],
    )
    .unwrap();
    assert_eq!(smooth.sample(0.5), Value::Float(5.0));
    assert!((smooth.sample(0.25).as_f32().unwrap() - 1.5625).abs() < 1e-5);
}

#[test]
fn quaternion_interpolation_takes_the_shortest_path() {
    let a = [0.0, 0.0, 0.0, 1.0];
    let b = [0.0, 0.0, 0.0, -1.0];
    let blended = Value::Quat(a).lerp(Value::Quat(b), 0.5).unwrap();
    let quat = blended.as_quat().unwrap();
    assert!((0.0..=1.0).contains(&quat[3].abs()));
    let c = [0.0, 0.0, 1.0, 0.0];
    let blended = Value::Quat(a).lerp(Value::Quat(c), 0.5).unwrap();
    let quat = blended.as_quat().unwrap();
    let norm = quat.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5);
}

#[test]
fn clip_validation_rejects_invalid_inputs() {
    assert_eq!(
        Clip::new("empty", 1.0, vec![], vec![]).unwrap_err(),
        AnimationError::EmptyClip
    );
    assert_eq!(
        Track::new("x", Interpolation::Linear, vec![]).unwrap_err(),
        AnimationError::EmptyTrack
    );
    assert_eq!(
        Track::new(
            "x",
            Interpolation::Linear,
            vec![
                Keyframe::new(0.5, Value::Float(0.0)),
                Keyframe::new(0.5, Value::Float(1.0))
            ]
        )
        .unwrap_err(),
        AnimationError::InvalidKeyframeOrder
    );
    assert_eq!(
        Track::new(
            "x",
            Interpolation::Linear,
            vec![Keyframe::new(0.0, Value::Bool(true))]
        )
        .unwrap_err(),
        AnimationError::UnsupportedInterpolation
    );
    assert_eq!(
        Clip::new(
            "bad",
            1.0,
            vec![
                Track::new(
                    "x",
                    Interpolation::Linear,
                    vec![Keyframe::new(2.0, Value::Float(0.0))]
                )
                .unwrap()
            ],
            vec![]
        )
        .unwrap_err(),
        AnimationError::InvalidKeyframeOrder
    );
}

#[test]
fn once_playback_clamps_and_stops() {
    let mut player = Player::new(linear_clip(), LoopMode::Once);
    let first = player.advance(0.25).unwrap();
    assert!(first.events.is_empty());
    assert_eq!(player.sample().f32("value"), Some(5.0));
    let second = player.advance(2.0).unwrap();
    assert_eq!(player.time(), 1.0);
    assert!(!player.is_playing());
    assert!(second.events.iter().any(|event| event.marker == "mid"));
    assert!(second.events.iter().any(|event| event.marker == "end"));
    let third = player.advance(1.0).unwrap();
    assert_eq!(third, Playback::default());
}

#[test]
fn loop_playback_emits_boundaries_once_and_handles_large_steps() {
    let mut player = Player::new(linear_clip(), LoopMode::Loop);
    let first = player.advance(0.25).unwrap();
    assert!(first.events.is_empty());
    let second = player.advance(0.25).unwrap();
    assert_eq!(
        second
            .events
            .iter()
            .map(|event| event.marker.as_str())
            .collect::<Vec<_>>(),
        vec!["mid"]
    );
    let third = player.advance(0.25).unwrap();
    assert!(!third.events.iter().any(|event| event.marker == "end"));
    assert!((player.time() - 0.75).abs() < 1e-5);
    let fourth = player.advance(0.25).unwrap();
    assert!(fourth.events.iter().any(|event| event.marker == "end"));
    assert_eq!(player.time(), 0.0);

    let large = player.advance(1000.5).unwrap();
    assert!(large.wraps >= 1000);
    assert!(large.skipped_loops > 0);
    assert!((player.time() - 0.5).abs() < 0.01);
    assert!(large.events.len() <= 6);
}

#[test]
fn negative_and_pingpong_playback_are_stable() {
    let mut player = Player::new(linear_clip(), LoopMode::Loop);
    player.seek(0.75).unwrap();
    player.set_speed(-1.0).unwrap();
    let playback = player.advance(0.5).unwrap();
    assert!(playback.events.iter().any(|event| event.marker == "mid"));
    assert!((player.time() - 0.25).abs() < 1e-5);

    let mut ping = Player::new(linear_clip(), LoopMode::PingPong);
    let first = ping.advance(0.75).unwrap();
    assert!(first.events.iter().any(|event| event.marker == "mid"));
    let second = ping.advance(0.75).unwrap();
    assert!(second.events.iter().any(|event| event.marker == "end"));
    assert!(ping.time() >= 0.0 && ping.time() <= 1.0);
    let huge = ping.advance(100_000.0).unwrap();
    assert!(huge.truncated);
    assert!(ping.time().is_finite());

    let mut reverse = Player::new(linear_clip(), LoopMode::PingPong);
    reverse.seek(1.5).unwrap();
    assert!((reverse.time() - 0.5).abs() < 1e-5);
    reverse.seek(0.0).unwrap();
    reverse.set_speed(-1.0).unwrap();
    let playback = reverse.advance(0.25).unwrap();
    assert!(!playback.truncated);
    assert!((reverse.time() - 0.25).abs() < 1e-5);
}

#[test]
fn poses_blend_matching_and_disjoint_properties() {
    let mut left = Pose::new();
    left.insert("a", Value::Float(0.0));
    left.insert("only_left", Value::Float(1.0));
    let mut right = Pose::new();
    right.insert("a", Value::Float(10.0));
    right.insert("only_right", Value::Float(2.0));
    let blended = left.blend(&right, 0.25);
    assert_eq!(blended.f32("a"), Some(2.5));
    assert_eq!(blended.f32("only_left"), Some(1.0));
    assert_eq!(blended.f32("only_right"), Some(2.0));
    let zero = left.blend(&right, f32::NAN);
    assert_eq!(zero.f32("a"), Some(0.0));
    assert_eq!(zero.f32("only_right"), Some(2.0));
}

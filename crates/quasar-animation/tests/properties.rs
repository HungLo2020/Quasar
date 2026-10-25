use proptest::prelude::*;
use quasar_animation::{
    Clip, Interpolation, Keyframe, LoopMode, Marker, Player, Pose, Track, Value,
};
use std::sync::Arc;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn linear_scalar_sampling_matches_manual_interpolation(
        a in -100.0f32..100.0,
        b in -100.0f32..100.0,
        t in 0.0f32..1.0,
        sample in 0.0f32..1.0,
    ) {
        let track = Track::new(
            "x",
            Interpolation::Linear,
            vec![Keyframe::new(t, Value::Float(a)), Keyframe::new(1.0, Value::Float(b))],
        ).unwrap();
        let expected = if sample <= t { a } else { a + (b - a) * ((sample - t) / (1.0 - t)) };
        let actual = track.sample(sample).as_f32().unwrap();
        prop_assert!((actual - expected).abs() < 1e-4);
    }

    #[test]
    fn random_advances_stay_finite_and_bounded(
        deltas in proptest::collection::vec(0.0f32..5.0, 200),
        speed in -3.0f32..3.0,
    ) {
        let clip = Arc::new(Clip::new(
            "x",
            0.75,
            vec![Track::new(
                "v",
                Interpolation::Linear,
                vec![
                    Keyframe::new(0.0, Value::Float(0.0)),
                    Keyframe::new(0.75, Value::Float(1.0)),
                ],
            ).unwrap()],
            vec![Marker::new(0.25, "a"), Marker::new(0.5, "b")],
        ).unwrap());
        let mut player = Player::new(clip, LoopMode::PingPong);
        player.set_speed(speed).unwrap();
        for delta in deltas {
            let playback = player.advance(delta).unwrap();
            prop_assert!(player.time().is_finite());
            prop_assert!((0.0..=0.75).contains(&player.time()));
            prop_assert!(playback.events.len() < 10_000);
        }
    }

    #[test]
    fn pose_blend_is_clamped_and_type_safe(
        a in proptest::collection::vec(-10.0f32..10.0, 8),
        b in proptest::collection::vec(-10.0f32..10.0, 8),
        weight in -1.0f32..2.0,
    ) {
        let mut left = Pose::new();
        let mut right = Pose::new();
        for index in 0..8 {
            left.insert(format!("p{index}"), Value::Float(a[index]));
            right.insert(format!("p{index}"), Value::Float(b[index]));
        }
        let w = weight.clamp(0.0, 1.0);
        let blended = left.blend(&right, weight);
        for index in 0..8 {
            let expected = a[index] + (b[index] - a[index]) * w;
            let key = format!("p{index}");
            let actual = blended.f32(&key).unwrap();
            prop_assert!((actual - expected).abs() < 1e-4);
        }
    }
}

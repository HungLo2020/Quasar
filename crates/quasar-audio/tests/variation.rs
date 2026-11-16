//! Deterministic per-play pitch/gain variation for one-shot effects.
use quasar_audio::{PlaybackConfig, Variation};
use quasar_core::Rng;

#[test]
fn variation_spreads_pitch_and_gain_within_bounds() {
    let mut rng = Rng::seed_from_u64(11);
    let mut pitches = Vec::new();
    for _ in 0..1000 {
        let config = PlaybackConfig::default().with_variation(0.05, 0.1, &mut rng);
        assert!((0.95..1.05).contains(&config.pitch), "{}", config.pitch);
        assert!((0.9..1.1).contains(&config.volume), "{}", config.volume);
        pitches.push(config.pitch);
    }
    let min = pitches.iter().copied().fold(f32::INFINITY, f32::min);
    let max = pitches.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(min < 0.96 && max > 1.04, "spread is used: {min}..{max}");
}

#[test]
fn variation_is_deterministic_and_keeps_streams_in_lockstep() {
    let run = |spread: f32| {
        let mut rng = Rng::seed_from_u64(99);
        let config = PlaybackConfig::default().with_variation(spread, spread, &mut rng);
        (config.pitch, config.volume, rng.next_u64())
    };
    assert_eq!(run(0.2), run(0.2));
    // No spread leaves the config unchanged but still consumes the same draws.
    let (pitch, volume, next) = run(0.0);
    assert_eq!((pitch, volume), (1.0, 1.0));
    assert_eq!(next, run(0.2).2);
}

#[test]
fn variation_clamps_to_mixer_ranges() {
    let mut rng = Rng::seed_from_u64(1);
    let mut loud = PlaybackConfig {
        volume: 4.0,
        pitch: 16.0,
        ..Default::default()
    };
    for _ in 0..100 {
        Variation::new(0.5, 0.5).apply(&mut loud, &mut rng);
        assert!(loud.volume <= 4.0 && loud.pitch <= 16.0 && loud.pitch >= 0.01);
    }
    let odd = PlaybackConfig::default().with_variation(f32::NAN, -1.0, &mut rng);
    assert_eq!((odd.pitch, odd.volume), (1.0, 1.0));
    assert_eq!(Variation::NONE, Variation::new(0.0, 0.0));
}
